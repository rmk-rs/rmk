use rmk_types::ble::BleState;
use rmk_types::connection::{ConnectionStatus, ConnectionType};

/// Bounded snapshot of RMK's layer state.
///
/// The complete active set is retained because the effective layer alone is
/// insufficient for transparent fallthrough and held-layer indicators.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LayerState {
    pub effective: u8,
    pub default: u8,
    active: u64,
}

impl LayerState {
    pub const CAPACITY: u8 = 64;

    pub const fn new(effective: u8, default: u8, active: u64) -> Self {
        Self {
            effective,
            default,
            active,
        }
    }

    pub const fn active_bits(self) -> u64 {
        self.active
    }

    pub const fn is_active(self, layer: u8) -> bool {
        layer < Self::CAPACITY && self.active & (1_u64 << layer) != 0
    }
}

impl Default for LayerState {
    fn default() -> Self {
        Self::new(0, 0, 1)
    }
}

/// Host-controlled keyboard indicators available to lighting sources.
///
/// This deliberately mirrors the semantic HID state rather than exposing its
/// wire bitfield. Sources can therefore use the same context on USB, BLE, and
/// split peripherals.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct IndicatorState {
    pub num_lock: bool,
    pub caps_lock: bool,
    pub scroll_lock: bool,
    pub compose: bool,
    pub kana: bool,
}

/// State RMK makes available to standard and external lighting sources.
/// Additional firmware-specific state can be carried in a source of the
/// board's own type; it does not belong in the core compositor.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct LightingContext {
    pub layers: LayerState,
    pub indicators: IndicatorState,
    /// USB/VBUS power is present at the authority. This remains true while
    /// USB is suspended. A split replica receives the authority's value in
    /// its replicated context.
    pub powered: bool,
    /// USB/VBUS power is present at the node doing the rendering. Equal to
    /// `powered` on the authority; a replica substitutes its own VBUS so a
    /// `PoweredOnlyScope::Local` board can follow it.
    pub local_powered: bool,
    /// Authoritative transport, BLE profile, and BLE lifecycle snapshot.
    pub connection: LightingConnection,
    /// BLE profile slots holding a stored bond, as a bitmap (bit N = slot N).
    ///
    /// `connection` only describes the *selected* profile, so on its own it
    /// can never say whether some other slot is paired. This makes every
    /// slot's paired-versus-empty state observable at once, which is what lets
    /// a board light one indicator per slot key. Zero without `_ble`.
    pub bonded_slots: u8,
}

/// The selected-profile view of [`ConnectionStatus`] that lighting reads.
/// Holding one profile rather than the whole list keeps the context `Copy`.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct LightingConnection {
    /// The transport carrying input, as [`ConnectionStatus::decide_active`] picks it.
    pub active: Option<ConnectionType>,
    /// USB is plugged and routable, whether or not it is the active transport.
    pub usb_ready: bool,
    /// The selected BLE profile.
    pub profile: u8,
    /// The selected BLE profile's state.
    pub ble_state: BleState,
}

impl From<&ConnectionStatus> for LightingConnection {
    fn from(status: &ConnectionStatus) -> Self {
        Self {
            active: status.decide_active(),
            usb_ready: status.usb_ready(),
            profile: status.selected,
            ble_state: status.selected_profile().state,
        }
    }
}

/// Access to RMK's standard lighting state from a board-extended snapshot.
///
/// A board may add battery or sensor fields to its snapshot and
/// still reuse built-in layer and indicator sources by implementing this
/// trait. The compositor itself remains generic over the complete context.
pub trait LightingContextProvider {
    fn lighting_context(&self) -> &LightingContext;
}

impl LightingContextProvider for LightingContext {
    fn lighting_context(&self) -> &LightingContext {
        self
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::ble::BleState;
    use rmk_types::connection::{ConnectionStatus, ConnectionType, UsbState};

    use super::LightingConnection;

    #[test]
    fn connection_view_follows_the_selected_profile() {
        let mut status = ConnectionStatus {
            usb: UsbState::Suspended,
            selected: 1,
            preferred: ConnectionType::Ble,
            ..ConnectionStatus::new()
        };
        status.ble[0].state = BleState::Connected;
        status.ble[1].state = BleState::Advertising;

        assert_eq!(
            LightingConnection::from(&status),
            LightingConnection {
                active: Some(ConnectionType::Usb),
                usb_ready: true,
                profile: 1,
                ble_state: BleState::Advertising,
            }
        );
    }
}
