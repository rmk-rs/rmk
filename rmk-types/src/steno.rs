//! Stenography (Plover HID) key identifiers.
//!
//! Each [`StenoKey`] is an index 0..=63 into the 64-bit chord bitmap that
//! Plover HID delivers to the host. The order of the chart matches the
//! canonical Plover HID specification at
//! <https://github.com/dnaq/plover-machine-hid>: index 0 is `S1-` and is
//! the most significant bit of byte 1 of the wire report; index 63 (`X26`)
//! is the least significant bit of byte 8.
//!
//! The standard Ward-Stone-Ireland keys come first (`S1-` through `#1`,
//! indices 0-22), followed by the extended steno keys (`S2-`, `*2`-`*4`,
//! `#2`-`#C`, indices 23-37), followed by 26 vendor-defined "extra" keys
//! `X1`..`X26` (indices 38-63).

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Report ID byte that prefixes every Plover HID input report on the wire.
/// Must match the `report_id` literal in the `gen_hid_descriptor!` macro
/// invocation for `StenoReport`.
pub const PLOVER_HID_REPORT_ID: u8 = 0x50;

/// A single steno key, identified by its position (0..=63) in the canonical
/// Plover HID key chart. See module docs for the full list.
///
/// Variants are declared in chart order with matching discriminants, keeping
/// serde's ordinal encoding byte-identical to a plain `u8` chart index, one
/// postcard varint byte per key.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "_codegen", derive(strum::VariantNames))]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
#[repr(u8)]
pub enum StenoKey {
    // Standard Ward-Stone-Ireland keys (chart indices 0-22)
    S1 = 0,
    T = 1,
    K = 2,
    P = 3,
    W = 4,
    H = 5,

    // Left-hand R; right-hand keys are prefixed `R` (e.g. `RR` = right R).
    R = 6,
    A = 7,
    O = 8,
    STAR1 = 9,
    RE = 10,
    RU = 11,
    RF = 12,
    RR = 13,
    RP = 14,
    RB = 15,
    RL = 16,
    RG = 17,
    RT = 18,
    RS = 19,
    RD = 20,
    RZ = 21,
    NUM1 = 22,

    // Extended steno keys (chart indices 23-37)
    S2 = 23,
    STAR2 = 24,
    STAR3 = 25,
    STAR4 = 26,
    NUM2 = 27,
    NUM3 = 28,
    NUM4 = 29,
    NUM5 = 30,
    NUM6 = 31,
    NUM7 = 32,
    NUM8 = 33,
    NUM9 = 34,
    NUMA = 35,
    NUMB = 36,
    NUMC = 37,

    // Extra vendor-defined keys X1..X26 (chart indices 38-63)
    X1 = 38,
    X2 = 39,
    X3 = 40,
    X4 = 41,
    X5 = 42,
    X6 = 43,
    X7 = 44,
    X8 = 45,
    X9 = 46,
    X10 = 47,
    X11 = 48,
    X12 = 49,
    X13 = 50,
    X14 = 51,
    X15 = 52,
    X16 = 53,
    X17 = 54,
    X18 = 55,
    X19 = 56,
    X20 = 57,
    X21 = 58,
    X22 = 59,
    X23 = 60,
    X24 = 61,
    X25 = 62,
    X26 = 63,
}

impl StenoKey {
    /// Alias for the most commonly used asterisk bit.
    pub const STAR: Self = Self::STAR1;

    /// Returns the chart index (0..=63) of this key.
    #[inline]
    pub const fn chart_index(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each chart index decodes to the key with that discriminant and re-encodes
    /// to the same byte, so the test also guards the chart-order invariant above:
    /// an inserted or reordered variant shifts serde's ordinals off the discriminants.
    #[test]
    fn postcard_round_trip_is_single_chart_index_byte() {
        assert_eq!(<StenoKey as MaxSize>::POSTCARD_MAX_SIZE, 1);
        for i in 0u8..=63 {
            let key: StenoKey = postcard::from_bytes(&[i]).expect("valid chart index");
            assert_eq!(key.chart_index(), i);
            let mut buf = [0u8; 4];
            let out = postcard::serialize_with_flavor(&key, postcard::ser_flavors::Slice::new(&mut buf))
                .expect("infallible flavor");
            assert_eq!(out, &[i]);
        }
    }
}
