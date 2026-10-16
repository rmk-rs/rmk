//! Shared context for the Vial and Rynk host services.

use embassy_time::Duration;
use rmk_types::action::{EncoderAction, KeyAction};
#[cfg(feature = "_ble")]
use rmk_types::battery::BatteryStatus;
use rmk_types::combo::Combo as ComboConfig;
use rmk_types::connection::{ConnectionStatus, ConnectionType};
use rmk_types::fork::Fork;
use rmk_types::led_indicator::LedIndicator;
use rmk_types::morse::{Morse, MorseProfile};
#[cfg(feature = "rynk")]
use rmk_types::protocol::rynk::{
    BehaviorConfig, MorseHoldTriggerPosition, MorseHoldTriggerPositionState, MorseHoldTriggerPositions,
    SetMorseHoldTriggerPositionsRequest,
};

#[cfg(feature = "rynk")]
use crate::config::HoldTriggerPositions;
use crate::event::KeyboardEventPos;
use crate::keyboard::combo::Combo;
use crate::keymap::KeyMap;
#[cfg(feature = "storage")]
use crate::storage::{StorageItem, store};

/// Context shared between Vial and Rynk host services.
pub(crate) struct KeyboardContext<'a> {
    pub keymap: &'a KeyMap<'a>,
    pub(crate) layout_blob: &'static [u8],
}

impl<'a> KeyboardContext<'a> {
    pub fn new(keymap: &'a KeyMap<'a>) -> Self {
        Self {
            keymap,
            layout_blob: &[],
        }
    }

    pub fn get_action(&self, layer: u8, row: u8, col: u8) -> KeyAction {
        self.keymap
            .get_action_at(KeyboardEventPos::key_pos(col, row), layer as usize)
    }

    pub fn get_action_flat(&self, index: usize) -> KeyAction {
        self.keymap.get_action_by_flat_index(index)
    }

    /// `(rows, cols, num_layers)`.
    pub fn keymap_dimensions(&self) -> (usize, usize, usize) {
        self.keymap.get_keymap_config()
    }

    /// The opaque, compressed physical-layout blob served by `GetLayout`.
    pub fn layout_blob(&self) -> &'static [u8] {
        self.layout_blob
    }

    pub async fn set_action(&self, layer: u8, row: u8, col: u8, action: KeyAction) -> Result<(), ()> {
        self.keymap
            .set_action_at(KeyboardEventPos::key_pos(col, row), layer as usize, action);
        #[cfg(feature = "storage")]
        store(StorageItem::Keymap {
            layer,
            row,
            col,
            action,
        })
        .await?;
        Ok(())
    }

    pub fn get_encoder(&self, layer: u8, idx: u8) -> Option<EncoderAction> {
        self.keymap.get_encoder_action(layer as usize, idx as usize)
    }

    /// Number of encoders per layer.
    pub fn num_encoders(&self) -> usize {
        self.keymap.num_encoders()
    }

    /// Write one encoder direction and persist the updated pair.
    pub async fn set_encoder_direction(
        &self,
        layer: u8,
        idx: u8,
        clockwise: bool,
        action: KeyAction,
    ) -> Result<(), ()> {
        let updated = if clockwise {
            self.keymap.set_encoder_clockwise(layer as usize, idx as usize, action)
        } else {
            self.keymap
                .set_encoder_counter_clockwise(layer as usize, idx as usize, action)
        };
        #[cfg(feature = "storage")]
        if let Some(encoder) = updated {
            store(StorageItem::Encoder {
                layer,
                idx,
                action: encoder,
            })
            .await?;
        }
        #[cfg(not(feature = "storage"))]
        let _ = updated;
        Ok(())
    }

    /// Write both encoder directions in one synchronous RAM update, then persist
    /// once.
    pub async fn set_encoder(&self, layer: u8, idx: u8, action: EncoderAction) -> Result<(), ()> {
        let written = self.keymap.set_encoder(layer as usize, idx as usize, action);
        #[cfg(feature = "storage")]
        if written {
            store(StorageItem::Encoder { layer, idx, action }).await?;
        }
        #[cfg(not(feature = "storage"))]
        let _ = written;
        Ok(())
    }

    pub fn with_combos<R>(&self, f: impl FnOnce(&[Option<Combo>]) -> R) -> R {
        self.keymap.with_combos(f)
    }

    /// Replace the combo at `idx` with `config` (or remove it if `config` is
    /// empty) and persist. No-op if `idx` is out of range.
    /// Returns `false` when `idx` is out of range (no slot written).
    /// `Ok(false)` when `idx` is out of range (no slot written).
    pub async fn set_combo(&self, idx: u8, config: ComboConfig) -> Result<bool, ()> {
        let valid = self.keymap.with_combos_mut(|combos| {
            if (idx as usize) >= combos.len() {
                return false;
            }
            combos[idx as usize] = if config.actions.is_empty() && config.output == KeyAction::No {
                None
            } else {
                Some(Combo::new(config.clone()))
            };
            true
        });
        if !valid {
            return Ok(false);
        }
        #[cfg(feature = "storage")]
        store(StorageItem::Combo { idx, config }).await?;
        #[cfg(not(feature = "storage"))]
        let _ = config;
        Ok(true)
    }

    pub fn get_morse(&self, idx: u8) -> Option<Morse> {
        self.keymap.get_morse(idx as usize)
    }

    pub fn morses_len(&self) -> usize {
        self.keymap.morses_len()
    }

    /// Mutate the morse at `idx` and persist. No-op if `idx` is out of range.
    pub async fn update_morse(&self, idx: u8, f: impl FnOnce(&mut Morse)) -> Result<(), ()> {
        #[cfg(feature = "storage")]
        {
            let updated = self.keymap.with_morse_mut(idx as usize, |morse| {
                f(morse);
                morse.clone()
            });
            if let Some(morse) = updated {
                store(StorageItem::Morse { idx, morse }).await?;
            }
        }
        #[cfg(not(feature = "storage"))]
        {
            self.keymap.with_morse_mut(idx as usize, f);
        }
        Ok(())
    }

    pub fn combo_timeout(&self) -> Duration {
        self.keymap.combo_timeout()
    }

    pub fn one_shot_timeout(&self) -> Duration {
        self.keymap.one_shot_timeout()
    }

    pub fn tap_interval(&self) -> u16 {
        self.keymap.tap_interval()
    }

    pub fn tap_capslock_interval(&self) -> u16 {
        self.keymap.tap_capslock_interval()
    }

    pub fn morse_default_profile(&self) -> MorseProfile {
        self.keymap.morse_default_profile()
    }

    pub fn morse_prior_idle_time(&self) -> Duration {
        self.keymap.morse_prior_idle_time()
    }

    #[cfg(feature = "rynk")]
    pub fn morse_hold_trigger_positions(&self) -> MorseHoldTriggerPositionState {
        // `Extend` works for both the firmware's bounded table and a host build's `Vec`.
        let mut positions = MorseHoldTriggerPositions::new();
        positions.extend(
            self.keymap
                .morse_hold_trigger_positions()
                .iter()
                .map(|&(profile, row, col)| MorseHoldTriggerPosition { profile, row, col }),
        );
        MorseHoldTriggerPositionState {
            capacity: crate::HOLD_TRIGGER_KEY_POSITION_MAX_NUM as u8,
            positions,
        }
    }

    #[cfg(feature = "rynk")]
    pub async fn set_morse_hold_trigger_positions(
        &self,
        request: SetMorseHoldTriggerPositionsRequest,
    ) -> Result<(), ()> {
        let mut stored = HoldTriggerPositions::new();
        for position in request.positions {
            stored
                .push((position.profile, position.row, position.col))
                .expect("Rynk decoder enforces the compiled hold-trigger capacity");
        }
        self.keymap.set_morse_hold_trigger_positions(stored.clone());
        #[cfg(feature = "storage")]
        store(StorageItem::MorseHoldTriggerPositions(stored)).await?;
        Ok(())
    }

    pub async fn set_combo_timeout(&self, ms: u16) -> Result<(), ()> {
        self.keymap.set_combo_timeout(Duration::from_millis(ms as u64));
        #[cfg(feature = "storage")]
        store(StorageItem::BehaviorConfig(self.keymap.behavior_snapshot())).await?;
        Ok(())
    }

    pub async fn set_one_shot_timeout(&self, ms: u16) -> Result<(), ()> {
        self.keymap.set_one_shot_timeout(Duration::from_millis(ms as u64));
        #[cfg(feature = "storage")]
        store(StorageItem::BehaviorConfig(self.keymap.behavior_snapshot())).await?;
        Ok(())
    }

    pub async fn set_tap_interval(&self, ms: u16) -> Result<(), ()> {
        self.keymap.set_tap_interval(ms);
        #[cfg(feature = "storage")]
        store(StorageItem::BehaviorConfig(self.keymap.behavior_snapshot())).await?;
        Ok(())
    }

    pub async fn set_tap_capslock_interval(&self, ms: u16) -> Result<(), ()> {
        self.keymap.set_tap_capslock_interval(ms);
        #[cfg(feature = "storage")]
        store(StorageItem::BehaviorConfig(self.keymap.behavior_snapshot())).await?;
        Ok(())
    }

    pub async fn set_morse_default_profile(&self, profile: MorseProfile) -> Result<(), ()> {
        self.keymap.set_morse_default_profile(profile);
        #[cfg(feature = "storage")]
        store(StorageItem::BehaviorConfig(self.keymap.behavior_snapshot())).await?;
        Ok(())
    }

    pub async fn set_morse_prior_idle_time(&self, ms: u16) -> Result<(), ()> {
        self.keymap.set_morse_prior_idle_time(Duration::from_millis(ms as u64));
        #[cfg(feature = "storage")]
        store(StorageItem::BehaviorConfig(self.keymap.behavior_snapshot())).await?;
        Ok(())
    }

    #[cfg(feature = "rynk")]
    pub async fn set_behavior_config(&self, cfg: BehaviorConfig) -> Result<(), ()> {
        self.keymap
            .set_combo_timeout(Duration::from_millis(cfg.combo_timeout_ms as u64));
        self.keymap
            .set_one_shot_timeout(Duration::from_millis(cfg.oneshot_timeout_ms as u64));
        self.keymap.set_tap_interval(cfg.tap_interval_ms);
        self.keymap.set_tap_capslock_interval(cfg.tap_capslock_interval_ms);
        self.keymap.set_morse_default_profile(cfg.morse_default_profile);
        self.keymap
            .set_morse_prior_idle_time(Duration::from_millis(cfg.morse_prior_idle_time_ms as u64));
        #[cfg(feature = "storage")]
        store(StorageItem::BehaviorConfig(self.keymap.behavior_snapshot())).await?;
        Ok(())
    }

    pub async fn set_layout_options(&self, opts: u32) -> Result<(), ()> {
        self.keymap.set_layout_option(opts);
        #[cfg(feature = "storage")]
        store(StorageItem::LayoutOption(opts)).await?;
        Ok(())
    }

    pub fn layout_options(&self) -> u32 {
        self.keymap.layout_option()
    }

    pub async fn reset_storage(&self) {
        #[cfg(feature = "storage")]
        crate::storage::reset().await;
    }

    pub fn led_indicator(&self) -> LedIndicator {
        crate::keyboard::current_led_indicator()
    }

    pub fn connection_status(&self) -> ConnectionStatus {
        crate::state::current_connection_status()
    }

    #[cfg(feature = "_ble")]
    pub fn battery_status(&self) -> BatteryStatus {
        crate::input_device::battery::current_battery_status()
    }

    pub fn active_layer(&self) -> u8 {
        self.keymap.active_layer()
    }

    pub fn default_layer(&self) -> u8 {
        self.keymap.get_default_layer()
    }

    pub async fn set_default_layer(&self, layer: u8) -> Result<(), ()> {
        self.keymap.set_default_layer(layer);
        #[cfg(feature = "storage")]
        store(StorageItem::DefaultLayer(layer)).await?;
        Ok(())
    }

    /// Tiebreaker connection currently chosen as preferred — independent
    /// of which transport is actively routable.
    pub fn preferred_connection(&self) -> ConnectionType {
        crate::state::current_connection_status().preferred
    }

    pub fn get_fork(&self, idx: u8) -> Option<Fork> {
        self.keymap.with_forks(|forks| forks.get(idx as usize).copied())
    }

    /// Replace the fork at `idx` with `fork` and persist.
    /// `Ok(false)` when `idx` is out of range (no slot written).
    pub async fn set_fork(&self, idx: u8, fork: Fork) -> Result<bool, ()> {
        let valid = self.keymap.with_forks_mut(|forks| {
            if let Some(slot) = forks.get_mut(idx as usize) {
                *slot = fork;
                true
            } else {
                false
            }
        });
        #[cfg(feature = "storage")]
        if valid {
            store(StorageItem::Fork { idx, fork }).await?;
        }
        Ok(valid)
    }

    #[cfg(feature = "host_lock")]
    pub fn read_matrix_state(&self, target: &mut [u8]) {
        self.keymap.read_matrix_state(target);
    }
}
