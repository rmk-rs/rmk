use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StickyKeyReleaseMode {
    pub other_key_press: bool,
    pub other_key_release: bool,
    pub layer_enter: bool,
    pub layer_exit: bool,
    pub double_tap: bool,
}

impl StickyKeyReleaseMode {
    pub const OTHER_KEY_PRESS: Self = Self {
        other_key_press: true,
        ..Self::default_const()
    };
    pub const OTHER_KEY_RELEASE: Self = Self {
        other_key_release: true,
        ..Self::default_const()
    };
    pub const LAYER_ENTER: Self = Self {
        layer_enter: true,
        ..Self::default_const()
    };
    pub const LAYER_EXIT: Self = Self {
        layer_exit: true,
        ..Self::default_const()
    };
    pub const DOUBLE_TAP: Self = Self {
        double_tap: true,
        ..Self::default_const()
    };

    const fn default_const() -> Self {
        Self {
            other_key_press: false,
            other_key_release: false,
            layer_enter: false,
            layer_exit: false,
            double_tap: false,
        }
    }

    pub const fn into_bits(self) -> u8 {
        (self.other_key_press as u8)
            | ((self.other_key_release as u8) << 1)
            | ((self.layer_enter as u8) << 2)
            | ((self.layer_exit as u8) << 3)
            | ((self.double_tap as u8) << 4)
    }

    pub fn parse(value: &str) -> Result<Self, String> {
        let mut mode = Self::default();
        for part in value.split('|').map(str::trim).filter(|part| !part.is_empty()) {
            match part {
                "other_key_press" => mode.other_key_press = true,
                "other_key_release" => mode.other_key_release = true,
                "layer_enter" => mode.layer_enter = true,
                "layer_exit" => mode.layer_exit = true,
                "double_tap" => mode.double_tap = true,
                _ => {
                    return Err(format!(
                        "unknown Sticky Key release_mode `{part}`; expected other_key_press, other_key_release, layer_enter, layer_exit, or double_tap"
                    ));
                }
            }
        }
        if mode == Self::default() {
            return Err("Sticky Key release_mode must contain at least one trigger".to_string());
        }
        Ok(mode)
    }
}

pub struct StickyKeyConfig {
    pub timeout_ms: Option<u64>,
    pub activate_on_keypress: Option<bool>,
    pub release_after_hold_ms: Option<u64>,
    pub max_repeat: Option<u16>,
    pub release_mode: Option<StickyKeyReleaseMode>,
    pub profiles: HashMap<String, StickyKeyProfile>,
}

#[derive(Clone, Debug, Default)]
pub struct StickyKeyProfile {
    pub timeout_ms: Option<u64>,
    pub activate_on_keypress: Option<bool>,
    pub release_after_hold_ms: Option<u64>,
    pub max_repeat: Option<u16>,
    pub release_mode: Option<StickyKeyReleaseMode>,
}

/// Resolved behavioral configuration.
pub struct Behavior {
    pub tri_layer: Option<[u8; 3]>,
    pub one_shot_timeout_ms: Option<u64>,
    pub one_shot_modifiers: Option<OneShot>,
    pub combos: Option<Combos>,
    pub macros: Option<Macros>,
    pub forks: Option<Forks>,
    pub morse: Option<Morse>,
    pub sticky_key: Option<StickyKeyConfig>,
    pub auto_mouse_layer: Vec<AutoMouseLayer>,
}

pub struct AutoMouseLayer {
    pub device_id: Option<u8>,
    pub target_layer: u8,
    pub timeout_ms: u64,
    pub threshold: u16,
    pub deactivate_on_key: bool,
    pub extra_mouse_keys: Vec<String>,
    pub reset_timeout_on_key: bool,
}

/// Default idle timeout (in milliseconds) for [`AutoMouseLayer`] when not specified in `keyboard.toml`.
pub const DEFAULT_AUTO_MOUSE_LAYER_TIMEOUT_MS: u64 = 500;

/// Default motion threshold for [`AutoMouseLayer`] when not specified.
pub const DEFAULT_AUTO_MOUSE_LAYER_THRESHOLD: u16 = 1;

/// Fallback for `auto_mouse_layer_max_num` when no `keyboard.toml` is loaded.
pub const DEFAULT_AUTO_MOUSE_LAYER_MAX_NUM: usize = 2;

pub struct OneShot {
    pub activate_on_keypress: Option<bool>,
    pub quick_release: Option<bool>,
}

pub struct Combos {
    pub combos: Vec<Combo>,
    pub timeout_ms: Option<u64>,
    pub prior_idle_time_ms: Option<u64>,
}

pub struct Combo {
    pub actions: Vec<String>,
    pub output: String,
    pub layer: Option<u8>,
}

pub struct Macros {
    pub macros: Vec<Macro>,
}

pub struct Macro {
    pub operations: Vec<MacroOperation>,
}

/// Resolved macro operation — all durations are plain milliseconds.
#[derive(Clone, Debug)]
pub enum MacroOperation {
    Tap { keycode: String },
    Down { keycode: String },
    Up { keycode: String },
    Delay { duration_ms: u64 },
    Text { text: String },
}

pub struct Forks {
    pub forks: Vec<Fork>,
}

pub struct Fork {
    pub trigger: String,
    pub negative_output: String,
    pub positive_output: String,
    pub match_any: Option<String>,
    pub match_none: Option<String>,
    pub kept_modifiers: Option<String>,
    pub bindable: bool,
}

pub struct Morse {
    pub enable_flow_tap: bool,
    pub prior_idle_time_ms: u64,
    pub default_profile: MorseProfile,
    pub profiles: HashMap<String, MorseProfile>,
    pub morses: Vec<MorseKey>,
}

#[derive(Clone)]
pub struct MorseProfile {
    pub enable_flow_tap: Option<bool>,
    pub unilateral_tap: Option<bool>,
    pub permissive_hold: Option<bool>,
    pub hold_on_other_press: Option<bool>,
    pub normal_mode: Option<bool>,
    pub hold_timeout_ms: Option<u64>,
    pub gap_timeout_ms: Option<u64>,
    pub quick_tap_timeout_ms: Option<u64>,
}

pub struct MorseKey {
    pub profile: Option<String>,
    pub tap: Option<String>,
    pub hold: Option<String>,
    pub hold_after_tap: Option<String>,
    pub double_tap: Option<String>,
    pub tap_actions: Option<Vec<String>>,
    pub hold_actions: Option<Vec<String>>,
    pub morse_actions: Option<Vec<MorseActionPair>>,
}

pub struct MorseActionPair {
    pub pattern: String,
    pub action: String,
}

impl crate::KeyboardTomlConfig {
    /// Resolve behavioral configuration from TOML config.
    pub fn behavior(&self) -> Result<Behavior, String> {
        let toml_behavior = self.get_behavior_config()?;

        let tri_layer = toml_behavior.tri_layer.map(|t| [t.upper, t.lower, t.adjust]);

        let one_shot_timeout_ms = toml_behavior.one_shot.and_then(|o| o.timeout.map(|t| t.0));

        let one_shot_modifiers = toml_behavior.one_shot_modifiers.map(|o| OneShot {
            activate_on_keypress: o.activate_on_keypress,
            quick_release: o.quick_release,
        });

        let combos = toml_behavior.combo.map(|c| Combos {
            combos: c
                .combos
                .into_iter()
                .map(|combo| Combo {
                    actions: combo.actions,
                    output: combo.output,
                    layer: combo.layer,
                })
                .collect(),
            timeout_ms: c.timeout.map(|t| t.0),
            prior_idle_time_ms: c.prior_idle_time.map(|t| t.0),
        });

        let macros = toml_behavior.macros.map(|m| Macros {
            macros: m
                .macros
                .into_iter()
                .map(|mc| Macro {
                    operations: mc.operations.into_iter().map(resolve_macro_operation).collect(),
                })
                .collect(),
        });

        let forks = toml_behavior.fork.map(|f| Forks {
            forks: f
                .forks
                .into_iter()
                .map(|fork| Fork {
                    trigger: fork.trigger,
                    negative_output: fork.negative_output,
                    positive_output: fork.positive_output,
                    match_any: fork.match_any,
                    match_none: fork.match_none,
                    kept_modifiers: fork.kept_modifiers,
                    bindable: fork.bindable.unwrap_or(false),
                })
                .collect(),
        });

        let morse = toml_behavior.morse.map(|m| {
            let profiles = m
                .profiles
                .as_ref()
                .map(|p| {
                    p.iter()
                        .map(|(name, p)| (name.clone(), resolve_morse_profile(p)))
                        .collect()
                })
                .unwrap_or_default();

            // Seeded rather than left `None` so the switch is stored and read
            // back over Rynk like the profile's other fields.
            let default_profile = MorseProfile {
                enable_flow_tap: Some(m.enable_flow_tap.unwrap_or(false)),
                unilateral_tap: m.unilateral_tap,
                permissive_hold: m.permissive_hold,
                hold_on_other_press: m.hold_on_other_press,
                normal_mode: m.normal_mode,
                hold_timeout_ms: Some(m.hold_timeout.as_ref().map(|t| t.0).unwrap_or(250)),
                gap_timeout_ms: Some(m.gap_timeout.as_ref().map(|t| t.0).unwrap_or(250)),
                quick_tap_timeout_ms: m.quick_tap_timeout.as_ref().map(|t| t.0),
            };

            let morses = m
                .morses
                .unwrap_or_default()
                .into_iter()
                .map(|mk| MorseKey {
                    profile: mk.profile,
                    tap: mk.tap,
                    hold: mk.hold,
                    hold_after_tap: mk.hold_after_tap,
                    double_tap: mk.double_tap,
                    tap_actions: mk.tap_actions,
                    hold_actions: mk.hold_actions,
                    morse_actions: mk.morse_actions.map(|pairs| {
                        pairs
                            .into_iter()
                            .map(|p| MorseActionPair {
                                pattern: p.pattern,
                                action: p.action,
                            })
                            .collect()
                    }),
                })
                .collect();

            Morse {
                enable_flow_tap: m.enable_flow_tap.unwrap_or(false),
                prior_idle_time_ms: m.prior_idle_time.map(|t| t.0).unwrap_or(120),
                default_profile,
                profiles,
                morses,
            }
        });

        // Named profiles are interned into the fixed-capacity morse profile
        // table; overflowing it would silently drop profiles at runtime.
        if let Some(m) = &morse
            && m.profiles.len() > self.rmk.morse_profile_max_num
        {
            return Err(format!(
                "behavior.morse.profiles defines {} profiles, but `[rmk] morse_profile_max_num` is {}. Raise it in keyboard.toml",
                m.profiles.len(),
                self.rmk.morse_profile_max_num
            ));
        }

        let sticky_key = toml_behavior
            .sticky_key
            .map(|sticky| -> Result<StickyKeyConfig, String> {
                let parse_profile = |profile: crate::StickyKeyProfile| -> Result<StickyKeyProfile, String> {
                    Ok(StickyKeyProfile {
                        timeout_ms: profile.timeout.map(|timeout| timeout.0),
                        activate_on_keypress: profile.activate_on_keypress,
                        release_after_hold_ms: profile.release_after_hold.map(|duration| duration.0),
                        max_repeat: profile.max_repeat,
                        release_mode: profile
                            .release_mode
                            .as_deref()
                            .map(StickyKeyReleaseMode::parse)
                            .transpose()?,
                    })
                };
                let profiles = sticky
                    .profiles
                    .into_iter()
                    .map(|(name, profile)| parse_profile(profile).map(|profile| (name, profile)))
                    .collect::<Result<HashMap<_, _>, _>>()?;
                Ok(StickyKeyConfig {
                    timeout_ms: sticky.timeout.map(|timeout| timeout.0),
                    activate_on_keypress: sticky.activate_on_keypress,
                    release_after_hold_ms: sticky.release_after_hold.map(|duration| duration.0),
                    max_repeat: sticky.max_repeat,
                    release_mode: sticky
                        .release_mode
                        .as_deref()
                        .map(StickyKeyReleaseMode::parse)
                        .transpose()?,
                    profiles,
                })
            })
            .transpose()?;

        let auto_mouse_layer = toml_behavior
            .auto_mouse_layer
            .unwrap_or_default()
            .into_iter()
            .map(|a| AutoMouseLayer {
                device_id: a.device_id,
                target_layer: a.target_layer,
                timeout_ms: a.timeout.map(|t| t.0).unwrap_or(DEFAULT_AUTO_MOUSE_LAYER_TIMEOUT_MS),
                threshold: a.threshold.unwrap_or(DEFAULT_AUTO_MOUSE_LAYER_THRESHOLD),
                deactivate_on_key: a.deactivate_on_key.unwrap_or(false),
                extra_mouse_keys: a.extra_mouse_keys.unwrap_or_default(),
                reset_timeout_on_key: a.reset_timeout_on_key.unwrap_or(false),
            })
            .collect();

        Ok(Behavior {
            tri_layer,
            one_shot_timeout_ms,
            one_shot_modifiers,
            combos,
            macros,
            forks,
            morse,
            sticky_key,
            auto_mouse_layer,
        })
    }
}

fn resolve_macro_operation(op: crate::MacroOperation) -> MacroOperation {
    match op {
        crate::MacroOperation::Tap { keycode } => MacroOperation::Tap { keycode },
        crate::MacroOperation::Down { keycode } => MacroOperation::Down { keycode },
        crate::MacroOperation::Up { keycode } => MacroOperation::Up { keycode },
        crate::MacroOperation::Delay { duration } => MacroOperation::Delay {
            duration_ms: duration.0,
        },
        crate::MacroOperation::Text { text } => MacroOperation::Text { text },
    }
}

fn resolve_morse_profile(p: &crate::MorseProfile) -> MorseProfile {
    MorseProfile {
        enable_flow_tap: p.enable_flow_tap,
        unilateral_tap: p.unilateral_tap,
        permissive_hold: p.permissive_hold,
        hold_on_other_press: p.hold_on_other_press,
        normal_mode: p.normal_mode,
        hold_timeout_ms: p.hold_timeout.as_ref().map(|t| t.0),
        gap_timeout_ms: p.gap_timeout.as_ref().map(|t| t.0),
        quick_tap_timeout_ms: p.quick_tap_timeout.as_ref().map(|t| t.0),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::StickyKeyReleaseMode;
    use crate::KeyboardTomlConfig;

    #[test]
    fn morse_profile_enable_flow_tap_resolves_as_override() {
        let toml = r#"
[layout]
rows = 1
cols = 1
map = "(0,0)"

[keymap]
layers = 1

[[keymap.layer]]
keys = "A"

[behavior.morse]
enable_flow_tap = true

[behavior.morse.profiles.flow_on]
enable_flow_tap = true

[behavior.morse.profiles.flow_off]
enable_flow_tap = false

[behavior.morse.profiles.inherit]
hold_timeout = "200ms"
"#;

        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("rmk-config-flow-tap-{}-{}.toml", std::process::id(), unique));

        fs::write(&path, toml).unwrap();
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        let _ = fs::remove_file(&path);

        let behavior = config.behavior().unwrap();
        let morse = behavior.morse.unwrap();
        assert!(morse.enable_flow_tap);
        assert_eq!(morse.default_profile.enable_flow_tap, Some(true));
        assert_eq!(morse.profiles["flow_on"].enable_flow_tap, Some(true));
        assert_eq!(morse.profiles["flow_off"].enable_flow_tap, Some(false));
        assert_eq!(morse.profiles["inherit"].enable_flow_tap, None);
    }

    #[test]
    fn morse_profiles_overflowing_morse_profile_max_num_is_an_error() {
        let toml = r#"
[rmk]
morse_profile_max_num = 1

[layout]
rows = 1
cols = 1
map = "(0,0)"

[keymap]
layers = 1

[[keymap.layer]]
keys = "A"

[behavior.morse.profiles.p1]
hold_timeout = "200ms"

[behavior.morse.profiles.p2]
hold_timeout = "300ms"
"#;

        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!(
            "rmk-config-profile-overflow-{}-{}.toml",
            std::process::id(),
            unique
        ));

        fs::write(&path, toml).unwrap();
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        let _ = fs::remove_file(&path);

        let err = match config.behavior() {
            Ok(_) => panic!("expected the profile-overflow error"),
            Err(e) => e,
        };
        assert!(err.contains("morse_profile_max_num"), "unexpected error: {err}");
    }

    #[test]
    fn sticky_profiles_and_release_modes_resolve() {
        let config: KeyboardTomlConfig = toml::from_str(
            r#"
[behavior.sticky_key]
release_after_hold = "300ms"
release_mode = "other_key_release | layer_exit | double_tap"

[behavior.sticky_key.profiles.alt_tab]
timeout = "5s"
release_after_hold = "500ms"
release_mode = "other_key_press | layer_enter"
"#,
        )
        .unwrap();

        let sticky = config.behavior().unwrap().sticky_key.unwrap();
        assert_eq!(sticky.timeout_ms, None);
        assert_eq!(sticky.release_after_hold_ms, Some(300));
        assert_eq!(
            sticky.release_mode.unwrap().into_bits(),
            StickyKeyReleaseMode::OTHER_KEY_RELEASE.into_bits()
                | StickyKeyReleaseMode::LAYER_EXIT.into_bits()
                | StickyKeyReleaseMode::DOUBLE_TAP.into_bits()
        );
        let alt_tab = &sticky.profiles["alt_tab"];
        assert_eq!(alt_tab.timeout_ms, Some(5000));
        assert_eq!(alt_tab.release_after_hold_ms, Some(500));
        assert_eq!(
            alt_tab.release_mode.unwrap().into_bits(),
            StickyKeyReleaseMode::OTHER_KEY_PRESS.into_bits() | StickyKeyReleaseMode::LAYER_ENTER.into_bits()
        );
    }
}
