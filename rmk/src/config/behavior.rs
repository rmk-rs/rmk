use embassy_time::Duration;
use heapless::Vec;
use rmk_types::action::{Action, KeyAction};
use rmk_types::fork::Fork;
use rmk_types::keycode::{HidKeyCode, KeyCode};
use rmk_types::modifier::ModifierCombination;
use rmk_types::morse::{Morse, MorseMode, MorseProfile};

use crate::keyboard::combo::Combo;
use crate::{
    AUTO_MOUSE_LAYER_MAX_NUM, COMBO_MAX_NUM, FORK_MAX_NUM, MACRO_SPACE_SIZE, MORSE_MAX_NUM, MORSE_PROFILE_MAX_NUM,
    MOUSE_KEY_INTERVAL, MOUSE_WHEEL_INTERVAL,
};

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
    pub auto_shift: AutoShiftConfig,
}

/// Auto shift: a plain key in `groups` or `keys` is processed as `TH(key, SHIFTED(key), profile)`.
/// `enabled`/`groups` change at runtime and persist.
#[derive(Clone, Copy, Debug)]
pub struct AutoShiftConfig {
    pub enabled: bool,
    /// Bit set of [`Self::ALPHA`], [`Self::NUMERIC`], [`Self::SYMBOLS`]
    pub groups: u8,
    /// Keys outside the groups that are auto-shifted too
    pub keys: &'static [HidKeyCode],
    /// Morse profile index; `u8::MAX` selects the default profile
    pub profile: u8,
}

impl Default for AutoShiftConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            groups: 0,
            keys: &[],
            profile: u8::MAX,
        }
    }
}

impl AutoShiftConfig {
    /// `A`–`Z`
    pub const ALPHA: u8 = 1;
    /// `1`–`0`
    pub const NUMERIC: u8 = 1 << 1;
    /// `- = [ ] \ ; ' ` , . /`
    pub const SYMBOLS: u8 = 1 << 2;

    pub fn contains(&self, key: HidKeyCode) -> bool {
        let in_group = match key {
            k if (HidKeyCode::A..=HidKeyCode::Z).contains(&k) => self.groups & Self::ALPHA != 0,
            k if (HidKeyCode::Kc1..=HidKeyCode::Kc0).contains(&k) => self.groups & Self::NUMERIC != 0,
            HidKeyCode::Minus
            | HidKeyCode::Equal
            | HidKeyCode::LeftBracket
            | HidKeyCode::RightBracket
            | HidKeyCode::Backslash
            | HidKeyCode::Semicolon
            | HidKeyCode::Quote
            | HidKeyCode::Grave
            | HidKeyCode::Comma
            | HidKeyCode::Dot
            | HidKeyCode::Slash => self.groups & Self::SYMBOLS != 0,
            _ => false,
        };
        in_group || self.keys.contains(&key)
    }

    /// Whether `action` is an auto-shift tap-hold
    pub fn is_tap_hold(&self, action: &KeyAction) -> bool {
        matches!(
            action,
            KeyAction::TapHold(Action::Key(KeyCode::Hid(tap)), Action::KeyWithModifier(hold, m), profile)
                if tap == hold && *m == ModifierCombination::LSHIFT && *profile == self.profile
        )
    }
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

    pub morses: Vec<Morse, MORSE_MAX_NUM>,
}

impl Default for MorsesConfig {
    fn default() -> Self {
        Self {
            enable_flow_tap: false,
            prior_idle_time: Duration::from_millis(120),
            default_profile: MorseProfile::new(Some(false), Some(MorseMode::Normal), Some(250u16), Some(250u16)),
            profiles: Vec::new(),
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
