//! BLE status types.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// BLE state (what the BLE subsystem is currently doing).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum BleState {
    /// The BLE is advertising.
    Advertising,
    /// The BLE is connected.
    Connected,
    /// The BLE is not in use (USB mode or sleep mode, default).
    Inactive,
}

/// Unified BLE status: which profile is active and what the BLE is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct BleStatus {
    pub profile: u8,
    pub state: BleState,
}

impl Default for BleStatus {
    fn default() -> Self {
        Self {
            profile: 0,
            state: BleState::Inactive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BleState, BleStatus};

    #[test]
    fn default_ble_status_is_profile_zero_and_inactive() {
        assert_eq!(
            BleStatus::default(),
            BleStatus {
                profile: 0,
                state: BleState::Inactive,
            }
        );
    }

    #[test]
    fn ble_status_variants_are_copy_and_comparable() {
        let advertising = BleStatus {
            profile: 0,
            state: BleState::Advertising,
        };
        let connected = BleStatus {
            profile: 2,
            state: BleState::Connected,
        };
        let inactive = BleStatus::default();

        assert_ne!(advertising, connected);
        assert_ne!(connected, inactive);
        assert_eq!(
            inactive,
            BleStatus {
                profile: 0,
                state: BleState::Inactive,
            }
        );
    }
}
