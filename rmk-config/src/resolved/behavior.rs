use std::collections::HashMap;

/// Resolved behavioral configuration.
pub struct Behavior {
    pub tri_layer: Option<[u8; 3]>,
    pub one_shot_timeout_ms: Option<u64>,
    pub one_shot_modifiers: Option<OneShot>,
    pub combos: Option<Combos>,
    pub macros: Option<Macros>,
    pub forks: Option<Forks>,
    pub morse: Option<Morse>,
    pub auto_mouse_layer: Vec<AutoMouseLayer>,
    pub auto_shift: Option<AutoShift>,
}

pub struct AutoShift {
    pub alpha: bool,
    pub numeric: bool,
    pub symbols: bool,
    /// Key names outside the groups
    pub keys: Vec<String>,
    pub profile: Option<String>,
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

        let auto_shift = match toml_behavior.auto_shift {
            Some(a) => {
                if let Some(profile) = &a.profile
                    && !morse.as_ref().is_some_and(|m| m.profiles.contains_key(profile))
                {
                    return Err(format!(
                        "behavior.auto_shift.profile `{profile}` is not found in behavior.morse.profiles"
                    ));
                }
                let names = a
                    .keys
                    .unwrap_or_else(|| ["alpha", "numeric", "symbols"].map(String::from).to_vec());
                if names.is_empty() {
                    return Err("behavior.auto_shift.keys is empty".to_string());
                }
                let (mut alpha, mut numeric, mut symbols, mut keys) = (false, false, false, Vec::new());
                for name in names {
                    match name.to_lowercase().as_str() {
                        "alpha" => alpha = true,
                        "numeric" => numeric = true,
                        "symbols" => symbols = true,
                        _ => keys.push(name),
                    }
                }
                Some(AutoShift {
                    alpha,
                    numeric,
                    symbols,
                    keys,
                    profile: a.profile,
                })
            }
            None => None,
        };

        Ok(Behavior {
            tri_layer,
            one_shot_timeout_ms,
            one_shot_modifiers,
            combos,
            macros,
            forks,
            morse,
            auto_mouse_layer,
            auto_shift,
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

        let morse = behavior_of(toml, "flow-tap").unwrap().morse.unwrap();
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

        let err = behavior_of(toml, "profile-overflow")
            .err()
            .expect("expected the profile-overflow error");
        assert!(err.contains("morse_profile_max_num"), "unexpected error: {err}");
    }

    fn behavior_of(toml: &str, tag: &str) -> Result<super::Behavior, String> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("rmk-config-{tag}-{}-{}.toml", std::process::id(), unique));
        fs::write(&path, toml).unwrap();
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        let _ = fs::remove_file(&path);
        config.behavior()
    }

    const AUTO_SHIFT_BASE: &str = r#"
[layout]
rows = 1
cols = 1
map = "(0,0)"

[keymap]
layers = 1

[[keymap.layer]]
keys = "A"

[behavior.morse.profiles.as]
hold_timeout = "175ms"
"#;

    #[test]
    fn auto_shift_defaults_to_all_groups() {
        let toml = format!("{AUTO_SHIFT_BASE}\n[behavior.auto_shift]\nprofile = \"as\"\n");
        let a = behavior_of(&toml, "auto-shift-default").unwrap().auto_shift.unwrap();
        assert!(a.alpha && a.numeric && a.symbols);
        assert!(a.keys.is_empty());
        assert_eq!(a.profile.as_deref(), Some("as"));
    }

    #[test]
    fn auto_shift_keys_split_groups_from_key_names() {
        let toml = format!("{AUTO_SHIFT_BASE}\n[behavior.auto_shift]\nkeys = [\"Alpha\", \"Tab\", \"ent\"]\n");
        let a = behavior_of(&toml, "auto-shift-keys").unwrap().auto_shift.unwrap();
        assert!(a.alpha && !a.numeric && !a.symbols);
        assert_eq!(a.keys, vec!["Tab".to_string(), "ent".to_string()]);
        assert_eq!(a.profile, None);
    }

    #[test]
    fn auto_shift_rejects_unknown_profile_and_empty_keys() {
        let toml = format!("{AUTO_SHIFT_BASE}\n[behavior.auto_shift]\nprofile = \"nope\"\n");
        let err = behavior_of(&toml, "auto-shift-profile").err().unwrap();
        assert!(err.contains("behavior.auto_shift.profile"), "unexpected error: {err}");

        let toml = format!("{AUTO_SHIFT_BASE}\n[behavior.auto_shift]\nkeys = []\n");
        let err = behavior_of(&toml, "auto-shift-empty").err().unwrap();
        assert!(err.contains("behavior.auto_shift.keys"), "unexpected error: {err}");
    }
}
