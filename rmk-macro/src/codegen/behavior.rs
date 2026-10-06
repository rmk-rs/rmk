//! Initialize behavior config boilerplate of RMK
//!
use std::collections::HashMap;

use quote::{format_ident, quote};
use rmk_config::resolved::Behavior;
use rmk_config::resolved::behavior::{
    AutoMouseLayer, Combos, Forks, MacroOperation, Macros, Morse, MorseActionPair, MorseKey,
    MorseProfile, StickyKey, StickyProfile, StickyReleaseCondition,
};

use super::action_parser::{
    ProfileNames, SetterTable, expand_profile, expand_profile_name, get_key_with_alias,
    parse_action, parse_key, parse_name_list,
};

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

fn expand_sticky_profile(p: &StickyProfile) -> proc_macro2::TokenStream {
    let wait_timeout_ms = p.wait_timeout_ms;
    let hold_timeout_ms = p.hold_timeout_ms;
    let release_on = p.release_on.iter().map(|condition| {
        let setter = format_ident!(
            "{}",
            match condition {
                StickyReleaseCondition::BeforeNextPress => "with_before_next_press",
                StickyReleaseCondition::AfterNextPress => "with_after_next_press",
                StickyReleaseCondition::AfterNextRelease => "with_after_next_release",
                StickyReleaseCondition::LayerEnter => "with_layer_enter",
                StickyReleaseCondition::LayerExit => "with_layer_exit",
            }
        );
        quote! { .#setter(true) }
    });
    let ignore_tokens = p.ignore.iter().map(|k| {
        let ident = get_key_with_alias(k.clone());
        quote! { ::rmk::types::keycode::KeyCode::Hid(::rmk::types::keycode::HidKeyCode::#ident) }
    });
    quote! {
        ::rmk::types::sticky::StickyProfile {
            wait_timeout_ms: #wait_timeout_ms,
            hold_timeout_ms: #hold_timeout_ms,
            ignore: ::rmk::heapless::Vec::from_iter([#(#ignore_tokens),*]),
            release_on: ::rmk::types::sticky::StickyReleaseConditions::new() #(#release_on)*,
        }
    }
}

/// The named profile tables in the order they are interned into the runtime
/// tables, so a name in the keymap resolves to the same index the table uses.
pub(crate) fn profile_names(behavior: &Behavior) -> ProfileNames {
    let mut morse: Vec<_> = behavior
        .morse
        .iter()
        .flat_map(|m| m.profiles.keys())
        .cloned()
        .collect();
    morse.sort();
    ProfileNames {
        morse,
        sticky: behavior
            .sticky_key
            .as_ref()
            .map(|s| s.profiles.iter().map(|(name, _)| name.clone()).collect())
            .unwrap_or_default(),
    }
}

fn expand_sticky_key(sticky: &Option<StickyKey>) -> proc_macro2::TokenStream {
    let Some(config) = sticky else {
        return quote! { ::rmk::config::StickyKeyConfig::default() };
    };

    let default_profile = expand_sticky_profile(&config.default);
    // Profile order must match `profile_index`.
    let profile_tokens = config
        .profiles
        .iter()
        .map(|(_, p)| expand_sticky_profile(p));

    quote! {
        ::rmk::config::StickyKeyConfig {
            default_profile: #default_profile,
            profiles: ::rmk::heapless::Vec::from_iter([#(#profile_tokens),*]),
        }
    }
}

fn expand_morse_action_pair(
    action_pair: &MorseActionPair,
    names: &ProfileNames,
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
    let action = parse_key(action_pair.action.to_owned(), names);
    quote! { (rmk::types::morse::MorsePattern::from_u16(#pattern), #action.to_action()) }
}

fn expand_morse_actions(
    actions: &[MorseActionPair],
    names: &ProfileNames,
) -> proc_macro2::TokenStream {
    if !actions.is_empty() {
        let action_pair_def = actions
            .iter()
            .map(|action_pair| expand_morse_action_pair(action_pair, names));
        quote! {
            actions: ::rmk::heapless::LinearMap::from_iter([#(#action_pair_def),*]),
        }
    } else {
        quote! {}
    }
}

fn expand_morse(morse: &Option<Morse>, names: &ProfileNames) -> proc_macro2::TokenStream {
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
        let morses = expand_morses(&config.morses, &profiles_ref, names);

        // Interned morse profile table, in the same sorted-name order
        // `profile_index` uses when it emits per-key indices. The pushes can't
        // overflow: the count is validated against the capacity in `behavior()`.
        let profiles_token = if names.morse.is_empty() {
            quote! {}
        } else {
            let profile_tokens = names.morse.iter().map(|name| {
                let profile = profiles_ref
                    .as_ref()
                    .and_then(|m| m.get(name))
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

        quote! {
            ::rmk::config::MorsesConfig {
                #enable_flow_tap_token
                #prior_idle_time_token
                default_profile: #default_profile,
                #profiles_token
                #morses
                ..Default::default()
            }
        }
    } else {
        quote! { ::rmk::config::MorsesConfig::default() }
    }
}

fn expand_combos(combos: &Option<Combos>, names: &ProfileNames) -> proc_macro2::TokenStream {
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
                    let actions = combo.actions.iter().map(|a| parse_key(a.to_owned(), names));
                    let output = parse_key(combo.output.to_owned(), names);
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

/// The default macro table, a `&[&[MacroOp]]` in rodata, checked by the same
/// `validate_default_macros` const assert a Rust-defined table uses.
fn expand_macros(macros: &Option<Macros>) -> proc_macro2::TokenStream {
    let Some(macros) = macros else {
        return quote! { &[] };
    };
    let op = |variant: &str, key: &str| {
        let variant = format_ident!("{variant}");
        let action = parse_action(key.trim());
        quote! { ::rmk::types::keyboard_macros::MacroOp::#variant(#action) }
    };
    let macros_def = macros.macros.iter().map(|m| {
        let ops = m.operations.iter().flat_map(|operation| match operation {
            MacroOperation::Tap { keycode } => vec![op("Tap", keycode)],
            MacroOperation::Down { keycode } => vec![op("Press", keycode)],
            MacroOperation::Up { keycode } => vec![op("Release", keycode)],
            MacroOperation::Delay { duration_ms } => {
                let millis = u16::try_from(*duration_ms).unwrap_or_else(|_| {
                    panic!("\n\u{274c} keyboard.toml: macro delay {duration_ms}ms exceeds 65535ms")
                });
                vec![quote! { ::rmk::types::keyboard_macros::MacroOp::Delay(#millis) }]
            }
            MacroOperation::Text { text } => text
                .bytes()
                .map(|c| quote! { ::rmk::types::keyboard_macros::MacroOp::Char(#c) })
                .collect(),
            MacroOperation::PauseForRelease => {
                vec![quote! { ::rmk::types::keyboard_macros::MacroOp::PauseForRelease }]
            }
        });
        quote! { &[#(#ops),*] }
    });
    quote! {
        {
            const MACROS: &[&[::rmk::types::keyboard_macros::MacroOp]] = &[#(#macros_def),*];
            const _: () = ::core::assert!(
                ::rmk::types::keyboard_macros::validate_default_macros(MACROS),
                "keyboard.toml: invalid [behavior.macro]: use at most `macro_max_num` macros that together fit `macro_space_size` bytes, each with at most one `pause_for_release`; `text` must be ASCII"
            );
            MACROS
        }
    }
}

fn expand_morses(
    morses: &[MorseKey],
    profiles: &Option<HashMap<String, MorseProfile>>,
    names: &ProfileNames,
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
                panic!("\n❌ keyboard.toml: `morse_actions` cannot be used together with `tap_actions`, `hold_actions`, `tap`, `hold`, `hold_after_tap`, or `double_tap`.");
            }

            let actions_def = expand_morse_actions(morse_actions, names);

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
                panic!("\n❌ keyboard.toml: `tap_actions` and `hold_actions` cannot be used together with `tap`, `hold`, `hold_after_tap`, or `double_tap`.");
            }

            let tap_actions_def = match &morse.tap_actions {
                Some(tap_actions) => {
                    let actions = tap_actions.iter().map(|action| {
                        let parsed_action = parse_key(action.clone(), names);
                        quote! { #parsed_action }
                    });
                    quote! { ::rmk::heapless::Vec::from_iter([#(#actions.to_action()),*]) }
                }
                None => quote! { ::rmk::heapless::Vec::new() },
            };

            let hold_actions_def = match &morse.hold_actions {
                Some(hold_actions) => {
                    let actions = hold_actions.iter().map(|action| {
                        let parsed_action = parse_key(action.clone(), names);
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
            let tap = parse_key(morse.tap.clone().unwrap_or_else(|| "No".to_string()), names);
            let hold = parse_key(morse.hold.clone().unwrap_or_else(|| "No".to_string()), names);
            let hold_after_tap = parse_key(morse.hold_after_tap.clone().unwrap_or_else(|| "No".to_string()), names);
            let double_tap = parse_key(morse.double_tap.clone().unwrap_or_else(|| "No".to_string()), names);

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
    const STATES: &SetterTable<StateBitsMacro> = &[
        ("LCtrl", |c| c.modifiers_left_ctrl = true),
        ("LShift", |c| c.modifiers_left_shift = true),
        ("LAlt", |c| c.modifiers_left_alt = true),
        ("LGui", |c| c.modifiers_left_gui = true),
        ("RCtrl", |c| c.modifiers_right_ctrl = true),
        ("RShift", |c| c.modifiers_right_shift = true),
        ("RAlt", |c| c.modifiers_right_alt = true),
        ("RGui", |c| c.modifiers_right_gui = true),
        ("NumLock", |c| c.leds_num_lock = true),
        ("CapsLock", |c| c.leds_caps_lock = true),
        ("ScrollLock", |c| c.leds_scroll_lock = true),
        ("Compose", |c| c.leds_compose = true),
        ("Kana", |c| c.leds_kana = true),
        ("MouseBtn1", |c| c.mouse_button1 = true),
        ("MouseBtn2", |c| c.mouse_button2 = true),
        ("MouseBtn3", |c| c.mouse_button3 = true),
        ("MouseBtn4", |c| c.mouse_button4 = true),
        ("MouseBtn5", |c| c.mouse_button5 = true),
        ("MouseBtn6", |c| c.mouse_button6 = true),
        ("MouseBtn7", |c| c.mouse_button7 = true),
        ("MouseBtn8", |c| c.mouse_button8 = true),
    ];

    parse_name_list(states_str, "state", "fork state", STATES, |w| w)
}

fn expand_forks(forks: &Option<Forks>, names: &ProfileNames) -> proc_macro2::TokenStream {
    let default = quote! { ::core::default::Default::default() };
    match forks {
        Some(forks) => {
            let forks_def = forks.forks.iter().map(|fork| {
                let trigger = parse_key(fork.trigger.to_owned(), names);
                let negative_output = parse_key(fork.negative_output.to_owned(), names);
                let positive_output = parse_key(fork.positive_output.to_owned(), names);
                let match_any  = fork.match_any.as_ref().map(|s| parse_state_combination(s)).unwrap_or_default();
                let match_none = fork.match_none.as_ref().map(|s| parse_state_combination(s)).unwrap_or_default();
                let kept = fork.kept_modifiers.as_ref().map(|s| parse_state_combination(s)).unwrap_or_default();
                let bindable = fork.bindable;

                if match_any.is_empty() && match_none.is_empty() {
                    panic!("\n❌ keyboard.toml: fork configuration missing match conditions!");
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
    let names = profile_names(behavior);

    let tri_layer = expand_tri_layer(&behavior.tri_layer);
    let sticky_key = expand_sticky_key(&behavior.sticky_key);
    let combos = expand_combos(&behavior.combos, &names);
    let macros = expand_macros(&behavior.macros);
    let forks = expand_forks(&behavior.forks, &names);
    let morse = expand_morse(&behavior.morse, &names);
    let auto_mouse_layer = expand_auto_mouse_layer(&behavior.auto_mouse_layer);

    quote! {
        #[allow(clippy::needless_update)]
        let mut behavior_config = ::rmk::config::BehaviorConfig {
            tri_layer: #tri_layer,
            sticky_key: #sticky_key,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_state_combination_sets_bits_for_valid_states() {
        let s = parse_state_combination("LShift|MouseBtn1 |CapsLock");
        assert!(s.modifiers_left_shift);
        assert!(s.mouse_button1);
        assert!(s.leds_caps_lock);
        assert!(!s.modifiers_right_ctrl);
    }

    #[test]
    #[should_panic(expected = "unknown state(s) [NumLok (did you mean NumLock?)]")]
    fn parse_state_combination_rejects_unknown_state() {
        let _ = parse_state_combination("NumLok | LShift");
    }

    #[test]
    #[should_panic(expected = "unknown state(s) [capslock (did you mean CapsLock?)]")]
    fn parse_state_combination_suggests_canonical_casing() {
        let _ = parse_state_combination("capslock | LShift");
    }

    #[test]
    #[should_panic(expected = "empty segment")]
    fn parse_state_combination_rejects_trailing_separator() {
        let _ = parse_state_combination("LShift|");
    }

    #[test]
    #[should_panic(expected = "empty segment")]
    fn parse_state_combination_rejects_doubled_separator() {
        let _ = parse_state_combination("CapsLock || MouseBtn1");
    }
}
