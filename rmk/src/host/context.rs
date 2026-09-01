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
use rmk_types::protocol::rynk::BehaviorConfig;

use crate::event::KeyboardEventPos;
use crate::keyboard::combo::Combo;
use crate::keymap::KeyMap;
#[cfg(feature = "storage")]
use crate::{channel::FLASH_CHANNEL, storage::FlashOperationMessage};

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

    pub async fn set_action(&self, layer: u8, row: u8, col: u8, action: KeyAction) {
        self.keymap
            .set_action_at(KeyboardEventPos::key_pos(col, row), layer as usize, action);
        #[cfg(feature = "storage")]
        FLASH_CHANNEL
            .send(FlashOperationMessage::KeymapKey {
                layer,
                row,
                col,
                action,
            })
            .await;
    }

    /// Synchronous on purpose: Vial's bulk-write path (`DynamicKeymapSetBuffer`)
    /// calls this in a tight loop and would otherwise serialize against flash
    /// for the whole packet. Drops the persist message on a full channel
    /// rather than awaiting capacity, matching pre-context Vial behavior.
    ///
    /// `rows` / `cols` are passed in so callers can hoist the dimensions read
    /// out of their loop — see `keymap_dimensions()`.
    pub fn try_set_action_flat(&self, index: usize, action: KeyAction, rows: usize, cols: usize) {
        self.keymap.set_action_by_flat_index(index, action);
        #[cfg(feature = "storage")]
        {
            let layer_size = rows * cols;
            let layer = index / layer_size;
            let layer_offset = index % layer_size;
            let row = layer_offset / cols;
            let col = layer_offset % cols;
            if FLASH_CHANNEL
                .try_send(FlashOperationMessage::KeymapKey {
                    layer: layer as u8,
                    row: row as u8,
                    col: col as u8,
                    action,
                })
                .is_err()
            {
                error!(
                    "Failed to persist keymap key at layer {} ({},{}): flash channel full",
                    layer, row, col
                );
            }
        }
        #[cfg(not(feature = "storage"))]
        let _ = (rows, cols);
    }

    pub fn get_encoder(&self, layer: u8, idx: u8) -> Option<EncoderAction> {
        self.keymap.get_encoder_action(layer as usize, idx as usize)
    }

    /// Number of encoders per layer.
    pub fn num_encoders(&self) -> usize {
        self.keymap.num_encoders()
    }

    /// Write one encoder direction and persist the updated pair.
    pub async fn set_encoder_direction(&self, layer: u8, idx: u8, clockwise: bool, action: KeyAction) {
        let updated = if clockwise {
            self.keymap.set_encoder_clockwise(layer as usize, idx as usize, action)
        } else {
            self.keymap
                .set_encoder_counter_clockwise(layer as usize, idx as usize, action)
        };
        #[cfg(feature = "storage")]
        if let Some(encoder) = updated {
            FLASH_CHANNEL
                .send(FlashOperationMessage::Encoder {
                    idx,
                    layer,
                    action: encoder,
                })
                .await;
        }
        #[cfg(not(feature = "storage"))]
        let _ = updated;
    }

    /// Write both encoder directions in one synchronous RAM update, then persist
    /// once.
    pub async fn set_encoder(&self, layer: u8, idx: u8, action: EncoderAction) {
        let written = self.keymap.set_encoder(layer as usize, idx as usize, action);
        #[cfg(feature = "storage")]
        if written {
            FLASH_CHANNEL
                .send(FlashOperationMessage::Encoder { idx, layer, action })
                .await;
        }
        #[cfg(not(feature = "storage"))]
        let _ = written;
    }

    pub fn read_macro_buffer(&self, offset: usize, target: &mut [u8]) {
        self.keymap.read_macro_buffer(offset, target);
    }

    /// Vial's protocol expects every set to be followed by a full-buffer save.
    pub async fn write_macro_buffer(&self, offset: usize, data: &[u8]) {
        self.keymap.write_macro_buffer(offset, data);
        #[cfg(feature = "storage")]
        {
            let buf = self.keymap.get_macro_sequences();
            FLASH_CHANNEL.send(FlashOperationMessage::MacroData(buf)).await;
            info!("Flush macros to storage");
        }
    }

    pub fn reset_macro_buffer(&self) {
        self.keymap.reset_macro_buffer();
    }

    pub fn with_combos<R>(&self, f: impl FnOnce(&[Option<Combo>]) -> R) -> R {
        self.keymap.with_combos(f)
    }

    /// Replace the combo at `idx` with `config` (or remove it if `config` is
    /// empty) and persist. No-op if `idx` is out of range.
    /// Returns `false` when `idx` is out of range (no slot written).
    pub async fn set_combo(&self, idx: u8, config: ComboConfig) -> bool {
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
            return false;
        }
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::Combo { idx, config }).await;
        #[cfg(not(feature = "storage"))]
        let _ = config;
        true
    }

    pub fn get_morse(&self, idx: u8) -> Option<Morse> {
        self.keymap.get_morse(idx as usize)
    }

    pub fn morses_len(&self) -> usize {
        self.keymap.morses_len()
    }

    /// Mutate the morse at `idx` and persist. No-op if `idx` is out of range.
    pub async fn update_morse(&self, idx: u8, f: impl FnOnce(&mut Morse)) {
        #[cfg(feature = "storage")]
        {
            let updated = self.keymap.with_morse_mut(idx as usize, |morse| {
                f(morse);
                morse.clone()
            });
            if let Some(morse) = updated {
                FLASH_CHANNEL.send(FlashOperationMessage::Morse { idx, morse }).await;
            }
        }
        #[cfg(not(feature = "storage"))]
        {
            self.keymap.with_morse_mut(idx as usize, f);
        }
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

    pub async fn set_combo_timeout(&self, ms: u16) {
        self.keymap.set_combo_timeout(Duration::from_millis(ms as u64));
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::ComboTimeout(ms)).await;
    }

    pub async fn set_one_shot_timeout(&self, ms: u16) {
        self.keymap.set_one_shot_timeout(Duration::from_millis(ms as u64));
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::OneShotTimeout(ms)).await;
    }

    pub async fn set_tap_interval(&self, ms: u16) {
        self.keymap.set_tap_interval(ms);
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::TapInterval(ms)).await;
    }

    pub async fn set_tap_capslock_interval(&self, ms: u16) {
        self.keymap.set_tap_capslock_interval(ms);
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::TapCapslockInterval(ms)).await;
    }

    pub async fn set_morse_default_profile(&self, profile: MorseProfile) {
        self.keymap.set_morse_default_profile(profile);
        #[cfg(feature = "storage")]
        FLASH_CHANNEL
            .send(FlashOperationMessage::MorseDefaultProfile(profile))
            .await;
    }

    pub async fn set_morse_prior_idle_time(&self, ms: u16) {
        self.keymap.set_morse_prior_idle_time(Duration::from_millis(ms as u64));
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::PriorIdleTime(ms)).await;
    }

    // Whole-struct write for Rynk's SetBehaviorConfig: one flash message instead
    // of six read-modify-write cycles of the same storage item.
    #[cfg(feature = "rynk")]
    pub async fn set_behavior_config(&self, cfg: BehaviorConfig) {
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
        FLASH_CHANNEL
            .send(FlashOperationMessage::BehaviorConfig(crate::storage::BehaviorConfig {
                prior_idle_time: cfg.morse_prior_idle_time_ms,
                morse_default_profile: cfg.morse_default_profile,
                combo_timeout: cfg.combo_timeout_ms,
                one_shot_timeout: cfg.oneshot_timeout_ms,
                tap_interval: cfg.tap_interval_ms,
                tap_capslock_interval: cfg.tap_capslock_interval_ms,
            }))
            .await;
    }

    pub async fn set_layout_options(&self, opts: u32) {
        self.keymap.set_layout_option(opts);
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::LayoutOptions(opts)).await;
    }

    pub fn layout_options(&self) -> u32 {
        self.keymap.layout_option()
    }

    pub async fn reset_storage(&self) {
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::Reset).await;
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

    pub async fn set_default_layer(&self, layer: u8) {
        self.keymap.set_default_layer(layer);
        #[cfg(feature = "storage")]
        FLASH_CHANNEL.send(FlashOperationMessage::DefaultLayer(layer)).await;
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
    /// Returns `false` when `idx` is out of range (no slot written).
    pub async fn set_fork(&self, idx: u8, fork: Fork) -> bool {
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
            FLASH_CHANNEL.send(FlashOperationMessage::Fork { idx, fork }).await;
        }
        valid
    }

    #[cfg(feature = "host_lock")]
    pub fn read_matrix_state(&self, target: &mut [u8]) {
        self.keymap.read_matrix_state(target);
    }
}
