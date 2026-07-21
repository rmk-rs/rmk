//! Adapters from authoritative RMK state to lighting snapshots.

use super::{IndicatorState, LayerState, LightingContext, SnapshotProvider};
use crate::keymap::KeyMap;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TooManyLayers {
    pub configured: usize,
    pub supported: usize,
}

/// Authoritative layer snapshot provider backed by the live RMK keymap.
///
/// Layer-change events are only wakeups. Every render and command reads this
/// provider, so startup and coalesced events cannot leave lighting with a
/// reconstructed or stale active-layer set.
#[derive(Clone, Copy)]
pub struct KeymapLightingState<'keymap, 'data> {
    keymap: &'keymap KeyMap<'data>,
}

impl<'keymap, 'data> KeymapLightingState<'keymap, 'data> {
    pub fn new(keymap: &'keymap KeyMap<'data>) -> Result<Self, TooManyLayers> {
        let configured = keymap.num_layer();
        if configured > LayerState::CAPACITY as usize {
            return Err(TooManyLayers {
                configured,
                supported: LayerState::CAPACITY as usize,
            });
        }
        Ok(Self { keymap })
    }

    pub const fn keymap(&self) -> &'keymap KeyMap<'data> {
        self.keymap
    }
}

impl SnapshotProvider for KeymapLightingState<'_, '_> {
    type Snapshot = LightingContext;

    fn snapshot(&self) -> Self::Snapshot {
        let effective = self.keymap.get_activated_layer();
        let default = self.keymap.get_default_layer();
        let mut active = 1_u64 << default;
        for layer in 0..self.keymap.num_layer() as u8 {
            if self.keymap.is_layer_active(layer) {
                active |= 1_u64 << layer;
            }
        }
        LightingContext {
            layers: LayerState::new(effective, default, active),
            indicators: indicator_state(),
            powered: crate::state::current_usb_state().is_powered(),
        }
    }
}

fn indicator_state() -> IndicatorState {
    let indicator = crate::keyboard::current_led_indicator();
    IndicatorState {
        num_lock: indicator.num_lock(),
        caps_lock: indicator.caps_lock(),
        scroll_lock: indicator.scroll_lock(),
        compose: indicator.compose(),
        kana: indicator.kana(),
    }
}
