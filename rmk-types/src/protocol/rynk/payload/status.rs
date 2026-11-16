//! Status endpoint types.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Maximum bitmap size: supports up to 256 keys (e.g., 16 rows x 16 cols).
/// Each row uses ceil(num_cols / 8) bytes. Host decodes using num_rows/num_cols
/// from DeviceCapabilities.
pub const MATRIX_BITMAP_SIZE: usize = 32;

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

/// Status of a single split peripheral. Peripherals without battery reporting
/// have `battery: Unavailable`.
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

    #[cfg(feature = "split")]
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
