//! Initialize behavior config boilerplate of RMK
//!
use std::collections::HashMap;

use quote::{format_ident, quote};
use rmk_config::resolved::Behavior;
use rmk_config::resolved::behavior::{
    AutoMouseLayer, Combos, Forks, MacroOperation, Macros, Morse, MorseActionPair, MorseKey,
    MorseProfile, OneShot,
};

use super::action_parser::{
    as_hid_keycode, expand_profile, expand_profile_name, get_key_with_alias, parse_action,
    parse_key, sorted_profile_names,
};
use super::feature::{get_rmk_features, is_feature_enabled};

fn expand_tri_layer(tri_layer: &Option<[u8; 3]>) -> proc_macro2::TokenStream {
    match tri_layer {
        Some(tri_layer) => {
            let upper = tri_layer[0];
            let lower = tri_layer[1];
            let adjust = tri_layer[2];
            quote! {::core::option::Option::Some([#upper, #lower, #adjust])}
        }
        None => quote! {::core::option::Option::None::<[u8; 3]>},
    }
}

fn expand_one_shot(one_shot_timeout_ms: &Option<u64>) -> proc_macro2::TokenStream {
    let default = quote! {::rmk::config::OneShotConfig::default()};
    match one_shot_timeout_ms {
        Some(millis) => {
            let timeout = quote! {::embassy_time::Duration::from_millis(#millis)};

            quote! {
                ::rmk::config::OneShotConfig {
                    timeout: #timeout,
                }
            }
        }
        None => default,
    }
}

fn expand_one_shot_modifiers(one_shot_modifiers: &Option<OneShot>) -> proc_macro2::TokenStream {
    let default = quote! { ::core::default::Default::default() };

    match one_shot_modifiers {
        Some(one_shot_modifier) => {
            let activate_on_keypress = match one_shot_modifier.activate_on_keypress {
                Some(value) => quote! { activate_on_keypress: #value, },
                None => quote! {},
            };
            let quick_release = match one_shot_modifier.quick_release {
                Some(value) => quote! { quick_release: #value, },
                None => quote! {},
            };

            quote! {
                ::rmk::config::OneShotModifiersConfig {
                    #activate_on_keypress
                    #quick_release
                    ..Default::default()
                }
            }
        }
        None => default,
    }
}

fn expand_morse_action_pair(
    action_pair: &MorseActionPair,
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> proc_macro2::TokenStream {
    let mut pattern = 0b1u16;
    for ch in action_pair.pattern.chars() {
        match ch {
            '1' => pattern = pattern << 1 | 1,
            '-' => pattern = pattern << 1 | 1,
            '_' => pattern = pattern << 1 | 1,
            '0' => pattern <<= 1,
            '.' => pattern <<= 1,
            _ => {}
        }
    }
    let action = parse_key(action_pair.action.to_owned(), profiles);
    quote! { (rmk::types::morse::MorsePattern::from_u16(#pattern), #action.to_action()) }
}

fn expand_morse_actions(
    actions: &[MorseActionPair],
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> proc_macro2::TokenStream {
    if !actions.is_empty() {
        let action_pair_def = actions
            .iter()
            .map(|action_pair| expand_morse_action_pair(action_pair, profiles));
        quote! {
            actions: ::rmk::heapless::LinearMap::from_iter([#(#action_pair_def),*]),
        }
    } else {
        quote! {}
    }
}

fn expand_morse(morse: &Option<Morse>) -> proc_macro2::TokenStream {
    if let Some(config) = morse {
        let enable_flow_tap = config.enable_flow_tap;
        let enable_flow_tap_token = quote! { enable_flow_tap: #enable_flow_tap, };

        let prior_idle_time_ms = config.prior_idle_time_ms;
        let prior_idle_time_token =
            quote! { prior_idle_time: ::embassy_time::Duration::from_millis(#prior_idle_time_ms), };

        let default_profile = expand_profile(&config.default_profile);

        let profiles_ref = if config.profiles.is_empty() {
            None
        } else {
            Some(config.profiles.clone())
        };
        let morses = expand_morses(&config.morses, &profiles_ref);

        // Interned morse profile table, in the same sorted-name order used by
        // `morse_profile` when it emits per-key indices. The pushes can't overflow:
        // the profile count is validated against the capacity in `behavior()`.
        let profile_names = sorted_profile_names(&profiles_ref);
        let profiles_token = if profile_names.is_empty() {
            quote! {}
        } else {
            let profile_tokens = profile_names.into_iter().map(|name| {
                let profile = profiles_ref
                    .as_ref()
                    .and_then(|m| m.get(&name))
                    .expect("name from same map");
                expand_profile(profile)
            });
            quote! {
                profiles: {
                    let mut v = ::rmk::heapless::Vec::new();
                    #( let _ = v.push(#profile_tokens); )*
                    v
                },
            }
        };

        // Hold trigger positions, keyed by the same profile index the profile table above is
        // interned under, with `HOLD_TRIGGER_DEFAULT_PROFILE` for the default profile. The
        // pushes can't overflow: the capacity is raised to fit every configured entry in
        // `auto_calculate_parameters`.
        let mut position_entries: Vec<proc_macro2::TokenStream> = config
            .default_profile
            .hold_trigger_key_positions
            .iter()
            .map(|p| {
                let (row, col) = (p[0], p[1]);
                quote! { (::rmk::config::HOLD_TRIGGER_DEFAULT_PROFILE, #row, #col) }
            })
            .collect();
        for (idx, name) in sorted_profile_names(&profiles_ref).into_iter().enumerate() {
            let idx = idx as u8;
            let profile = profiles_ref
                .as_ref()
                .and_then(|m| m.get(&name))
                .expect("name from same map");
            position_entries.extend(profile.hold_trigger_key_positions.iter().map(|p| {
                let (row, col) = (p[0], p[1]);
                quote! { (#idx, #row, #col) }
            }));
        }

        quote! {
            ::rmk::config::MorsesConfig {
                #enable_flow_tap_token
                #prior_idle_time_token
                default_profile: #default_profile,
                hold_trigger_positions: {
                    let mut v = ::rmk::heapless::Vec::new();
                    #( let _ = v.push(#position_entries); )*
                    v
                },
                #profiles_token
                #morses
                ..Default::default()
            }
        }
    } else {
        quote! { ::rmk::config::MorsesConfig::default() }
    }
}

fn expand_combos(
    combos: &Option<Combos>,
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> proc_macro2::TokenStream {
    let default = quote! { ::core::default::Default::default() };
    match combos {
        Some(combos) => {
            let timeout = match &combos.timeout_ms {
                Some(millis) => {
                    quote! { timeout: ::embassy_time::Duration::from_millis(#millis), }
                }
                None => quote! {},
            };

            let prior_idle_time = match &combos.prior_idle_time_ms {
                Some(millis) => {
                    quote! { prior_idle_time: ::core::option::Option::Some(::embassy_time::Duration::from_millis(#millis)), }
                }
                None => quote! {},
            };

            // When no combos are defined the `let v = [#(#combos_def),*]` expression
            // collapses to `let v = []`, which Rust can't type-infer. Emit an
            // all-`None` array directly in that case.
            let combos_field = if combos.combos.is_empty() {
                quote! {
                    combos: core::array::from_fn(|_| ::core::option::Option::None),
                }
            } else {
                let combos_def = combos.combos.iter().map(|combo| {
                    let actions = combo.actions.iter().map(|a| parse_key(a.to_owned(), profiles));
                    let output = parse_key(combo.output.to_owned(), profiles);
                    let layer = match combo.layer {
                        Some(layer) => quote! { ::core::option::Option::Some(#layer) },
                        None => quote! { ::core::option::Option::None },
                    };
                    quote! { ::rmk::keyboard::combo::Combo::new(::rmk::keyboard::combo::ComboConfig::new([#(#actions),*], #output, #layer)) }
                });
                quote! {
                    combos: {
                        let v = [#(#combos_def),*];
                        core::array::from_fn(|i| {
                            if i < v.len() {
                                Some(v[i].clone())
                            } else {
                                None
                            }
                        })
                    },
                }
            };

            quote! {
                ::rmk::config::CombosConfig {
                    #combos_field
                    #timeout
                    #prior_idle_time
                    ..Default::default()
                }
            }
        }
        None => default,
    }
}

/// One key-carrying macro operation. A plain keycode takes the compact 3-byte
/// encoding; a richer action (`WM(A, LCtrl)`, `PDF(1)`, `MACRO(0)`) takes Vial's
/// 4-byte extended form, which is decoded through the Vial keycode table and so
/// needs the `vial` feature.
fn expand_macro_key(key: &str, compact: &str, extended: &str) -> proc_macro2::TokenStream {
    let key = key.trim();
    if let Some(keycode) = as_hid_keycode(key) {
        let variant = format_ident!("{compact}");
        return quote! {
            ::rmk::keyboard_macros::MacroOperation::#variant(::rmk::types::keycode::HidKeyCode::#keycode).into_iter()
        };
    }
    // No feature list means no `rmk` dependency to read — rmk's own test crate,
    // where the scenario's `features` gate decides instead. Only reject when the
    // manifest positively says `vial` is off.
    if let Some(features) = get_rmk_features()
        && !is_feature_enabled(&Some(features), "vial")
    {
        panic!(
            "\n\u{274c} keyboard.toml: macro operation `{key}` is not a plain keycode, so it needs the extended encoding — enable rmk's `vial` feature or use a plain keycode"
        );
    }
    let action = parse_action(key);
    let variant = format_ident!("{extended}");
    quote! { ::rmk::keyboard_macros::MacroOperation::#variant(#action).into_iter() }
}

fn expand_macros(macros: &Option<Macros>) -> proc_macro2::TokenStream {
    let default = quote! { ::core::default::Default::default() };

    match macros {
        Some(macros) => {
            let macros_def = macros.macros.iter().map(|m| {
                if m.operations.is_empty() {
                    // `[].into_iter().flatten().collect()` cannot infer the element type.
                    return quote! { ::rmk::heapless::Vec::new() };
                }
                let operations = m.operations.iter().map(|op| match op {
                    MacroOperation::Tap { keycode } => expand_macro_key(keycode, "Tap", "TapAction"),
                    MacroOperation::Down { keycode } => expand_macro_key(keycode, "Press", "PressAction"),
                    MacroOperation::Up { keycode } => expand_macro_key(keycode, "Release", "ReleaseAction"),
                    MacroOperation::Delay { duration_ms } => {
                        let millis = u16::try_from(*duration_ms).ok().filter(|ms| *ms <= 65024).unwrap_or_else(|| {
                            panic!("\n\u{274c} keyboard.toml: macro delay {duration_ms}ms exceeds the 65024ms the Vial macro encoding can hold")
                        });
                        quote! { ::rmk::keyboard_macros::MacroOperation::Delay(#millis).into_iter() }
                    }
                    MacroOperation::Text { text } => {
                        quote! { ::rmk::keyboard_macros::to_macro_sequence(#text).into_iter() }
                    }
                });

                quote! { [#(#operations),*].into_iter().flatten().collect() }
            });

            quote! { ::rmk::config::KeyboardMacrosConfig::new(::rmk::keyboard_macros::define_macro_sequences(&[#(#macros_def),*])) }
        }
        None => default,
    }
}

fn expand_morses(
    morses: &[MorseKey],
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> proc_macro2::TokenStream {
    if morses.is_empty() {
        return quote! {};
    }
    let morses_def = morses.iter().map(|morse| {
        let profile = if let Some(profile_name) = &morse.profile {
            let morse_profile = expand_profile_name(profile_name, profiles);
            quote! { #morse_profile }
        } else {
            quote! { rmk::types::morse::MorseProfile::const_default() }
        };

        if let Some(morse_actions) = &morse.morse_actions {
            if morse.tap.is_some() || morse.hold.is_some() || morse.hold_after_tap.is_some() || morse.double_tap.is_some() || morse.tap_actions.is_some() || morse.hold_actions.is_some() {
                panic!("\n❌ keyboard.toml: `morse_actions` cannot be used together with `tap_actions`, `hold_actions`, `tap`, `hold`, `hold_after_tap`, or `double_tap`. Please check the documentation: https://rmk.rs/docs/features/configuration/behavior.html#morse");
            }

            let actions_def = expand_morse_actions(morse_actions, profiles);

            quote! {
                ::rmk::types::morse::Morse {
                    profile: #profile,
                    #actions_def
                    ..Default::default()
                }
            }

        } else if morse.tap_actions.is_some() || morse.hold_actions.is_some() {
            // Check first
            if morse.tap.is_some() || morse.hold.is_some() || morse.hold_after_tap.is_some() || morse.double_tap.is_some() {
                panic!("\n❌ keyboard.toml: `tap_actions` and `hold_actions` cannot be used together with `tap`, `hold`, `hold_after_tap`, or `double_tap`. Please check the documentation: https://rmk.rs/docs/features/configuration/behavior.html#morse");
            }

            let tap_actions_def = match &morse.tap_actions {
                Some(tap_actions) => {
                    let actions = tap_actions.iter().map(|action| {
                        let parsed_action = parse_key(action.clone(), profiles);
                        quote! { #parsed_action }
                    });
                    quote! { ::rmk::heapless::Vec::from_iter([#(#actions.to_action()),*]) }
                }
                None => quote! { ::rmk::heapless::Vec::new() },
            };

            let hold_actions_def = match &morse.hold_actions {
                Some(hold_actions) => {
                    let actions = hold_actions.iter().map(|action| {
                        let parsed_action = parse_key(action.clone(), profiles);
                        quote! { #parsed_action }
                    });
                    quote! { ::rmk::heapless::Vec::from_iter([#(#actions.to_action()),*]) }
                }
                None => quote! { ::rmk::heapless::Vec::new() },
            };

            quote! {
                ::rmk::types::morse::Morse::new_with_actions(
                    #tap_actions_def,
                    #hold_actions_def,
                    #profile,
                )
            }
        } else {
            let tap = parse_key(morse.tap.clone().unwrap_or_else(|| "No".to_string()), profiles);
            let hold = parse_key(morse.hold.clone().unwrap_or_else(|| "No".to_string()), profiles);
            let hold_after_tap = parse_key(morse.hold_after_tap.clone().unwrap_or_else(|| "No".to_string()), profiles);
            let double_tap = parse_key(morse.double_tap.clone().unwrap_or_else(|| "No".to_string()), profiles);

            quote! {
                ::rmk::types::morse::Morse::new_from_vial(
                    #tap.to_action(),
                    #hold.to_action(),
                    #hold_after_tap.to_action(),
                    #double_tap.to_action(),
                    #profile,
                )
            }
        }
    });

    quote! { morses: ::rmk::heapless::Vec::from_iter([#(#morses_def),*]), }
}

#[derive(PartialEq, Eq, Default)]
struct StateBitsMacro {
    modifiers_left_ctrl: bool,
    modifiers_left_shift: bool,
    modifiers_left_alt: bool,
    modifiers_left_gui: bool,
    modifiers_right_ctrl: bool,
    modifiers_right_shift: bool,
    modifiers_right_alt: bool,
    modifiers_right_gui: bool,

    leds_num_lock: bool,
    leds_caps_lock: bool,
    leds_scroll_lock: bool,
    leds_compose: bool,
    leds_kana: bool,

    mouse_button1: bool,
    mouse_button2: bool,
    mouse_button3: bool,
    mouse_button4: bool,
    mouse_button5: bool,
    mouse_button6: bool,
    mouse_button7: bool,
    mouse_button8: bool,
}

impl StateBitsMacro {
    fn is_empty(&self) -> bool {
        !(self.modifiers_left_ctrl
            || self.modifiers_left_shift
            || self.modifiers_left_alt
            || self.modifiers_left_gui
            || self.modifiers_right_ctrl
            || self.modifiers_right_shift
            || self.modifiers_right_alt
            || self.modifiers_right_gui
            || self.leds_num_lock
            || self.leds_caps_lock
            || self.leds_scroll_lock
            || self.leds_compose
            || self.leds_kana
            || self.mouse_button1
            || self.mouse_button2
            || self.mouse_button3
            || self.mouse_button4
            || self.mouse_button5
            || self.mouse_button6
            || self.mouse_button7
            || self.mouse_button8)
    }
}
// Allows to use `#modifiers` in the quote
impl quote::ToTokens for StateBitsMacro {
    fn to_tokens(&self, tokens: &mut proc_macro2::TokenStream) {
        let left_ctrl = self.modifiers_left_ctrl;
        let left_shift = self.modifiers_left_shift;
        let left_alt = self.modifiers_left_alt;
        let left_gui = self.modifiers_left_gui;
        let right_ctrl = self.modifiers_right_ctrl;
        let right_shift = self.modifiers_right_shift;
        let right_alt = self.modifiers_right_alt;
        let right_gui = self.modifiers_right_gui;

        let num_lock = self.leds_num_lock;
        let caps_lock = self.leds_caps_lock;
        let scroll_lock = self.leds_scroll_lock;
        let compose = self.leds_compose;
        let kana = self.leds_kana;

        let button1 = self.mouse_button1;
        let button2 = self.mouse_button2;
        let button3 = self.mouse_button3;
        let button4 = self.mouse_button4;
        let button5 = self.mouse_button5;
        let button6 = self.mouse_button6;
        let button7 = self.mouse_button7;
        let button8 = self.mouse_button8;

        tokens.extend(quote! {
            ::rmk::types::fork::StateBits::new_from(
                ::rmk::types::modifier::ModifierCombination::new_from_vals(#left_ctrl, #left_shift, #left_alt, #left_gui, #right_ctrl, #right_shift, #right_alt, #right_gui),
                ::rmk::types::led_indicator::LedIndicator::new_from(#num_lock, #caps_lock, #scroll_lock, #compose, #kana),
                ::rmk::types::mouse_button::MouseButtons::new_from(#button1, #button2, #button3, #button4, #button5, #button6, #button7, #button8))
        });
    }
}

/// Get modifier combination, in types of mod1 | mod2 | ...
fn parse_state_combination(states_str: &str) -> StateBitsMacro {
    let mut combination = StateBitsMacro::default();
    let tokens = states_str.split_terminator("|");
    tokens.for_each(|w| {
        let w = w.trim();
        match w {
            "LCtrl" => combination.modifiers_left_ctrl = true,
            "LShift" => combination.modifiers_left_shift = true,
            "LAlt" => combination.modifiers_left_alt = true,
            "LGui" => combination.modifiers_left_gui = true,
            "RCtrl" => combination.modifiers_right_ctrl = true,
            "RShift" => combination.modifiers_right_shift = true,
            "RAlt" => combination.modifiers_right_alt = true,
            "RGui" => combination.modifiers_right_gui = true,

            "NumLock" => combination.leds_num_lock = true,
            "CapsLock" => combination.leds_caps_lock = true,
            "ScrollLock" => combination.leds_scroll_lock = true,
            "Compose" => combination.leds_compose = true,
            "Kana" => combination.leds_kana = true,

            "MouseBtn1" => combination.mouse_button1 = true,
            "MouseBtn2" => combination.mouse_button2 = true,
            "MouseBtn3" => combination.mouse_button3 = true,
            "MouseBtn4" => combination.mouse_button4 = true,
            "MouseBtn5" => combination.mouse_button5 = true,
            "MouseBtn6" => combination.mouse_button6 = true,
            "MouseBtn7" => combination.mouse_button7 = true,
            "MouseBtn8" => combination.mouse_button8 = true,
            _ => (),
        }
    });

    combination
}

fn expand_forks(
    forks: &Option<Forks>,
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> proc_macro2::TokenStream {
    let default = quote! { ::core::default::Default::default() };
    match forks {
        Some(forks) => {
            let forks_def = forks.forks.iter().map(|fork| {
                let trigger = parse_key(fork.trigger.to_owned(), profiles);
                let negative_output = parse_key(fork.negative_output.to_owned(), profiles);
                let positive_output = parse_key(fork.positive_output.to_owned(), profiles);
                let match_any  = fork.match_any.as_ref().map(|s| parse_state_combination(s)).unwrap_or_default();
                let match_none = fork.match_none.as_ref().map(|s| parse_state_combination(s)).unwrap_or_default();
                let kept = fork.kept_modifiers.as_ref().map(|s| parse_state_combination(s)).unwrap_or_default();
                let bindable = fork.bindable;

                if match_any.is_empty() && match_none.is_empty() {
                    panic!("\n❌ keyboard.toml: fork configuration missing match conditions! Please check the documentation: https://rmk.rs/docs/features/configuration/behavior.html#fork");
                }

                quote! { ::rmk::types::fork::Fork::new(#trigger, #negative_output, #positive_output, #match_any, #match_none, #kept.modifiers, #bindable) }
            });

            quote! {
                ::rmk::config::ForksConfig {
                    forks: ::rmk::heapless::Vec::from_iter([#(#forks_def),*]),
                    ..Default::default()
                }
            }
        }
        None => default,
    }
}

fn expand_auto_mouse_layer(auto_mouse_layer: &[AutoMouseLayer]) -> proc_macro2::TokenStream {
    if auto_mouse_layer.is_empty() {
        return quote! { ::core::default::Default::default() };
    }
    let entries = auto_mouse_layer.iter().map(|cfg| {
        let target_layer = cfg.target_layer;
        let timeout_ms = cfg.timeout_ms;
        let threshold = cfg.threshold;
        let device_id = match cfg.device_id {
            Some(id) => quote! { ::core::option::Option::Some(#id) },
            None => quote! { ::core::option::Option::None },
        };
        let deactivate_on_key = cfg.deactivate_on_key;
        let reset_timeout_on_key = cfg.reset_timeout_on_key;
        let exception_idents: Vec<_> = cfg
            .extra_mouse_keys
            .iter()
            .map(|k| get_key_with_alias(k.clone()))
            .collect();
        let exception_tokens = exception_idents.iter().map(|ident| {
            quote! {
                ::rmk::types::keycode::KeyCode::Hid(::rmk::types::keycode::HidKeyCode::#ident)
            }
        });
        quote! {
            ::rmk::config::AutoMouseLayerConfig {
                device_id: #device_id,
                target_layer: #target_layer,
                timeout: ::embassy_time::Duration::from_millis(#timeout_ms),
                threshold: #threshold,
                deactivate_on_key: #deactivate_on_key,
                extra_mouse_keys: &[#(#exception_tokens),*],
                reset_timeout_on_key: #reset_timeout_on_key,
            }
        }
    });
    quote! {
        ::rmk::heapless::Vec::from_iter([#(#entries),*])
    }
}

pub(crate) fn expand_behavior_config(behavior: &Behavior) -> proc_macro2::TokenStream {
    let profiles = behavior
        .morse
        .as_ref()
        .map(|m| m.profiles.clone())
        .filter(|p| !p.is_empty());

    let tri_layer = expand_tri_layer(&behavior.tri_layer);
    let one_shot = expand_one_shot(&behavior.one_shot_timeout_ms);
    let one_shot_modifiers = expand_one_shot_modifiers(&behavior.one_shot_modifiers);
    let combos = expand_combos(&behavior.combos, &profiles);
    let macros = expand_macros(&behavior.macros);
    let forks = expand_forks(&behavior.forks, &profiles);
    let morse = expand_morse(&behavior.morse);
    let auto_mouse_layer = expand_auto_mouse_layer(&behavior.auto_mouse_layer);

    quote! {
        #[allow(clippy::needless_update)]
        let mut behavior_config = ::rmk::config::BehaviorConfig {
            tri_layer: #tri_layer,
            one_shot: #one_shot,
            one_shot_modifiers: #one_shot_modifiers,
            combo: #combos,
            fork: #forks,
            morse: #morse,
            keyboard_macros: #macros,
            mouse_key: ::rmk::config::MouseKeyConfig::default(),
            tap: ::rmk::config::TapConfig::default(),
            auto_mouse_layer: #auto_mouse_layer,
            ..Default::default()
        };
    }
}
