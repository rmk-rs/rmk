//! Status endpoint types.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Maximum bitmap size: supports up to 256 keys (e.g., 16 rows x 16 cols).
/// Each row uses ceil(num_cols / 8) bytes. Host decodes using num_rows/num_cols
/// from DeviceCapabilities.
pub const MATRIX_BITMAP_SIZE: usize = 32;

/// Number of layers represented by [`LayerState`].
pub const LAYER_STATE_CAPACITY: usize = 64;
/// Serialized byte width of [`LayerState::active_bitmap`].
pub const LAYER_STATE_BITMAP_SIZE: usize = LAYER_STATE_CAPACITY / 8;

/// Authoritative snapshot of every layer participating in key resolution.
///
/// RMK stores the default layer separately from its mutable layer mask. The
/// corresponding bit in `active_bitmap` is nevertheless always set by the
/// firmware so callers can treat this as the complete active set. Bit `n`
/// reports layer `n`, least-significant bit first within each byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LayerState {
    pub default_layer: u8,
    #[cfg_attr(feature = "wasm", tsify(type = "number[]"))]
    pub active_bitmap: [u8; LAYER_STATE_BITMAP_SIZE],
}

impl LayerState {
    /// Whether `layer` participates in the sampled active layer stack.
    pub const fn is_active(&self, layer: u8) -> bool {
        let index = layer as usize;
        index < LAYER_STATE_CAPACITY && self.active_bitmap[index / 8] & (1_u8 << (index % 8)) != 0
    }
}

/// Current matrix key-press state as a bitmap.
/// Bit ordering: row-major, bit 0 = col 0, bit 1 = col 1, etc.
/// Total meaningful bytes = num_rows * ceil(num_cols / 8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct MatrixState {
    #[cfg_attr(feature = "wasm", tsify(type = "number[]"))]
    pub pressed_bitmap: heapless::Vec<u8, MATRIX_BITMAP_SIZE>,
}

impl MaxSize for MatrixState {
    const POSTCARD_MAX_SIZE: usize = crate::heapless_vec_max_size::<u8, MATRIX_BITMAP_SIZE>();
}

/// Status of a single split peripheral. Wired peripherals report
/// `connected: true` with `battery: Unavailable`.
#[cfg(feature = "split")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct PeripheralStatus {
    pub connected: bool,
    pub battery: crate::battery::BatteryStatus,
}

#[cfg(test)]
mod tests {
    use heapless::Vec;

    use super::*;
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    #[test]
    fn round_trip_layer_state_covers_all_64_layers() {
        let mut active_bitmap = [0; LAYER_STATE_BITMAP_SIZE];
        active_bitmap[0] = 0b0010_0001;
        active_bitmap[7] = 0b1000_0000;
        let state = LayerState {
            default_layer: 5,
            active_bitmap,
        };

        round_trip(&state);
        assert_max_size_bound(&state);
        assert!(state.is_active(0));
        assert!(state.is_active(5));
        assert!(!state.is_active(6));
        assert!(state.is_active(63));
        assert!(!state.is_active(64));
    }

    #[test]
    fn round_trip_matrix_state() {
        let mut bitmap = Vec::new();
        bitmap.extend_from_slice(&[0b0000_0101, 0x00, 0b0010_0000]).unwrap();
        round_trip(&MatrixState { pressed_bitmap: bitmap });

        // Max-capacity case
        let mut bitmap = Vec::new();
        for i in 0..MATRIX_BITMAP_SIZE {
            bitmap.push(i as u8).unwrap();
        }
        let state = MatrixState { pressed_bitmap: bitmap };
        round_trip(&state);
        assert_max_size_bound(&state);
    }

    #[cfg(all(feature = "_ble", feature = "split"))]
    #[test]
    fn round_trip_peripheral_status() {
        use crate::battery::{BatteryStatus, ChargeState};
        round_trip(&PeripheralStatus {
            connected: true,
            battery: BatteryStatus::Available {
                charge_state: ChargeState::Discharging,
                level: Some(85),
            },
        });
        round_trip(&PeripheralStatus {
            connected: false,
            battery: BatteryStatus::Unavailable,
        });
    }
}
