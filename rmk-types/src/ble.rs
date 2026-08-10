//! BLE status types and advertising constants.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Company identifier in RMK's manufacturer-specific advertising data.
pub const RMK_ADV_COMPANY_ID: u16 = 0x5253;

/// First manufacturer-specific-data byte of a keyboard's dongle-seeking
/// advertisement, followed by the Rynk protocol major version. Every RMK
/// advertisement kind shares [`RMK_ADV_COMPANY_ID`], so this must not collide
/// with the split-peripheral payload, whose first byte is a small peripheral id.
pub const DONGLE_SEEKING_ADV_KIND: u8 = 0xD0;

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
