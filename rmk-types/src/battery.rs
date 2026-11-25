//! Battery status and BLE battery identifiers.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Identifies the battery represented by a BLE Battery Level characteristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BleBatteryId {
    Left,
    Right,
    /// Battery of a unibody keyboard or a split central without a `Left` or `Right` identity.
    /// Reported as GATT `main` (`0x0106`).
    Central,
    /// Zero-based board index for an RMK peripheral battery service.
    Peripheral(u8),
}

/// Charge state of the battery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum ChargeState {
    Charging,
    Discharging,
    Unknown,
}

impl From<bool> for ChargeState {
    /// `true` = Charging, `false` = Discharging.
    fn from(charging: bool) -> Self {
        if charging {
            ChargeState::Charging
        } else {
            ChargeState::Discharging
        }
    }
}

/// Battery status used for both status queries and event notifications.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum BatteryStatus {
    Unavailable,
    Available {
        charge_state: ChargeState,
        level: Option<u8>,
    },
}

impl BatteryStatus {
    pub fn is_available(&self) -> bool {
        matches!(self, BatteryStatus::Available { .. })
    }
}
