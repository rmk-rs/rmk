#[cfg(feature = "vial")]
use rmk_types::action::{Action, KeyAction};
use rmk_types::keycode::{HidKeyCode, from_ascii, to_ascii};
#[cfg(feature = "vial")]
use rmk_types::protocol::vial_keycode::{from_via_keycode, to_via_keycode};

use crate::MACRO_SPACE_SIZE;
use crate::keymap::fill_vec;

/// MacroOperation: encoded with the two bytes, content at the third byte
///
/// TODO save space: refactor to use 1 byte for encoding and convert to/from vial 2 byte encoding
#[derive(Debug, Clone)]
pub enum MacroOperation {
    /// 0x00, 1 byte
    /// Marks the end of a macro sequence
    /// Don't use it on your own,
    /// will be automatically removed and added
    /// by MacroOperations::define_macro_sequences()
    End,
    /// 0x01 01 + 1 byte keycode
    Tap(HidKeyCode),
    /// 0x01 02 + 1 byte keycode
    Press(HidKeyCode),
    /// 0x01 03 + 1 byte keycode
    Release(HidKeyCode),
    /// 0x01 04 + 2 byte for the delay in ms, Vial-packed (so at most 65024 ms)
    Delay(u16),
    /// Anything not covered above (and starting at
    /// 0x30 (= b'0'), is the 1 byte ascii character.
    Text(HidKeyCode, bool), // bool = shifted
    /// 0x01 05 + 2 byte 16-bit keycode: tap an extended (>8-bit) keycode, e.g. a
    /// Bluetooth-profile or persistent-default-layer key, decoded via the Vial keycode table.
    #[cfg(feature = "vial")]
    TapAction(Action),
    /// 0x01 06 + 2 byte 16-bit keycode: press (hold down) an extended keycode.
    #[cfg(feature = "vial")]
    PressAction(Action),
    /// 0x01 07 + 2 byte 16-bit keycode: release an extended keycode.
    #[cfg(feature = "vial")]
    ReleaseAction(Action),
}

impl MacroOperation {
    /// Get the next macro operation starting from given index and offset (=position in the sequence)
    /// Return current macro operation and the next operations's offset
    pub(crate) fn get_next_macro_operation(
        macro_sequences: &[u8],
        macro_start_idx: usize,
        offset: usize,
    ) -> (MacroOperation, usize) {
        let idx = macro_start_idx + offset;
        if idx >= macro_sequences.len() - 1 {
            return (MacroOperation::End, offset);
        }
        match (macro_sequences[idx], macro_sequences[idx + 1]) {
            (0, _) => (MacroOperation::End, offset),
            (1, 1) => {
                if idx + 2 < macro_sequences.len() {
                    let keycode = macro_sequences[idx + 2].into();
                    (MacroOperation::Tap(keycode), offset + 3)
                } else {
                    (MacroOperation::End, offset + 3)
                }
            }
            (1, 2) => {
                if idx + 2 < macro_sequences.len() {
                    let keycode = macro_sequences[idx + 2].into();
                    (MacroOperation::Press(keycode), offset + 3)
                } else {
                    (MacroOperation::End, offset + 3)
                }
            }
            (1, 3) => {
                if idx + 2 < macro_sequences.len() {
                    let keycode = macro_sequences[idx + 2].into();
                    (MacroOperation::Release(keycode), offset + 3)
                } else {
                    (MacroOperation::End, offset + 3)
                }
            }
            (1, 4) => {
                if idx + 3 < macro_sequences.len() {
                    let delay_ms = (macro_sequences[idx + 2].max(1) as u16 - 1)
                        + (macro_sequences[idx + 3].max(1) as u16 - 1) * 255;
                    (MacroOperation::Delay(delay_ms), offset + 4)
                } else {
                    (MacroOperation::End, offset + 4)
                }
            }
            #[cfg(feature = "vial")]
            (1, kind @ 5..=7) => {
                if idx + 3 < macro_sequences.len() {
                    // Undo Vial's little-endian encoding and its zero-low-byte escape
                    // (`0xFF00 | kc>>8`, which keeps 0x00 out of the payload).
                    let raw = u16::from_le_bytes([macro_sequences[idx + 2], macro_sequences[idx + 3]]);
                    let keycode = if raw & 0xFF00 == 0xFF00 {
                        (raw & 0x00FF) << 8
                    } else {
                        raw
                    };
                    let action = from_via_keycode(keycode).to_action();
                    let operation = match kind {
                        5 => MacroOperation::TapAction(action),
                        6 => MacroOperation::PressAction(action),
                        _ => MacroOperation::ReleaseAction(action),
                    };
                    (operation, offset + 4)
                } else {
                    (MacroOperation::End, offset + 4)
                }
            }
            #[cfg(not(feature = "vial"))]
            (1, 5..=7) => {
                warn!("VIAL_MACRO_EXT is not supported");
                (MacroOperation::Delay(0), offset + 4)
            }
            _ => {
                // Current byte is the ascii code, convert it to keyboard keycode(with caps state)
                let (keycode, is_caps) = from_ascii(macro_sequences[idx]);
                (MacroOperation::Text(keycode, is_caps), offset + 1)
            }
        }
    }

    /// finds the start of a macro sequence by providing a guessed start index
    pub(crate) fn get_macro_sequence_start(macro_sequences: &[u8], guessed_macro_start_idx: u8) -> Option<usize> {
        let mut idx = 0;
        // Find idx until the macro start of given index
        let mut potential_start_idx = guessed_macro_start_idx;
        loop {
            if potential_start_idx == 0 || idx >= macro_sequences.len() {
                break;
            }
            if macro_sequences[idx] == 0 {
                potential_start_idx -= 1;
            }
            idx += 1;
        }

        if idx == macro_sequences.len() { None } else { Some(idx) }
    }
}

/// serializes macro sequences
/// macros are filled up with 0 if shorter than MACRO_SPACE_SIZE
/// so that it has enough space for macros defined my Vial
/// panics if the resulting binary macro sequence is longer than MACRO_SPACE_SIZE
pub fn define_macro_sequences(
    macro_sequences: &[heapless::Vec<MacroOperation, MACRO_SPACE_SIZE>],
) -> [u8; MACRO_SPACE_SIZE] {
    // TODO after binary format is understood and
    // TEXT is smaller than others,
    // refactor, exchanging tab for text (as this is shorter),
    // taking care of press/release LSHIFT and RSHIFT as well
    let mut macro_sequences_linear = fold_to_binary(macro_sequences);

    fill_vec(&mut macro_sequences_linear);
    macro_sequences_linear
        .into_array()
        .expect("as we resized the vector, this can't happen!")
}

impl IntoIterator for MacroOperation {
    type Item = MacroOperation;

    type IntoIter = <heapless::Vec<MacroOperation, MACRO_SPACE_SIZE> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        heapless::Vec::from_iter([self]).into_iter()
    }
}

/// Convinience function to convert a String into a sequence of MacroOptions::Text.
/// Currently only u8 ascii is supported.
pub fn to_macro_sequence(text: &str) -> heapless::Vec<MacroOperation, MACRO_SPACE_SIZE> {
    // if !text.is_ascii() {
    //     compile_error!("Only ascii text is supported!")
    // };
    text.as_bytes()
        .iter()
        .map(|character| {
            let (keycode, shifted) = from_ascii(*character);
            MacroOperation::Text(keycode, shifted)
        })
        .collect()
}

/// converts macro sequences [Vec<MacroOperation>] binary form and flattens to [Vec<u8, MACRO_SPACE_SIZE>]
/// Note that the Vec is still at it's minimal needed length and needs to be etended with zeros to the desired size
/// (with vec.resize())
fn fold_to_binary(
    macro_sequences: &[heapless::Vec<MacroOperation, MACRO_SPACE_SIZE>],
) -> heapless::Vec<u8, MACRO_SPACE_SIZE> {
    // TODO after binary format is understood and
    // TEXT is smaller than others,
    // refactor, exchanging tab for text (as this is shorter),
    // taking care of press/release LSHIFT and RSHIFT as well
    const TOO_MANY_ELEMENTS_ERROR_TEXT: &str = "Too many Macro Operations! The sum of all Macro Operations of all Macro Sequences cannot be more than MACRO_SPACE_SIZE";

    macro_sequences
        .iter()
        .map(|macro_sequence| {
            let mut vec_seq = macro_sequence
                .into_iter()
                .filter(|macro_operation| !matches!(macro_operation, MacroOperation::End))
                .map(serialize)
                .fold(heapless::Vec::<u8, MACRO_SPACE_SIZE>::new(), |mut acc, e| {
                    acc.extend_from_slice(&e).expect(TOO_MANY_ELEMENTS_ERROR_TEXT);
                    acc
                });
            vec_seq.push(0x00).expect(TOO_MANY_ELEMENTS_ERROR_TEXT); //= serialize(&MacroOperation::End));
            vec_seq
        })
        .fold(heapless::Vec::<u8, MACRO_SPACE_SIZE>::new(), |mut acc, e| {
            acc.extend_from_slice(&e).expect(TOO_MANY_ELEMENTS_ERROR_TEXT);
            acc
        })
}

fn serialize(macro_operation: &MacroOperation) -> heapless::Vec<u8, 4> {
    match macro_operation {
        MacroOperation::End => heapless::Vec::from_slice(&[0x00]).unwrap(),
        MacroOperation::Tap(key_code) => {
            let mut result = heapless::Vec::from_slice(&[0x01, 0x01]).unwrap();
            // TODO check is Keycode is correct
            result
                .extend_from_slice(&[(*key_code as u16).to_be_bytes()[1]])
                .expect("impossible error");
            result
        }
        MacroOperation::Press(key_code) => {
            let mut result = heapless::Vec::from_slice(&[0x01, 0x02]).unwrap();
            // TODO check is Keycode is correct
            result
                .extend_from_slice(&[(*key_code as u16).to_be_bytes()[1]])
                .expect("impossible error");
            result
        }
        MacroOperation::Release(key_code) => {
            let mut result = heapless::Vec::from_slice(&[0x01, 0x03]).unwrap();
            result
                .extend_from_slice(&[(*key_code as u16).to_be_bytes()[1]])
                .expect("impossible error");
            result
        }
        MacroOperation::Delay(duration) => {
            // Vial packs the delay into two 1-based bytes so the payload never contains
            // 0x00; this is the exact inverse of the decode in `get_next_macro_operation`.
            let mut result = heapless::Vec::from_slice(&[0x01, 0x04]).unwrap();
            result
                .extend_from_slice(&[(duration % 255) as u8 + 1, (duration / 255).min(254) as u8 + 1])
                .expect("impossible error");
            result
        }
        MacroOperation::Text(key_code, shifted) => heapless::Vec::from_slice(&[to_ascii(*key_code, *shifted)]).unwrap(),
        #[cfg(feature = "vial")]
        MacroOperation::TapAction(action) => serialize_extended_keycode(0x05, *action),
        #[cfg(feature = "vial")]
        MacroOperation::PressAction(action) => serialize_extended_keycode(0x06, *action),
        #[cfg(feature = "vial")]
        MacroOperation::ReleaseAction(action) => serialize_extended_keycode(0x07, *action),
    }
}

/// Inverse of the EXT decode in `get_next_macro_operation`: `0x01`, the kind
/// (`0x05`/`0x06`/`0x07`), then the keycode little-endian with the zero-low-byte escape.
#[cfg(feature = "vial")]
fn serialize_extended_keycode(kind: u8, action: Action) -> heapless::Vec<u8, 4> {
    let keycode = to_via_keycode(KeyAction::Single(action));
    // Mirror Vial's zero-low-byte escape so the payload never contains 0x00.
    let word = if keycode.is_multiple_of(256) {
        0xFF00 | (keycode >> 8)
    } else {
        keycode
    };
    let [lo, hi] = word.to_le_bytes();
    heapless::Vec::from_slice(&[0x01, kind, lo, hi]).unwrap()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_define_one_macro_sequence_manual() {
        let macro_sequences = &[heapless::Vec::from_slice(&[
            MacroOperation::Press(HidKeyCode::LShift),
            MacroOperation::Tap(HidKeyCode::P),
            MacroOperation::Release(HidKeyCode::LShift),
            MacroOperation::Tap(HidKeyCode::A),
            MacroOperation::Tap(HidKeyCode::T),
        ])
        .expect("too many elements")];
        let macro_sequences_binary = define_macro_sequences(macro_sequences);
        // let result = [0b 0000 0000 0100 1000]
        let result: [u8; 16] = [
            0x01, 0x02, 0xE1, 0x01, 0x01, 0x13, 0x01, 0x03, 0xE1, 0x01, 0x01, 0x4, 0x01, 0x01, 0x17, 0x00,
        ];
        let mut result_filled = [0; MACRO_SPACE_SIZE];
        for (i, element) in result.into_iter().enumerate() {
            result_filled[i] = element
        }
        assert_eq!(macro_sequences_binary, result_filled);
    }
    #[test]
    fn test_define_two_macro_sequence_manual() {
        let macro_sequences_terminated_uneccessarily = [
            heapless::Vec::from_slice(&[
                MacroOperation::Text(HidKeyCode::H, true),
                MacroOperation::Text(HidKeyCode::I, false),
            ])
            .expect("too many elements"),
            heapless::Vec::from_slice(&[
                MacroOperation::Press(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::P),
                MacroOperation::Release(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::A),
                MacroOperation::Tap(HidKeyCode::T),
            ])
            .expect("too many elements"),
        ];
        let macro_sequences_binary = define_macro_sequences(&macro_sequences_terminated_uneccessarily);
        let result: [u8; 19] = [
            0x48, 0x69, 0x00, 0x01, 0x02, 0xE1, 0x01, 0x01, 0x13, 0x01, 0x03, 0xE1, 0x01, 0x01, 0x4, 0x01, 0x01, 0x17,
            0x00,
        ];
        let mut result_filled = [0; MACRO_SPACE_SIZE];
        for (i, element) in result.into_iter().enumerate() {
            result_filled[i] = element
        }
        assert_eq!(macro_sequences_binary, result_filled);
    }

    #[test]
    fn test_define_macro_sequences_clean() {
        let macro_sequences_clean = [
            heapless::Vec::from_slice(&[
                MacroOperation::Press(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::H),
                MacroOperation::Release(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::E),
                MacroOperation::Tap(HidKeyCode::L),
                MacroOperation::Tap(HidKeyCode::L),
                MacroOperation::Tap(HidKeyCode::O),
            ])
            .expect("too many elements"),
            heapless::Vec::from_slice(&[
                MacroOperation::Tap(HidKeyCode::W),
                MacroOperation::Tap(HidKeyCode::O),
                MacroOperation::Tap(HidKeyCode::R),
                MacroOperation::Tap(HidKeyCode::L),
                MacroOperation::Tap(HidKeyCode::D),
            ])
            .expect("too many elements"),
            heapless::Vec::from_slice(&[
                MacroOperation::Press(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::Kc2),
                MacroOperation::Release(HidKeyCode::LShift),
            ])
            .expect("too many elements"),
        ];
        let macro_sequences_binary = define_macro_sequences(&macro_sequences_clean);
        let result: [u8; 48] = [
            1, 2, 225, 1, 1, 11, 1, 3, 225, 1, 1, 8, 1, 1, 15, 1, 1, 15, 1, 1, 18, 0, 1, 1, 26, 1, 1, 18, 1, 1, 21, 1,
            1, 15, 1, 1, 7, 0, 1, 2, 225, 1, 1, 31, 1, 3, 225, 0,
        ];
        let mut result_filled = [0; MACRO_SPACE_SIZE];
        for (i, element) in result.into_iter().enumerate() {
            result_filled[i] = element
        }
        assert_eq!(macro_sequences_binary, result_filled);
    }

    #[test]
    fn test_define_macro_sequences_uneccessarily_terminated() {
        let macro_sequences_terminated_uneccessarily = [
            heapless::Vec::from_slice(&[
                MacroOperation::Press(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::H),
                MacroOperation::Release(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::E),
                MacroOperation::Tap(HidKeyCode::L),
                MacroOperation::Tap(HidKeyCode::L),
                MacroOperation::Tap(HidKeyCode::O),
                MacroOperation::End,
            ])
            .expect("too many elements"),
            heapless::Vec::from_slice(&[
                MacroOperation::Tap(HidKeyCode::W),
                MacroOperation::Tap(HidKeyCode::O),
                MacroOperation::Tap(HidKeyCode::R),
                MacroOperation::Tap(HidKeyCode::L),
                MacroOperation::End,
            ])
            .expect("too many elements"),
            heapless::Vec::from_slice(&[
                MacroOperation::Press(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::Kc2),
                MacroOperation::Release(HidKeyCode::LShift),
                MacroOperation::End,
            ])
            .expect("too many elements"),
        ];
        let macro_sequences_binary = define_macro_sequences(&macro_sequences_terminated_uneccessarily);
        let result: [u8; 45] = [
            1, 2, 225, 1, 1, 11, 1, 3, 225, 1, 1, 8, 1, 1, 15, 1, 1, 15, 1, 1, 18, 0, 1, 1, 26, 1, 1, 18, 1, 1, 21, 1,
            1, 15, 0, 1, 2, 225, 1, 1, 31, 1, 3, 225, 0,
        ];
        let mut result_filled = [0; MACRO_SPACE_SIZE];
        for (i, element) in result.into_iter().enumerate() {
            result_filled[i] = element
        }
        assert_eq!(macro_sequences_binary, result_filled);
    }

    #[test]
    fn test_define_macro_sequences_random_end_markers() {
        let macro_sequences_random_end_markers = [
            heapless::Vec::from_slice(&[
                MacroOperation::Press(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::H),
                MacroOperation::End,
                MacroOperation::Release(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::E),
                MacroOperation::End,
                MacroOperation::End,
                MacroOperation::Tap(HidKeyCode::L),
                MacroOperation::End,
                MacroOperation::Tap(HidKeyCode::L),
                MacroOperation::Tap(HidKeyCode::O),
                MacroOperation::End,
            ])
            .expect("too many elements"),
            heapless::Vec::from_slice(&[
                MacroOperation::Tap(HidKeyCode::W),
                MacroOperation::Tap(HidKeyCode::O),
                MacroOperation::End,
                MacroOperation::End,
                MacroOperation::End,
                MacroOperation::End,
                MacroOperation::Tap(HidKeyCode::R),
                MacroOperation::Tap(HidKeyCode::L),
            ])
            .expect("too many elements"),
            heapless::Vec::from_slice(&[
                MacroOperation::Press(HidKeyCode::LShift),
                MacroOperation::Tap(HidKeyCode::Kc2),
                MacroOperation::Release(HidKeyCode::LShift),
                MacroOperation::End,
                MacroOperation::End,
                MacroOperation::End,
                MacroOperation::End,
                MacroOperation::End,
            ])
            .expect("too many elements"),
        ];
        let macro_sequences_binary = define_macro_sequences(&macro_sequences_random_end_markers);
        let result: [u8; 45] = [
            1, 2, 225, 1, 1, 11, 1, 3, 225, 1, 1, 8, 1, 1, 15, 1, 1, 15, 1, 1, 18, 0, 1, 1, 26, 1, 1, 18, 1, 1, 21, 1,
            1, 15, 0, 1, 2, 225, 1, 1, 31, 1, 3, 225, 0,
        ];
        let mut result_filled = [0; MACRO_SPACE_SIZE];
        for (i, element) in result.into_iter().enumerate() {
            result_filled[i] = element
        }
        assert_eq!(macro_sequences_binary, result_filled);
    }

    /// Build a macro buffer from raw Vial wire bytes.
    #[cfg(feature = "vial")]
    fn wire(bytes: &[u8]) -> [u8; MACRO_SPACE_SIZE] {
        let mut seq = [0u8; MACRO_SPACE_SIZE];
        seq[..bytes.len()].copy_from_slice(bytes);
        seq
    }

    // M0: TAP(BT0) -> Delay(100ms) -> TAP(PDF0), using the reconstructed wire vectors.
    #[cfg(feature = "vial")]
    #[test]
    fn test_parse_extended_macro_m0() {
        use rmk_types::action::Action;

        let seq = wire(&[
            0x01, 0x05, 0x7E, 0xFF, // TAP(BT0): USER00 = 0x7E00, zero-byte escaped
            0x01, 0x04, 0x65, 0x01, // Delay 100ms
            0x01, 0x05, 0xE0, 0x52, // TAP(PDF0): 0x52E0
            0x00, // End
        ]);
        let start = MacroOperation::get_macro_sequence_start(&seq, 0).unwrap();

        let (op, off) = MacroOperation::get_next_macro_operation(&seq, start, 0);
        assert!(matches!(op, MacroOperation::TapAction(Action::User(0))));
        let (op, off) = MacroOperation::get_next_macro_operation(&seq, start, off);
        assert!(matches!(op, MacroOperation::Delay(100)));
        let (op, off) = MacroOperation::get_next_macro_operation(&seq, start, off);
        assert!(matches!(
            op,
            MacroOperation::TapAction(Action::PersistentDefaultLayer(0))
        ));
        let (op, _) = MacroOperation::get_next_macro_operation(&seq, start, off);
        assert!(matches!(op, MacroOperation::End));
    }

    // M1: TAP(BT1) -> Delay(100ms) -> TAP(PDF1); BT1 has a non-zero low byte (no escape).
    #[cfg(feature = "vial")]
    #[test]
    fn test_parse_extended_macro_m1() {
        use rmk_types::action::Action;

        let seq = wire(&[
            0x01, 0x05, 0x01, 0x7E, // TAP(BT1): USER01 = 0x7E01, little-endian
            0x01, 0x04, 0x65, 0x01, // Delay 100ms
            0x01, 0x05, 0xE1, 0x52, // TAP(PDF1): 0x52E1
            0x00,
        ]);
        let start = MacroOperation::get_macro_sequence_start(&seq, 0).unwrap();

        let (op, off) = MacroOperation::get_next_macro_operation(&seq, start, 0);
        assert!(matches!(op, MacroOperation::TapAction(Action::User(1))));
        let (op, off) = MacroOperation::get_next_macro_operation(&seq, start, off);
        assert!(matches!(op, MacroOperation::Delay(100)));
        let (op, _) = MacroOperation::get_next_macro_operation(&seq, start, off);
        assert!(matches!(
            op,
            MacroOperation::TapAction(Action::PersistentDefaultLayer(1))
        ));
    }

    // EXT KEY DOWN / UP (0x01 06 / 0x01 07) decode into Press/ReleaseAction.
    #[cfg(feature = "vial")]
    #[test]
    fn test_parse_extended_macro_down_up() {
        use rmk_types::action::Action;

        let seq = wire(&[0x01, 0x06, 0x7E, 0xFF, 0x01, 0x07, 0x7E, 0xFF, 0x00]);
        let start = MacroOperation::get_macro_sequence_start(&seq, 0).unwrap();

        let (op, off) = MacroOperation::get_next_macro_operation(&seq, start, 0);
        assert!(matches!(op, MacroOperation::PressAction(Action::User(0))));
        let (op, _) = MacroOperation::get_next_macro_operation(&seq, start, off);
        assert!(matches!(op, MacroOperation::ReleaseAction(Action::User(0))));
    }

    // Serializing the decoded operations reproduces M0's exact wire bytes, including the
    // 0x7E00 -> `7E FF` zero-byte escape and the Vial-packed delay.
    #[cfg(feature = "vial")]
    #[test]
    fn test_serialize_extended_macro_keycodes() {
        use rmk_types::action::Action;

        let sequences = [heapless::Vec::from_slice(&[
            MacroOperation::TapAction(Action::User(0)),
            MacroOperation::Delay(100),
            MacroOperation::TapAction(Action::PersistentDefaultLayer(0)),
        ])
        .expect("too many elements")];
        let binary = define_macro_sequences(&sequences);
        let expected = [
            0x01, 0x05, 0x7E, 0xFF, 0x01, 0x04, 0x65, 0x01, 0x01, 0x05, 0xE0, 0x52, 0x00,
        ];
        assert_eq!(&binary[..expected.len()], &expected);
    }

    // A delay round-trips through the packed encoding at both ends of its range.
    #[test]
    fn test_delay_round_trips_through_the_packed_encoding() {
        for ms in [0, 1, 50, 254, 255, 1000, 65024] {
            let sequences = [heapless::Vec::from_slice(&[MacroOperation::Delay(ms)]).expect("too many elements")];
            let binary = define_macro_sequences(&sequences);
            let (op, _) = MacroOperation::get_next_macro_operation(&binary, 0, 0);
            assert!(
                matches!(op, MacroOperation::Delay(d) if d == ms),
                "{ms}ms did not round-trip"
            );
        }
    }

    // A buffer that ends in the middle of an EXT command must terminate safely, without
    // panicking or reading out of bounds.
    #[cfg(feature = "vial")]
    #[test]
    fn test_parse_extended_macro_truncated_is_safe() {
        let mut seq = [0u8; MACRO_SPACE_SIZE];
        // "01 05" right at the end: the two payload bytes are out of range.
        seq[MACRO_SPACE_SIZE - 2] = 0x01;
        seq[MACRO_SPACE_SIZE - 1] = 0x05;
        let (op, _) = MacroOperation::get_next_macro_operation(&seq, MACRO_SPACE_SIZE - 2, 0);
        assert!(matches!(op, MacroOperation::End));
    }
}
