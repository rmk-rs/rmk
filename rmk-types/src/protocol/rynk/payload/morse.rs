//! Morse endpoint types.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::morse::Morse;
#[cfg(not(feature = "host"))]
use crate::protocol::rynk::payload::bulk_capacity::MAX_BULK_ITEMS;

// Firmware uses a bounded Vec; host bounds transfers from capabilities.
#[cfg(not(feature = "host"))]
type BulkMorses = heapless::Vec<Morse, MAX_BULK_ITEMS>;
#[cfg(feature = "host")]
type BulkMorses = alloc::vec::Vec<Morse>;

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
    use crate::action::{Action, KeyAction};
    use crate::constants::MORSE_SIZE;
    use crate::keycode::HidKeyCode;
    use crate::modifier::ModifierCombination;
    use crate::morse::{MorsePattern, MorseProfile};
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    /// Build a `Morse` whose `actions` `LinearMap` is filled to `MORSE_SIZE`
    /// distinct entries, each using a multi-field `KeyAction` variant so both the
    /// entry count *and* the per-entry encoded size meaningfully exercise the
    /// manual `MaxSize` impl. `MorsePattern::from_u16(0)` panics (the empty
    /// pattern is `0b1`), so patterns start at 1.
    fn full_morse() -> Morse {
        // Use a multi-byte action so MaxSize catches per-entry under-counts.
        let action = KeyAction::Sticky(
            Action::KeyWithModifier(HidKeyCode::A, ModifierCombination::new()),
            u8::MAX,
        );
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
        morse
            .actions
            .insert(MorsePattern::from_u16(0b101), KeyAction::No)
            .unwrap();
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
}
