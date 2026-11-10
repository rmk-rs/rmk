//! BLE status types and the limits an identity string has to clear.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Longest name a legacy advertisement can carry: flags (3), the battery + HID service
/// UUIDs (6) and the appearance (4) claim the rest of its 31 bytes.
pub const BLE_ADV_NAME_MAX_LEN: usize = 16;

/// Connection and advertising state of one BLE host or dongle profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum BleState {
    /// The profile is advertising.
    Advertising,
    /// The profile has an encrypted connection that can carry input.
    Connected,
    /// The profile has no connection ready for input.
    #[default]
    Inactive,
}

/// Connection state and bond of one BLE host or dongle profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct BleStatus {
    pub state: BleState,
    /// Whether the profile holds a bond, independent of connection state. False while it re-pairs.
    pub bonded: bool,
}
