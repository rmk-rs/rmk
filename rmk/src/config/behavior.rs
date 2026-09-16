use embassy_time::Duration;
use heapless::Vec;
use rmk_types::fork::Fork;
use rmk_types::keycode::KeyCode;
use rmk_types::morse::{Morse, MorseMode, MorseProfile};

use crate::keyboard::combo::Combo;
use crate::{
    AUTO_MOUSE_LAYER_MAX_NUM, COMBO_MAX_NUM, FORK_MAX_NUM, HOLD_TRIGGER_KEY_POSITION_MAX_NUM, MACRO_SPACE_SIZE,
    MORSE_MAX_NUM, MORSE_PROFILE_MAX_NUM, MOUSE_KEY_INTERVAL, MOUSE_WHEEL_INTERVAL,
};

/// Profile index standing for the default profile in [`HoldTriggerPositions`].
pub const HOLD_TRIGGER_DEFAULT_PROFILE: u8 = u8::MAX;

/// The key positions allowed to trigger a hold, as `(profile_idx, row, col)` triples across
/// every profile. A profile with no entry here places no positional restriction on its hold.
///
/// One flat table rather than a list per profile: the positional restriction is opt-in and
/// usually set on one or two profiles, so sizing a list for every profile would spend most of
/// the capacity on empty lists. Capacity comes from `[rmk] hold_trigger_key_position_max_num`
/// and is raised automatically to fit the entries in `keyboard.toml`.
pub type HoldTriggerPositions = Vec<(u8, u8, u8), HOLD_TRIGGER_KEY_POSITION_MAX_NUM>;

/// Config for configurable action behavior
#[derive(Debug, Default)]
pub struct BehaviorConfig {
    /// Base layer; restored from flash (LayoutConfig) on boot, set at runtime by DF/PDF
    pub default_layer: u8,
    pub tri_layer: Option<[u8; 3]>,
    pub tap: TapConfig,
    pub one_shot: OneShotConfig,
    pub one_shot_modifiers: OneShotModifiersConfig,
    pub combo: CombosConfig,
    pub fork: ForksConfig,
    pub morse: MorsesConfig,
    pub keyboard_macros: KeyboardMacrosConfig,
    pub mouse_key: MouseKeyConfig,
    pub auto_mouse_layer: Vec<AutoMouseLayerConfig, AUTO_MOUSE_LAYER_MAX_NUM>,
}

/// Config for auto mouse layer behavior
///
/// When a pointing device reports motion above [`AutoMouseLayerConfig::threshold`],
/// the configured [`AutoMouseLayerConfig::target_layer`] is activated. The layer
/// is deactivated once no motion has been reported for [`AutoMouseLayerConfig::timeout`].
///
/// `device_id` selects which pointing device this entry applies to. Multiple
/// entries can be configured; for each incoming [`crate::event::PointingEvent`]
/// the matching entry (or a fallback entry with `device_id == None`) drives the
/// layer state.
#[derive(Clone, Debug)]
pub struct AutoMouseLayerConfig {
    /// Pointing device id this entry applies to. When `None`, the entry acts as
    /// a fallback for devices not covered by any other entry.
    pub device_id: Option<u8>,
    /// Layer index to activate when pointing-device motion is detected
    pub target_layer: u8,
    /// Idle duration after the last motion before the layer is deactivated
    pub timeout: Duration,
    /// Minimum absolute X/Y axis delta to be considered as motion (must be `>= 1`)
    pub threshold: u16,
    /// When `true`, non-mouse key presses deactivate [`Self::target_layer`] immediately (mouse HID keys and [`Self::extra_mouse_keys`] excepted).
    /// Keys are classified by their resolved action; macro-emitted keycodes, `Again`/`Repeat`,
    /// and `GraveEscape` cannot be classified and never deactivate the layer.
    /// Modifier-only actions (e.g. the hold side of `MT`) deactivate unless every contained
    /// modifier is listed in [`Self::extra_mouse_keys`].
    pub deactivate_on_key: bool,
    /// Extra keycodes (e.g. modifiers) that do not trigger deactivation when [`Self::deactivate_on_key`] is set.
    pub extra_mouse_keys: &'static [KeyCode],
    /// When `true`, key presses that do NOT deactivate [`Self::target_layer`] extend the timeout deadline.
    pub reset_timeout_on_key: bool,
}

impl Default for AutoMouseLayerConfig {
    fn default() -> Self {
        Self {
            device_id: None,
            target_layer: 0,
            timeout: Duration::from_millis(500),
            threshold: 1,
            deactivate_on_key: false,
            extra_mouse_keys: &[],
            reset_timeout_on_key: false,
        }
    }
}

impl AutoMouseLayerConfig {
    pub fn new(device_id: Option<u8>, target_layer: u8, timeout: Duration, threshold: u16) -> Self {
        assert!(threshold >= 1, "AutoMouseLayerConfig::new: threshold must be >= 1");
        assert!(
            timeout >= Duration::from_millis(1),
            "AutoMouseLayerConfig::new: timeout must be at least 1ms"
        );
        Self {
            device_id,
            target_layer,
            timeout,
            threshold,
            ..Self::default()
        }
    }

    /// Enable [`Self::deactivate_on_key`] with `exceptions` as additional non-deactivating keycodes.
    pub fn with_deactivate_on_key(mut self, exceptions: &'static [KeyCode]) -> Self {
        self.deactivate_on_key = true;
        self.extra_mouse_keys = exceptions;
        self
    }

    /// Enable [`Self::reset_timeout_on_key`].
    pub fn with_reset_timeout_on_key(mut self) -> Self {
        self.reset_timeout_on_key = true;
        self
    }
}

/// Configurations for tap behavior
#[derive(Clone, Copy, Debug)]
pub struct TapConfig {
    // TODO: Use `Duration` instead?
    pub tap_interval: u16,
    pub tap_capslock_interval: u16,
}

impl Default for TapConfig {
    fn default() -> Self {
        Self {
            tap_interval: 20,
            tap_capslock_interval: 20,
        }
    }
}

/// Configuration for morse, tap dance, tap-hold and home row mods
#[derive(Clone, Debug)]
pub struct MorsesConfig {
    pub enable_flow_tap: bool,
    pub prior_idle_time: Duration, //used only when flow tap is enabled
    pub default_profile: MorseProfile,

    /// Named morse profiles (`[behavior.morse.profiles]`), indexed by
    /// `KeyAction::TapHold(_, _, idx)`. A missing index resolves to the
    /// default profile.
    pub profiles: Vec<MorseProfile, MORSE_PROFILE_MAX_NUM>,

    /// Key positions allowed to trigger the hold, keyed by profile index.
    pub hold_trigger_positions: HoldTriggerPositions,

    pub morses: Vec<Morse, MORSE_MAX_NUM>,
}

impl Default for MorsesConfig {
    fn default() -> Self {
        Self {
            enable_flow_tap: false,
            prior_idle_time: Duration::from_millis(120),
            default_profile: MorseProfile::new(Some(false), Some(MorseMode::Normal), Some(250u16), Some(250u16)),
            profiles: Vec::new(),
            hold_trigger_positions: Vec::new(),
            morses: Vec::new(),
        }
    }
}

/// Config for one shot behavior
#[derive(Clone, Copy, Debug)]
pub struct OneShotConfig {
    /// Timeout after which modifiers/layers are canceled/released
    pub timeout: Duration,
}

impl Default for OneShotConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(1),
        }
    }
}
/// Config for one-shot behavior
#[derive(Clone, Copy, Debug, Default)]
pub struct OneShotModifiersConfig {
    /// Should modifiers be active from keypress (sticky modifiers)
    pub activate_on_keypress: bool,
    /// If true, OSM releases on next key press (ZMK skq); if false, on next key release (ZMK skn)
    pub quick_release: bool,
}

/// Config for combo behavior
#[derive(Clone, Debug)]
pub struct CombosConfig {
    pub combos: [Option<Combo>; COMBO_MAX_NUM],
    pub timeout: Duration,
    /// Cooldown after any key press before a combo can start recording.
    /// `None` = no idle check (backward compatible). Equivalent to ZMK `require-prior-idle-ms`.
    pub prior_idle_time: Option<Duration>,
}

impl Default for CombosConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_millis(50),
            combos: core::array::from_fn(|_| None),
            prior_idle_time: None,
        }
    }
}

/// Config for fork behavior
#[derive(Clone, Debug)]
pub struct ForksConfig {
    pub forks: Vec<Fork, FORK_MAX_NUM>,
}

impl Default for ForksConfig {
    fn default() -> Self {
        Self { forks: Vec::new() }
    }
}

#[derive(Debug)]
pub struct KeyboardMacrosConfig {
    /// macros stored in biunary format to be compatible with Vial
    pub macro_sequences: [u8; MACRO_SPACE_SIZE],
}

impl Default for KeyboardMacrosConfig {
    fn default() -> Self {
        Self {
            macro_sequences: [0; MACRO_SPACE_SIZE],
        }
    }
}

impl KeyboardMacrosConfig {
    pub fn new(macro_sequences: [u8; MACRO_SPACE_SIZE]) -> Self {
        Self { macro_sequences }
    }
}

/// Config for mouse key behavior
#[derive(Clone, Copy, Debug)]
pub struct MouseKeyConfig {
    // Accelerated mode parameters
    /// Initial delay between pressing a movement key and first cursor movement (in milliseconds)
    pub initial_delay_ms: u16,
    /// Time between subsequent cursor movements in milliseconds
    pub repeat_interval_ms: u16,
    /// Step size for each movement
    pub move_delta: u8,
    /// Maximum cursor speed at which acceleration stops
    pub max_speed: u8,
    /// Number of repeat ticks until maximum cursor speed is reached
    pub ticks_to_max: u8,
    /// Initial delay between pressing a wheel key and first wheel movement (in milliseconds)
    pub wheel_initial_delay_ms: u16,
    /// Time between subsequent wheel movements in milliseconds
    pub wheel_repeat_interval_ms: u16,
    /// Wheel movement step size
    pub wheel_delta: u8,
    /// Maximum wheel speed
    pub wheel_max_speed: u8,
    /// Number of repeat ticks until maximum wheel speed is reached
    pub wheel_ticks_to_max: u8,
    /// Maximum movement distance per report
    pub move_max: u8,
    /// Maximum wheel distance per report
    pub wheel_max: u8,
}

impl Default for MouseKeyConfig {
    fn default() -> Self {
        Self {
            // Optimized values for comfortable and responsive mouse movement
            initial_delay_ms: 100,                          // 100ms initial delay
            repeat_interval_ms: MOUSE_KEY_INTERVAL,         // 20ms between movements
            move_delta: 5,                                  // 5 pixels per movement (~250 px/sec)
            max_speed: 3,                                   // Max speed multiplier (250 -> 750 px/sec)
            ticks_to_max: 50,                               // 50 ticks to max speed (~1s)
            wheel_initial_delay_ms: 100,                    // 100ms initial wheel delay
            wheel_repeat_interval_ms: MOUSE_WHEEL_INTERVAL, // 80ms between wheel movements
            wheel_delta: 1,                                 // 1 wheel unit per movement
            wheel_max_speed: 2,                             // Wheel max speed multiplier
            wheel_ticks_to_max: 40,                         // 40 ticks to max wheel speed (~3.2s)
            move_max: 25,                                   // Maximum movement per report
            wheel_max: 4,                                   // Maximum wheel movement per report
        }
    }
}

impl MouseKeyConfig {
    /// Get the appropriate delay for cursor movement based on repeat count
    pub fn get_movement_delay(&self, repeat_count: u8) -> u16 {
        if repeat_count == 0 {
            self.initial_delay_ms
        } else {
            self.repeat_interval_ms
        }
    }

    /// Get the appropriate delay for wheel movement based on repeat count
    pub fn get_wheel_delay(&self, repeat_count: u8) -> u16 {
        if repeat_count == 0 {
            self.wheel_initial_delay_ms
        } else {
            self.wheel_repeat_interval_ms
        }
    }
}
