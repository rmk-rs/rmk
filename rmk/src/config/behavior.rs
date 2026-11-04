use embassy_time::Duration;
use heapless::Vec;
use rmk_types::fork::Fork;
use rmk_types::keyboard_macros::MacroOp;
use rmk_types::keycode::KeyCode;
use rmk_types::morse::{Morse, MorseMode, MorseProfile};

use crate::keyboard::combo::Combo;
use crate::{AUTO_MOUSE_LAYER_MAX_NUM, COMBO_MAX_NUM, FORK_MAX_NUM, MORSE_MAX_NUM, MORSE_PROFILE_MAX_NUM};

/// Config for configurable action behavior
#[derive(Debug, Default)]
pub struct BehaviorConfig {
    /// Base layer; restored from flash (`DefaultLayer`) on boot, set at runtime by DF/PDF
    pub default_layer: u8,
    pub tri_layer: Option<[u8; 3]>,
    pub tap: TapConfig,
    pub one_shot: OneShotConfig,
    pub one_shot_modifiers: OneShotModifiersConfig,
    pub combo: CombosConfig,
    pub fork: ForksConfig,
    pub morse: MorsesConfig,
    /// The default macros, `MACRO(i)` runs the `i`th. A host-written macro in
    /// flash replaces its default. Check a hand-written table with
    /// `const _: () = assert!(validate_default_macros(MACROS));`.
    pub keyboard_macros: &'static [&'static [MacroOp]],
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

/// Cursor speeds are HID counts per second. Scrolling is measured in wheel units per second.
#[derive(Clone, Copy, Debug)]
pub struct MouseKeyConfig {
    initial_speed: u16,
    max_speed: u16,
    acceleration_time_ms: u16,
    scroll_speed: u16,
}

impl Default for MouseKeyConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl MouseKeyConfig {
    pub const DEFAULT: Self = Self::new(0, 600, 200, 10);

    /// A zero acceleration time selects the maximum speed immediately.
    ///
    /// # Panics
    /// Panics if the maximum or scroll speed is zero, or the initial speed exceeds the maximum.
    pub const fn new(initial_speed: u16, max_speed: u16, acceleration_time_ms: u16, scroll_speed: u16) -> Self {
        core::assert!(max_speed > 0, "behavior.mouse_key.max_speed must be positive");
        core::assert!(
            initial_speed <= max_speed,
            "behavior.mouse_key.initial_speed must not exceed max_speed"
        );
        core::assert!(scroll_speed > 0, "behavior.mouse_key.scroll_speed must be positive");
        Self {
            initial_speed,
            max_speed,
            acceleration_time_ms,
            scroll_speed,
        }
    }

    /// Starting cursor speed in HID counts per second.
    pub const fn initial_speed(&self) -> u16 {
        self.initial_speed
    }

    /// Normal cursor speed after acceleration, in HID counts per second.
    pub const fn max_speed(&self) -> u16 {
        self.max_speed
    }

    /// Duration of the normal cursor ramp in milliseconds.
    pub const fn acceleration_time_ms(&self) -> u16 {
        self.acceleration_time_ms
    }

    /// Normal horizontal and vertical scroll speed in wheel units per second.
    pub const fn scroll_speed(&self) -> u16 {
        self.scroll_speed
    }
}
