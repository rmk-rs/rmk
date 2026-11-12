//! Shared connection type definitions used across RMK crates.

use heapless::Vec;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::ble::{BleState, BleStatus};
use crate::constants::BOND_SLOTS;

/// Connection type for the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum ConnectionType {
    Usb,
    Ble,
}

/// USB device lifecycle. `Suspended` is distinct from `Configured` because
/// the bus is enumerated but transmission is gated on remote wakeup — the
/// first key still needs to reach the USB writer to trigger that wakeup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[derive(Default)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum UsbState {
    #[default]
    Disabled,
    Enabled,
    Configured,
    Suspended,
}

/// Unified connection status: the single source of truth for transport
/// availability and routing. The active transport is derived on demand via
/// [`Self::decide_active`] from the input fields below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct ConnectionStatus {
    pub usb: UsbState,
    /// Every host profile, then the dongle profile when there is one, indexed by profile number.
    #[cfg_attr(feature = "wasm", tsify(type = "BleStatus[]"))]
    pub ble: Vec<BleStatus, BOND_SLOTS>,
    /// The profile that carries input when BLE is the active transport.
    pub selected: u8,
    /// Tiebreaker when both transports are ready.
    pub preferred: ConnectionType,
}

impl MaxSize for ConnectionStatus {
    const POSTCARD_MAX_SIZE: usize = UsbState::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<BleStatus, BOND_SLOTS>()
        + u8::POSTCARD_MAX_SIZE
        + ConnectionType::POSTCARD_MAX_SIZE;
}

impl ConnectionStatus {
    pub const fn new() -> Self {
        Self {
            usb: UsbState::Disabled,
            ble: Vec::from_array(
                [BleStatus {
                    state: BleState::Inactive,
                    bonded: false,
                }; BOND_SLOTS],
            ),
            selected: 0,
            preferred: ConnectionType::Usb,
        }
    }
}

impl Default for ConnectionStatus {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionStatus {
    /// Status of the selected profile; `Inactive` on a firmware without BLE profiles.
    pub fn selected_profile(&self) -> BleStatus {
        self.ble.get(self.selected as usize).copied().unwrap_or_default()
    }

    fn usb_ready(&self) -> bool {
        matches!(self.usb, UsbState::Configured | UsbState::Suspended)
    }

    fn ble_ready(&self) -> bool {
        matches!(self.selected_profile().state, BleState::Connected)
    }

    /// Pick the active transport from current readiness + preference. Suspended
    /// USB remains routable for remote wakeup, so it participates in the same
    /// preference tie-break as configured USB.
    pub fn decide_active(&self) -> Option<ConnectionType> {
        match (self.usb_ready(), self.ble_ready()) {
            (true, false) => Some(ConnectionType::Usb),
            (false, true) => Some(ConnectionType::Ble),
            (true, true) => Some(self.preferred),
            (false, false) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ConnectionStatus, ConnectionType, UsbState};
    use crate::ble::BleState;

    fn status(usb: UsbState, ble_state: BleState, preferred: ConnectionType) -> ConnectionStatus {
        let mut status = ConnectionStatus {
            usb,
            preferred,
            ..ConnectionStatus::new()
        };
        status.ble[0].state = ble_state;
        status
    }

    #[test]
    fn decide_active_only_usb_ready() {
        let s = status(UsbState::Configured, BleState::Inactive, ConnectionType::Ble);
        assert_eq!(s.decide_active(), Some(ConnectionType::Usb));
    }

    #[test]
    fn decide_active_only_ble_ready() {
        let s = status(UsbState::Disabled, BleState::Connected, ConnectionType::Usb);
        assert_eq!(s.decide_active(), Some(ConnectionType::Ble));
    }

    #[test]
    fn decide_active_both_ready_prefers_preference() {
        let s = status(UsbState::Configured, BleState::Connected, ConnectionType::Ble);
        assert_eq!(s.decide_active(), Some(ConnectionType::Ble));
        let s = status(UsbState::Configured, BleState::Connected, ConnectionType::Usb);
        assert_eq!(s.decide_active(), Some(ConnectionType::Usb));
    }

    #[test]
    fn decide_active_neither_ready_is_none() {
        let s = status(UsbState::Disabled, BleState::Inactive, ConnectionType::Usb);
        assert_eq!(s.decide_active(), None);
    }

    #[test]
    fn suspended_usb_stays_routable_for_remote_wakeup() {
        let s = status(UsbState::Suspended, BleState::Advertising, ConnectionType::Ble);
        assert_eq!(s.decide_active(), Some(ConnectionType::Usb));
        assert!(s.usb_ready());
    }

    #[test]
    fn suspended_usb_with_connected_ble_prefers_preference() {
        let s = status(UsbState::Suspended, BleState::Connected, ConnectionType::Usb);
        assert_eq!(s.decide_active(), Some(ConnectionType::Usb));
        assert!(s.usb_ready());
        assert!(s.ble_ready());

        let s = status(UsbState::Suspended, BleState::Connected, ConnectionType::Ble);
        assert_eq!(s.decide_active(), Some(ConnectionType::Ble));
        assert!(s.usb_ready());
        assert!(s.ble_ready());
    }
}
