//! Morse endpoint types.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "host"))]
use crate::constants::HOLD_TRIGGER_KEY_POSITION_MAX_NUM;
use crate::morse::Morse;
#[cfg(not(feature = "host"))]
use crate::protocol::rynk::payload::bulk_capacity::MAX_BULK_ITEMS;

// Firmware uses a bounded Vec; host bounds transfers from capabilities.
#[cfg(not(feature = "host"))]
type BulkMorses = heapless::Vec<Morse, MAX_BULK_ITEMS>;
#[cfg(feature = "host")]
type BulkMorses = alloc::vec::Vec<Morse>;

/// One key position allowed to trigger a tap-hold profile's hold action.
///
/// `profile == u8::MAX` addresses the keyboard-wide default profile. Every
/// other value is a runtime morse-profile slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct MorseHoldTriggerPosition {
    pub profile: u8,
    pub row: u8,
    pub col: u8,
}

// Firmware uses the keyboard's compiled capacity; host clients accept the
// capacity advertised by whichever keyboard they connected to.
#[cfg(not(feature = "host"))]
pub type MorseHoldTriggerPositions = heapless::Vec<MorseHoldTriggerPosition, HOLD_TRIGGER_KEY_POSITION_MAX_NUM>;
#[cfg(feature = "host")]
pub type MorseHoldTriggerPositions = alloc::vec::Vec<MorseHoldTriggerPosition>;

/// Current runtime hold-trigger table and the firmware's compiled capacity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct MorseHoldTriggerPositionState {
    pub capacity: u8,
    #[cfg_attr(feature = "wasm", tsify(type = "MorseHoldTriggerPosition[]"))]
    pub positions: MorseHoldTriggerPositions,
}

#[cfg(not(feature = "host"))]
impl MaxSize for MorseHoldTriggerPositionState {
    const POSTCARD_MAX_SIZE: usize = u8::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<MorseHoldTriggerPosition, HOLD_TRIGGER_KEY_POSITION_MAX_NUM>();
}

/// Atomic replacement payload for the runtime hold-trigger table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct SetMorseHoldTriggerPositionsRequest {
    #[cfg_attr(feature = "wasm", tsify(type = "MorseHoldTriggerPosition[]"))]
    pub positions: MorseHoldTriggerPositions,
}

#[cfg(not(feature = "host"))]
impl MaxSize for SetMorseHoldTriggerPositionsRequest {
    const POSTCARD_MAX_SIZE: usize =
        crate::heapless_vec_max_size::<MorseHoldTriggerPosition, HOLD_TRIGGER_KEY_POSITION_MAX_NUM>();
}

/// Request payload for `SetMorse`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct SetMorseRequest {
    pub index: u8,
    pub config: Morse,
}

/// Request payload for `GetMorseBulk`: read a page of morses starting at slot
/// `start_index`. The firmware returns as many as fit, or an empty page once
/// `start_index` reaches the slot count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct GetMorseBulkRequest {
    pub start_index: u8,
}

/// Bulk request payload for `SetMorseBulk`: write `configs` starting at slot
/// `start_index`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct SetMorseBulkRequest {
    pub start_index: u8,
    #[cfg_attr(feature = "wasm", tsify(type = "Morse[]"))]
    pub configs: BulkMorses,
}

// Set pages pack by real encoded size, so the wire bound is the whole payload budget.
#[cfg(not(feature = "host"))]
impl MaxSize for SetMorseBulkRequest {
    const POSTCARD_MAX_SIZE: usize = crate::protocol::rynk::RYNK_MAX_PAYLOAD_SIZE;
}

/// Bulk response for getting multiple morse configs at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct GetMorseBulkResponse {
    #[cfg_attr(feature = "wasm", tsify(type = "Morse[]"))]
    pub configs: BulkMorses,
}

#[cfg(not(feature = "host"))]
impl MaxSize for GetMorseBulkResponse {
    const POSTCARD_MAX_SIZE: usize = crate::heapless_vec_max_size::<Morse, MAX_BULK_ITEMS>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::constants::MORSE_SIZE;
    use crate::keycode::HidKeyCode;
    use crate::modifier::ModifierCombination;
    use crate::morse::{MorsePattern, MorseProfile};
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    /// Build a `Morse` whose `actions` `LinearMap` is filled to `MORSE_SIZE`
    /// distinct entries, each using a multi-field `Action` variant so both the
    /// entry count *and* the per-entry encoded size meaningfully exercise the
    /// manual `MaxSize` impl. `MorsePattern::from_u16(0)` panics (the empty
    /// pattern is `0b1`), so patterns start at 1.
    fn full_morse() -> Morse {
        // Use a multi-byte action so MaxSize catches per-entry under-counts.
        let action = Action::KeyWithModifier(HidKeyCode::A, ModifierCombination::new());
        let mut m = Morse {
            profile: MorseProfile::const_default(),
            actions: heapless::LinearMap::new(),
        };
        for i in 0..MORSE_SIZE {
            m.actions
                .insert(MorsePattern::from_u16((i + 1) as u16), action)
                .unwrap();
        }
        m
    }

    #[test]
    fn round_trip_morse() {
        round_trip(&Morse {
            profile: MorseProfile::const_default(),
            actions: heapless::LinearMap::new(),
        });
    }

    #[test]
    fn round_trip_set_morse_request() {
        let mut morse = Morse {
            profile: MorseProfile::const_default(),
            actions: heapless::LinearMap::new(),
        };
        morse.actions.insert(MorsePattern::from_u16(0b101), Action::No).unwrap();
        round_trip(&SetMorseRequest {
            index: 0,
            config: morse,
        });
    }

    #[test]
    fn round_trip_morse_max_capacity() {
        let m = full_morse();
        assert_eq!(m.actions.len(), MORSE_SIZE);
        round_trip(&m);
        assert_max_size_bound(&m);
    }

    // Firmware-only: exercises heapless bulk capacity.
    #[cfg(not(feature = "host"))]
    mod bulk {
        use heapless::Vec;

        use super::super::*;
        use super::full_morse;
        use crate::morse::Morse;
        use crate::protocol::rynk::payload::bulk_capacity::MAX_BULK_ITEMS;
        use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

        #[test]
        fn round_trip_set_morse_bulk_request_max_capacity() {
            let mut configs: Vec<Morse, MAX_BULK_ITEMS> = Vec::new();
            for _ in 0..MAX_BULK_ITEMS {
                configs.push(full_morse()).unwrap();
            }
            let req = SetMorseBulkRequest {
                start_index: u8::MAX,
                configs,
            };
            round_trip(&req);
            assert_max_size_bound(&req);
        }

        #[test]
        fn round_trip_get_morse_bulk_response_max_capacity() {
            let mut configs: Vec<Morse, MAX_BULK_ITEMS> = Vec::new();
            for _ in 0..MAX_BULK_ITEMS {
                configs.push(full_morse()).unwrap();
            }
            let resp = GetMorseBulkResponse { configs };
            round_trip(&resp);
            assert_max_size_bound(&resp);
        }
    }

    // Firmware-only: the host side intentionally uses an unbounded Vec and
    // therefore has no meaningful compile-time MaxSize implementation.
    #[cfg(not(feature = "host"))]
    mod hold_trigger_positions {
        use heapless::Vec;

        use super::super::*;
        use crate::constants::HOLD_TRIGGER_KEY_POSITION_MAX_NUM;
        use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

        fn full_positions() -> MorseHoldTriggerPositions {
            let mut positions: MorseHoldTriggerPositions = Vec::new();
            for index in 0..HOLD_TRIGGER_KEY_POSITION_MAX_NUM {
                positions
                    .push(MorseHoldTriggerPosition {
                        profile: if index == 0 { u8::MAX } else { (index - 1) as u8 },
                        row: (index % 6) as u8,
                        col: (index % 14) as u8,
                    })
                    .unwrap();
            }
            positions
        }

        #[test]
        fn round_trip_hold_trigger_position_state_max_capacity() {
            let state = MorseHoldTriggerPositionState {
                capacity: HOLD_TRIGGER_KEY_POSITION_MAX_NUM as u8,
                positions: full_positions(),
            };
            round_trip(&state);
            assert_max_size_bound(&state);
        }

        #[test]
        fn round_trip_set_hold_trigger_positions_max_capacity() {
            let request = SetMorseHoldTriggerPositionsRequest {
                positions: full_positions(),
            };
            round_trip(&request);
            assert_max_size_bound(&request);
        }
    }
}
