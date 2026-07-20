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
}

/// Access to RMK's standard lighting state from a board-extended snapshot.
///
/// A board may add battery, connection, or sensor fields to its snapshot and
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
