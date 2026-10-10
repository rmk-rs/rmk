use embassy_time::Duration;
use embedded_storage_async::nor_flash::NorFlash as AsyncNorFlash;
use rmk_types::constants::MACRO_CHUNK_SIZE;

use crate::keyboard::combo::Combo;
use crate::storage::{Storage, StorageKey, StorageValue, print_storage_error};

impl<F: AsyncNorFlash> Storage<F> {
    pub(crate) async fn read_layout<
        const ROW: usize,
        const COL: usize,
        const NUM_LAYER: usize,
        const NUM_ENCODER: usize,
        const NUM_TOUCHPAD: usize,
    >(
        &mut self,
        data: &mut crate::keymap::KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER, NUM_TOUCHPAD>,
        behavior: &mut crate::config::BehaviorConfig,
    ) -> Result<(), ()> {
        // Use fetch_all_items to speed up the keymap reading
        let mut key_iterator = self
            .flash
            .fetch_all_items(&mut self.buffer)
            .await
            .map_err(|e| print_storage_error::<F>(e))?;

        // Read all keymap keys and encoder configs
        while let Some((key, value)) = key_iterator
            .next::<StorageValue>(&mut self.buffer)
            .await
            .map_err(|e| print_storage_error::<F>(e))?
        {
            match (key, value) {
                (StorageKey::Keymap { layer, row, col }, StorageValue::KeyAction(action)) => {
                    let layer = layer as usize;
                    let row = row as usize;
                    let col = col as usize;
                    if layer < NUM_LAYER && row < ROW && col < COL {
                        data.keymap[layer][row][col] = action;
                    }
                }
                (StorageKey::Encoder { layer, idx }, StorageValue::EncoderAction(action)) => {
                    let idx = idx as usize;
                    let layer = layer as usize;
                    if layer < NUM_LAYER && idx < NUM_ENCODER {
                        data.encoder_map[layer][idx] = action;
                    }
                }
                // Restore the default (base) layer set via a `PDF` key
                (StorageKey::DefaultLayer, StorageValue::DefaultLayer(layer)) => behavior.default_layer = layer,
                // Restore the VIA/Vial layout options selection
                (StorageKey::LayoutOption, StorageValue::LayoutOption(option)) => data.layout_option = option,
                (StorageKey::MacroChunk(idx), StorageValue::MacroChunk(bytes)) => {
                    if let Some(chunk) = data.macros.as_chunks_mut::<MACRO_CHUNK_SIZE>().0.get_mut(idx as usize) {
                        *chunk = bytes;
                        data.macros_stored = true;
                    }
                }
                (StorageKey::BehaviorConfig, StorageValue::BehaviorConfig(c)) => {
                    behavior.morse.prior_idle_time = Duration::from_millis(c.prior_idle_time as u64);
                    behavior.morse.default_profile = c.morse_default_profile;
                    behavior.combo.timeout = Duration::from_millis(c.combo_timeout as u64);
                    behavior.one_shot.timeout = Duration::from_millis(c.one_shot_timeout as u64);
                    behavior.tap.tap_interval = c.tap_interval;
                    behavior.tap.tap_capslock_interval = c.tap_capslock_interval;
                }
                (StorageKey::Combo(idx), StorageValue::Combo(config)) => {
                    if let Some(slot) = behavior.combo.combos.get_mut(idx as usize) {
                        *slot = Some(Combo::new(config));
                    }
                }
                (StorageKey::Fork(idx), StorageValue::Fork(fork)) => {
                    if let Some(slot) = behavior.fork.forks.get_mut(idx as usize) {
                        *slot = fork;
                    }
                }
                (StorageKey::Morse(idx), StorageValue::Morse(morse)) => {
                    if let Some(slot) = behavior.morse.morses.get_mut(idx as usize) {
                        *slot = morse;
                    }
                }
                _ => continue,
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::action::Action;
    use rmk_types::keycode::{HidKeyCode, KeyCode};
    use rmk_types::morse::{HOLD, Morse, MorseMode, MorsePattern, MorseProfile, TAP};
    use sequential_storage::map::Value;

    use super::*;

    /// Every shape of `Morse` a host can write has to survive the round trip:
    /// Vial's four fixed actions, a partly filled table, and raw morse patterns.
    #[test]
    fn morse_round_trips_through_a_storage_value() {
        let key = |k| Action::Key(KeyCode::Hid(k));

        let vial = Morse::new_from_vial(
            key(HidKeyCode::A),
            key(HidKeyCode::B),
            key(HidKeyCode::C),
            key(HidKeyCode::D),
            MorseProfile::new(Some(true), Some(MorseMode::PermissiveHold), Some(190), Some(180)),
        );

        let mut partial = Morse::default();
        _ = partial.put(TAP, key(HidKeyCode::A));
        _ = partial.put(HOLD, key(HidKeyCode::B));

        let mut patterns = Morse {
            profile: MorseProfile::new(Some(false), Some(MorseMode::HoldOnOtherPress), Some(210), Some(220)),
            actions: heapless::LinearMap::default(),
        };
        for (pattern, k) in [
            (0b1_01, HidKeyCode::A),
            (0b1_1000, HidKeyCode::B),
            (0b1_1010, HidKeyCode::C),
        ] {
            patterns.actions.insert(MorsePattern::from_u16(pattern), key(k)).ok();
        }

        for morse in [vial, partial, patterns] {
            let mut buffer = [0u8; 64];
            let size = Value::serialize_into(&StorageValue::Morse(morse.clone()), &mut buffer).unwrap();
            let StorageValue::Morse(decoded) = StorageValue::deserialize_from(&buffer[..size]).unwrap().0 else {
                panic!("decoded as another variant");
            };
            assert_eq!(decoded.actions, morse.actions);
            assert_eq!(decoded.profile, morse.profile);
        }
    }
}
