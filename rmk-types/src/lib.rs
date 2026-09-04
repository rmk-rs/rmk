//! # RMK Types
//!
//! Shared type definitions used across the RMK keyboard firmware ecosystem.
//!
//! ## Modules
//!
//! ### Actions & keycodes
//! - [`action`] — What keys do: `Action`, `KeyAction`, `EncoderAction`, `LightAction`, etc.
//! - [`keycode`] — What keys are: `KeyCode`, `HidKeyCode`, `ConsumerKey`, `SystemControlKey`
//!
//! ### Behaviors (key overrides, combos, tap-dance)
//! - [`combo`] — `Combo`: combo trigger configuration
//! - [`fork`] — `Fork`, `StateBits`: key-override configuration
//! - [`morse`] — `Morse`, `MorsePattern`, `MorseProfile`, `MorseMode`: tap-dance/tap-hold
//!
//! ### Hardware state
//! - [`modifier`] — `ModifierCombination` bitfield
//! - [`mouse_button`] — `MouseButtons` bitfield
//! - [`led_indicator`] — `LedIndicator` bitfield
//! - [`battery`] — `BatteryStatus`, `ChargeState`
//! - [`ble`] — `BleStatus`, `BleState`
//! - [`connection`] — `ConnectionType` (USB/BLE), `UsbState`, `ConnectionStatus`
//!
//! ### Protocol
//! - [`protocol::vial`] — Vial/Via protocol types
//! - [`protocol::rynk`] — RMK native protocol ICD (feature-gated: `rynk`)
//!
//! ### Build-time
//! - [`constants`] — Generated from `keyboard.toml` by `build.rs`

#![cfg_attr(not(feature = "wasm"), no_std)]

// The host build (no_std, but on an allocator-backed platform) uses `alloc::Vec`
// for bulk message fields, which are unbounded there — see `protocol::rynk`.
#[cfg(feature = "host")]
extern crate alloc;

pub mod action;
pub mod battery;
pub mod ble;
pub mod combo;
pub mod connection;
pub mod constants;
#[cfg(feature = "dfu")]
pub mod dfu;
pub mod fmt;
pub mod fork;
pub mod keycode;
pub mod led_indicator;
pub mod modifier;
pub mod morse;
pub mod mouse_button;
pub mod protocol;
#[cfg(feature = "steno")]
pub mod steno;
pub mod sticky_key;

/// Compute the maximum varint-encoded length for a given max value.
/// Mirrors `postcard`'s internal `varint_size`.
pub(crate) const fn varint_max_size(max_n: usize) -> usize {
    const BITS_PER_BYTE: usize = 8;
    const BITS_PER_VARINT_BYTE: usize = 7;
    if max_n == 0 {
        return 1;
    }
    let bits = core::mem::size_of::<usize>() * BITS_PER_BYTE - max_n.leading_zeros() as usize;
    let roundup_bits = bits + (BITS_PER_VARINT_BYTE - 1);
    roundup_bits / BITS_PER_VARINT_BYTE
}

/// Worst-case postcard-encoded size of `heapless::Vec<T, N>`:
/// every element at its own max, plus the widest varint for the length prefix.
///
/// Use this in manual `MaxSize` impls for structs whose fields contain
/// `heapless::Vec<T, N>`, since `#[derive(MaxSize)]` doesn't support `heapless::Vec`.
/// TODO: Use derived `MaxSize` after postcard updates its heapless version.
pub(crate) const fn heapless_vec_max_size<T: postcard::experimental::max_size::MaxSize, const N: usize>() -> usize {
    T::POSTCARD_MAX_SIZE * N + varint_max_size(N)
}

#[cfg(test)]
mod tests {
    use heapless::Vec;

    use super::{heapless_vec_max_size, varint_max_size};

    /// Validate varint_max_size against known postcard varint encoding sizes
    /// and cross-check with actual postcard serialization.
    #[test]
    fn varint_max_size_matches_postcard() {
        // Known varint size boundaries
        assert_eq!(varint_max_size(0), 1);
        assert_eq!(varint_max_size(1), 1);
        assert_eq!(varint_max_size(127), 1);
        assert_eq!(varint_max_size(128), 2);
        assert_eq!(varint_max_size(16383), 2);
        assert_eq!(varint_max_size(16384), 3);

        // Cross-check: serialize actual values with postcard and verify
        // the varint prefix length doesn't exceed our calculation
        let mut buf = [0u8; 16];
        for &n in &[0usize, 1, 127, 128, 255, 256, 16383, 16384, 65535] {
            let bytes = postcard::to_slice(&n, &mut buf).unwrap();
            assert!(
                bytes.len() <= varint_max_size(n),
                "varint_max_size({n}) = {} but postcard used {} bytes",
                varint_max_size(n),
                bytes.len()
            );
        }
    }

    /// Worst-case `Vec<u32, 8>` (every element at `u32::MAX`, max-width varint
    /// length prefix) must encode to exactly `heapless_vec_max_size::<u32, 8>()`.
    #[test]
    fn heapless_vec_max_size_matches_postcard() {
        let mut v: Vec<u32, 8> = Vec::new();
        for _ in 0..8 {
            v.push(u32::MAX).unwrap();
        }
        let mut buf = [0u8; 64];
        let bytes = postcard::to_slice(&v, &mut buf).unwrap();
        assert_eq!(
            bytes.len(),
            heapless_vec_max_size::<u32, 8>(),
            "tight bound: 8 × u32::MAX + varint(8)",
        );

        // Empty Vec is below the bound (loose check).
        let empty: Vec<u32, 8> = Vec::new();
        let bytes = postcard::to_slice(&empty, &mut buf).unwrap();
        assert!(bytes.len() <= heapless_vec_max_size::<u32, 8>());
    }
}
