use embedded_storage_async::nor_flash::NorFlash as AsyncNorFlash;
use rmk_types::fork::Fork;
use rmk_types::morse::Morse;
use serde::de::{Error as DeError, SeqAccess, Visitor};
use serde::{Deserializer, Serializer};

use crate::keyboard::combo::Combo;
use crate::storage::{Storage, StorageData, StorageKey, print_storage_error};
use crate::{COMBO_MAX_NUM, FORK_MAX_NUM, MACRO_SPACE_SIZE, MORSE_MAX_NUM};

pub(crate) mod macro_bytes_serde {
    use super::*;

    pub(crate) fn serialize<S>(value: &[u8; MACRO_SPACE_SIZE], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(value)
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<[u8; MACRO_SPACE_SIZE], D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MacroBytesVisitor;

        impl<'de> Visitor<'de> for MacroBytesVisitor {
            type Value = [u8; MACRO_SPACE_SIZE];

            fn expecting(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                write!(formatter, "exactly {MACRO_SPACE_SIZE} bytes")
            }

            fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
            where
                E: DeError,
            {
                if value.len() != MACRO_SPACE_SIZE {
                    return Err(E::invalid_length(value.len(), &self));
                }

                let mut bytes = [0u8; MACRO_SPACE_SIZE];
                bytes.copy_from_slice(value);
                Ok(bytes)
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut bytes = [0u8; MACRO_SPACE_SIZE];
                for (idx, slot) in bytes.iter_mut().enumerate() {
                    *slot = seq
                        .next_element()?
                        .ok_or_else(|| A::Error::invalid_length(idx, &self))?;
                }

                if (seq.next_element::<u8>()?).is_some() {
                    return Err(A::Error::invalid_length(MACRO_SPACE_SIZE + 1, &self));
                }

                Ok(bytes)
            }
        }

        deserializer.deserialize_bytes(MacroBytesVisitor)
    }
}

impl<F: AsyncNorFlash, const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    pub(crate) async fn read_keymap(
        &mut self,
        data: &mut crate::keymap::KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>,
        behavior: &mut crate::config::BehaviorConfig,
    ) -> Result<(), ()> {
        // Use fetch_all_items to speed up the keymap reading
        let mut key_iterator = self
            .flash
            .fetch_all_items(&mut self.buffer)
            .await
            .map_err(|e| print_storage_error::<F>(e))?;

        // Read all keymap keys and encoder configs
        while let Some((key, item)) = key_iterator
            .next::<StorageData>(&mut self.buffer)
            .await
            .map_err(|e| print_storage_error::<F>(e))?
        {
            match (key, item) {
                (StorageKey::Keymap { layer, row, col }, StorageData::KeyAction(action)) => {
                    let layer = layer as usize;
                    let row = row as usize;
                    let col = col as usize;
                    if layer < NUM_LAYER && row < ROW && col < COL {
                        data.keymap[layer][row][col] = action;
                    }
                }
                (StorageKey::Encoder { layer, idx }, StorageData::EncoderAction(action)) => {
                    let idx = idx as usize;
                    let layer = layer as usize;
                    if layer < NUM_LAYER && idx < NUM_ENCODER {
                        data.encoder_map[layer][idx] = action;
                    }
                }
                (StorageKey::LayoutConfig, StorageData::LayoutConfig(config)) => {
                    // Restore the default (base) layer set via a `PDF` key
                    behavior.default_layer = config.default_layer;
                    // Restore the VIA/Vial layout options selection
                    data.layout_option = config.layout_option;
                }
                _ => continue,
            }
        }

        Ok(())
    }

    pub(crate) async fn read_macro_cache(&mut self, macro_cache: &mut [u8]) -> Result<(), ()> {
        let read_data = self
            .flash
            .fetch_item(&mut self.buffer, &StorageKey::MacroData)
            .await
            .map_err(|e| print_storage_error::<F>(e))?;

        if let Some(StorageData::MacroData(data)) = read_data {
            macro_cache.copy_from_slice(&data);
        }

        Ok(())
    }

    pub(crate) async fn read_combos(&mut self, combos: &mut [Option<Combo>; COMBO_MAX_NUM]) -> Result<(), ()> {
        use crate::keyboard::combo::Combo;

        for (i, item) in combos.iter_mut().enumerate() {
            let key = StorageKey::combo(i as u8);
            let read_data = self
                .flash
                .fetch_item(&mut self.buffer, &key)
                .await
                .map_err(|e| print_storage_error::<F>(e))?;

            if let Some(StorageData::Combo(config)) = read_data {
                debug!("Read combo config: {:?}", config);
                *item = Some(Combo::new(config));
            }
        }

        Ok(())
    }

    pub(crate) async fn read_forks(&mut self, forks: &mut heapless::Vec<Fork, FORK_MAX_NUM>) -> Result<(), ()> {
        for (i, item) in forks.iter_mut().enumerate() {
            let key = StorageKey::fork(i as u8);
            let read_data = self
                .flash
                .fetch_item(&mut self.buffer, &key)
                .await
                .map_err(|e| print_storage_error::<F>(e))?;

            if let Some(StorageData::Fork(fork)) = read_data {
                *item = fork;
            }
        }

        Ok(())
    }

    pub(crate) async fn read_morses(&mut self, morses: &mut heapless::Vec<Morse, MORSE_MAX_NUM>) -> Result<(), ()> {
        for (i, item) in morses.iter_mut().enumerate() {
            let key = StorageKey::morse(i as u8);
            let read_data = self
                .flash
                .fetch_item(&mut self.buffer, &key)
                .await
                .map_err(|e| print_storage_error::<F>(e))?;

            if let Some(StorageData::Morse(morse)) = read_data {
                *item = morse;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::action::Action;
    use rmk_types::keycode::{HidKeyCode, KeyCode};
    use rmk_types::morse::{HOLD, MorseMode, MorsePattern, MorseProfile, TAP};
    use sequential_storage::map::Value;

    use super::*;

    #[test]
    fn test_morse_serialization_deserialization() {
        let morse = Morse::new_from_vial(
            Action::Key(KeyCode::Hid(HidKeyCode::A)),
            Action::Key(KeyCode::Hid(HidKeyCode::B)),
            Action::Key(KeyCode::Hid(HidKeyCode::C)),
            Action::Key(KeyCode::Hid(HidKeyCode::D)),
            MorseProfile::new(Some(true), Some(MorseMode::PermissiveHold), Some(190u16), Some(180u16)),
        );

        // Serialization
        let mut buffer = [0u8; 64];
        let storage_data = StorageData::Morse(morse.clone());
        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();

        // Deserialization
        let deserialized_data = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();

        // Validation
        match deserialized_data {
            (StorageData::Morse(deserialized_morse), _) => {
                // actions
                assert_eq!(deserialized_morse.actions.len(), morse.actions.len());
                for (original, deserialized) in morse.actions.iter().zip(deserialized_morse.actions.iter()) {
                    assert_eq!(original, deserialized);
                }
                // profile
                assert_eq!(deserialized_morse.profile, morse.profile);
            }
            _ => panic!("Expected MorseData"),
        }
    }

    #[test]
    fn test_morse_with_partial_actions() {
        // Create a Morse with partial actions
        let mut morse = Morse::default();
        _ = morse.put(TAP, Action::Key(KeyCode::Hid(HidKeyCode::A)));
        _ = morse.put(HOLD, Action::Key(KeyCode::Hid(HidKeyCode::B)));

        // Serialization
        let mut buffer = [0u8; 64];
        let storage_data = StorageData::Morse(morse.clone());
        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();

        // Deserialization
        let deserialized_data = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();

        // Validation
        match deserialized_data {
            (StorageData::Morse(deserialized_morse), _) => {
                // actions
                assert_eq!(deserialized_morse.actions.len(), morse.actions.len());
                for (original, deserialized) in morse.actions.iter().zip(deserialized_morse.actions.iter()) {
                    assert_eq!(original, deserialized);
                }
                // profile
                assert_eq!(deserialized_morse.profile, morse.profile);
            }
            _ => panic!("Expected MorseData"),
        }
    }

    #[test]
    fn test_morse_with_morse_serialization_deserialization() {
        let mut morse = Morse {
            profile: MorseProfile::new(
                Some(false),
                Some(MorseMode::HoldOnOtherPress),
                Some(210u16),
                Some(220u16),
            ),
            actions: heapless::LinearMap::default(),
        };
        morse
            .actions
            .insert(MorsePattern::from_u16(0b1_01), Action::Key(KeyCode::Hid(HidKeyCode::A)))
            .ok();
        morse
            .actions
            .insert(
                MorsePattern::from_u16(0b1_1000),
                Action::Key(KeyCode::Hid(HidKeyCode::B)),
            )
            .ok();
        morse
            .actions
            .insert(
                MorsePattern::from_u16(0b1_1010),
                Action::Key(KeyCode::Hid(HidKeyCode::C)),
            )
            .ok();

        // Serialization
        let mut buffer = [0u8; 64];
        let storage_data = StorageData::Morse(morse.clone());
        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();

        // Deserialization
        let deserialized_data = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();

        // Validation
        match deserialized_data {
            (StorageData::Morse(deserialized_morse), _) => {
                // actions
                assert_eq!(deserialized_morse.actions.len(), morse.actions.len());
                for (original, deserialized) in morse.actions.iter().zip(deserialized_morse.actions.iter()) {
                    assert_eq!(original, deserialized);
                }
                // profile
                assert_eq!(deserialized_morse.profile, morse.profile);
            }
            _ => panic!("Expected MorseData"),
        }
    }
}
