//! Shared connection type definitions used across RMK crates.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::ble::{BleState, BleStatus};

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

impl UsbState {
    /// Whether VBUS is present, independent of enumeration or suspension.
    pub const fn is_powered(self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

/// Unified connection status: the single source of truth for transport
/// availability and routing. The active transport is derived on demand via
/// [`Self::decide_active`] from the input fields below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct ConnectionStatus {
    pub usb: UsbState,
    pub ble: BleStatus,
    /// Tiebreaker when both transports are ready.
    pub preferred: ConnectionType,
}

impl ConnectionStatus {
    pub const fn new() -> Self {
        Self {
            usb: UsbState::Disabled,
            ble: BleStatus {
                profile: 0,
                state: BleState::Inactive,
            },
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
    fn usb_ready(&self) -> bool {
        matches!(self.usb, UsbState::Configured | UsbState::Suspended)
    }

    fn ble_ready(&self) -> bool {
        matches!(self.ble.state, BleState::Connected)
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
    use crate::ble::{BleState, BleStatus};

    fn status(usb: UsbState, ble_state: BleState, preferred: ConnectionType) -> ConnectionStatus {
        ConnectionStatus {
            usb,
            ble: BleStatus {
                profile: 0,
                state: ble_state,
            },
            preferred,
        }
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
