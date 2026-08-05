//! Shared key-parsing and action-expansion helpers.
//!
//! Extracted from `layout.rs` and `behavior.rs` to break the circular
//! dependency between those two modules.

use std::collections::HashMap;

use proc_macro2::{Ident, TokenStream as TokenStream2};
use quote::{format_ident, quote};
use rmk_config::resolved::KEYCODE_ALIAS;
use rmk_config::resolved::behavior::MorseProfile;
use strum::VariantNames;

struct ModifierCombinationMacro {
    right: bool,
    gui: bool,
    alt: bool,
    shift: bool,
    ctrl: bool,
}
impl ModifierCombinationMacro {
    fn new() -> Self {
        Self {
            right: false,
            gui: false,
            alt: false,
            shift: false,
            ctrl: false,
        }
    }
    fn is_empty(&self) -> bool {
        !(self.gui || self.alt || self.shift || self.ctrl)
    }
}
// Allows to use `#modifiers` in the quote
impl quote::ToTokens for ModifierCombinationMacro {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        let right = self.right;
        let gui = self.gui;
        let alt = self.alt;
        let shift = self.shift;
        let ctrl = self.ctrl;

        tokens.extend(quote! {
            ::rmk::types::modifier::ModifierCombination::new_from(#right, #gui, #alt, #shift, #ctrl)
        });
    }
}

/// Get modifier combination, in types of mod1 | mod2 | ...
fn parse_modifiers(modifiers_str: &str) -> ModifierCombinationMacro {
    let mut combination = ModifierCombinationMacro::new();
    let tokens = modifiers_str.split_terminator("|");
    tokens.for_each(|w| {
        let w = w.trim();
        let key = match KEYCODE_ALIAS.get(w.to_lowercase().as_str()) {
            Some(k) => *k,
            None => w,
        };
        match key {
            "LShift" => combination.shift = true,
            "LCtrl" => combination.ctrl = true,
            "LAlt" => combination.alt = true,
            "LGui" => combination.gui = true,
            "RShift" => {
                combination.right = true;
                combination.shift = true;
            }
            "RCtrl" => {
                combination.right = true;
                combination.ctrl = true;
            }
            "RAlt" => {
                combination.right = true;
                combination.alt = true;
            }
            "RGui" => {
                combination.right = true;
                combination.gui = true;
            }
            _ => (),
        }
    });
    combination
}

pub(crate) fn expand_profile(profile: &MorseProfile) -> proc_macro2::TokenStream {
    let mode = if let Some(enable) = profile.permissive_hold
        && enable
    {
        quote! { ::core::option::Option::Some(rmk::types::morse::MorseMode::PermissiveHold) }
    } else if let Some(enable) = profile.hold_on_other_press
        && enable
    {
        quote! { ::core::option::Option::Some(rmk::types::morse::MorseMode::HoldOnOtherPress) }
    } else if let Some(enable) = profile.normal_mode
        && enable
    {
        quote! { ::core::option::Option::Some(rmk::types::morse::MorseMode::Normal) }
    } else {
        quote! { ::core::option::Option::None }
    };

    let unilateral_tap = if let Some(enable) = profile.unilateral_tap {
        quote! { ::core::option::Option::Some(#enable) }
    } else {
        quote! { ::core::option::Option::None }
    };

    let enable_flow_tap = if let Some(enable) = profile.enable_flow_tap {
        quote! { ::core::option::Option::Some(#enable) }
    } else {
        quote! { ::core::option::Option::None }
    };

    let hold_timeout_ms = expand_timeout("hold_timeout", &profile.hold_timeout_ms, 13);
    let gap_timeout_ms = expand_timeout("gap_timeout", &profile.gap_timeout_ms, 13);
    let quick_tap_timeout_ms =
        expand_timeout("quick_tap_timeout", &profile.quick_tap_timeout_ms, 13);

    quote! {
        rmk::types::morse::MorseProfile::new(#unilateral_tap, #mode, #hold_timeout_ms, #gap_timeout_ms)
            .with_enable_flow_tap(#enable_flow_tap)
            .with_quick_tap_timeout_ms(#quick_tap_timeout_ms)
    }
}

/// Expands an optional timeout in ms to `Option<u16>` tokens, failing the build
/// when the value exceeds the packed bit-field capacity.
fn expand_timeout(field: &str, value: &Option<u64>, bits: u8) -> proc_macro2::TokenStream {
    let max_ms = (1u64 << bits) - 1;
    match value {
        Some(t) => {
            if *t > max_ms {
                panic!(
                    "\n\u{274c} keyboard.toml: behavior.morse.{} = {}ms exceeds the maximum of {}ms ({}-bit field).",
                    field, t, max_ms, bits
                );
            }
            let timeout = *t as u16;
            quote! { ::core::option::Option::Some(#timeout) }
        }
        None => quote! { ::core::option::Option::None },
    }
}

pub(crate) fn expand_profile_name(
    profile_name: &str,
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> proc_macro2::TokenStream {
    if let Some(profiles) = profiles {
        if let Some(profile) = profiles.get(profile_name) {
            let morse_profile = expand_profile(profile);
            quote! { #morse_profile }
        } else {
            panic!(
                "\n\u{274c} `{:?}` profile name is not found in behavior.morse.profiles",
                profile_name
            );
        }
    } else {
        panic!(
            "\n\u{274c} behavior.morse.profiles is missing, so `{:?}` profile name is not found",
            profile_name
        );
    }
}

/// Split `s` on commas that are *not* nested inside parentheses.
///
/// Each piece is trimmed and empty pieces are dropped. This lets an argument
/// value itself be a parenthesised sub-action that contains commas, e.g.
/// splitting `WM(P, RAlt), LShift, HRM` yields `["WM(P, RAlt)", "LShift", "HRM"]`.
fn split_top_level(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in s.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                let piece = s[start..i].trim();
                if !piece.is_empty() {
                    parts.push(piece.to_string());
                }
                start = i + 1;
            }
            _ => {}
        }
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        parts.push(last.to_string());
    }
    parts
}

/// Strip the `NAME(` prefix and the single trailing `)` of a call-form action,
/// returning the inner argument string (e.g. `WM(P, RAlt)` -> `P, RAlt`).
fn strip_call(s: &str) -> &str {
    let open = s.find('(').expect("call-form action must contain '('");
    s[open + 1..].strip_suffix(')').unwrap_or_else(|| {
        panic!("\n\u{274c} keyboard.toml: `{}` is missing a closing ')'", s);
    })
}

/// Parse a single "action expression" into an [`rmk_types::action::Action`] token stream.
///
/// These forms each map to exactly one `Action`, so they may appear both at the
/// top level (wrapped in `KeyAction::Single` by [`parse_key`]) and inside the
/// tap/hold slots of `MT`/`TH`/`LT`. Composite forms (`MT`/`TH`/`LT`/`TT`/`TD`)
/// and `Transparent` are *not* handled here — they only exist at the top level
/// and are dispatched by [`parse_key`].
pub(crate) fn parse_action(key: &str) -> TokenStream2 {
    let lower = key.to_lowercase();

    if lower == "no" {
        return quote! { ::rmk::types::action::Action::No };
    } else if lower.starts_with("mod(") {
        let modifiers = parse_modifiers(strip_call(key));
        if modifiers.is_empty() {
            panic!(
                "\n\u{274c} keyboard.toml: modifier in MOD(modifier) is not valid! Please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        return quote! { ::rmk::types::action::Action::Modifier(#modifiers) };
    } else if lower.starts_with("wm(") {
        let keys = split_top_level(strip_call(key));
        if keys.len() != 2 {
            panic!(
                "\n\u{274c} keyboard.toml: WM(key, modifier) invalid, please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        let ident = get_key_with_alias(keys[0].clone());
        let modifiers = parse_modifiers(&keys[1]);
        if modifiers.is_empty() {
            panic!(
                "\n\u{274c} keyboard.toml: modifier in WM(key, modifier) is not valid! Please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        return quote! {
            ::rmk::types::action::Action::KeyWithModifier(
                ::rmk::types::keycode::HidKeyCode::#ident,
                #modifiers,
            )
        };
    } else if lower.starts_with("osm(") {
        let modifiers = parse_modifiers(strip_call(key));
        if modifiers.is_empty() {
            panic!(
                "\n\u{274c} keyboard.toml: modifier in OSM(modifier) is not valid! Please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        return quote! { ::rmk::types::action::Action::OneShotModifier(#modifiers) };
    } else if lower.starts_with("lm(") {
        let keys = split_top_level(strip_call(key));
        if keys.len() != 2 {
            panic!(
                "\n\u{274c} keyboard.toml: LM(layer, modifier) invalid, please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        let layer = keys[0].parse::<u8>().unwrap();
        let modifiers = parse_modifiers(&keys[1]);
        if modifiers.is_empty() {
            panic!(
                "\n\u{274c} keyboard.toml: modifier in LM(layer, modifier) is not valid! Please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        return quote! { ::rmk::types::action::Action::LayerOnWithModifier(#layer, #modifiers) };
    } else if lower.starts_with("mo(") {
        let layer = parse_layer(key);
        return quote! { ::rmk::types::action::Action::LayerOn(#layer) };
    } else if lower.starts_with("osl(") {
        let layer = parse_layer(key);
        return quote! { ::rmk::types::action::Action::OneShotLayer(#layer) };
    } else if lower.starts_with("tg(") {
        let layer = parse_layer(key);
        return quote! { ::rmk::types::action::Action::LayerToggle(#layer) };
    } else if lower.starts_with("to(") {
        let layer = parse_layer(key);
        return quote! { ::rmk::types::action::Action::LayerToggleOnly(#layer) };
    } else if lower.starts_with("pdf(") {
        let layer = parse_layer(key);
        return quote! { ::rmk::types::action::Action::PersistentDefaultLayer(#layer) };
    } else if lower.starts_with("df(") {
        let layer = parse_layer(key);
        return quote! { ::rmk::types::action::Action::DefaultLayer(#layer) };
    } else if lower.starts_with("macro(") {
        let index = strip_call(key).trim().parse::<u8>().unwrap();
        return quote! { ::rmk::types::action::Action::TriggerMacro(#index) };
    } else if lower.starts_with("shifted(") {
        let internal = strip_call(key);
        if internal.is_empty() {
            panic!(
                "\n\u{274c} keyboard.toml: SHIFTED(key) invalid, please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        let ident = get_key_with_alias(internal.to_string());
        return quote! {
            ::rmk::types::action::Action::KeyWithModifier(
                ::rmk::types::keycode::HidKeyCode::#ident,
                ::rmk::types::modifier::ModifierCombination::new_from(false, false, false, true, false),
            )
        };
    } else if lower.starts_with("stn(") {
        let key_ident = format_ident!("{}", strip_call(key).trim().to_uppercase());
        return quote! { ::rmk::types::action::Action::Steno(::rmk::types::steno::StenoKey::#key_ident) };
    } else if lower.starts_with("user") {
        // Support both User(X) and UserX formats
        let number_str = if lower.starts_with("user(") {
            key.trim_start_matches(|c: char| !c.is_ascii_digit())
                .trim_end_matches(')')
        } else if key[4..]
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            &key[4..]
        } else {
            ""
        };
        let number = number_str.parse::<u8>().unwrap_or(255);
        if number > 31 {
            panic!(
                "\n\u{274c} keyboard.toml: {} is not a valid user key! User keys are numbered 0-31. Please check the documentation: https://rmk.rs/docs/features/configuration/layout.html",
                key
            );
        }
        return quote! { ::rmk::types::action::Action::User(#number) };
    } else if lower.starts_with("macro")
        && key[5..]
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
    {
        // Support Macro0, Macro1, Macro2, etc.
        let index = key[5..].parse::<u8>().unwrap();
        return quote! { ::rmk::types::action::Action::TriggerMacro(#index) };
    }

    // Check if it's a keyboard control, light control, or special key action (case-insensitive).
    // Use strum::VariantNames to automatically get all enum variants.
    if let Some(action) = rmk_types::action::KeyboardAction::VARIANTS
        .iter()
        .find(|&&a| a.to_lowercase() == lower)
    {
        let action_ident = format_ident!("{}", action);
        return quote! {
            ::rmk::types::action::Action::KeyboardControl(::rmk::types::action::KeyboardAction::#action_ident)
        };
    }
    if let Some(action) = rmk_types::action::LightAction::VARIANTS
        .iter()
        .find(|&&a| a.to_lowercase() == lower)
    {
        let action_ident = format_ident!("{}", action);
        return quote! {
            ::rmk::types::action::Action::Light(::rmk::types::action::LightAction::#action_ident)
        };
    }
    if let Some(special_key) = rmk_types::keycode::SpecialKey::VARIANTS
        .iter()
        .find(|&&k| k.to_lowercase() == lower)
    {
        let key_ident = format_ident!("{}", special_key);
        return quote! {
            ::rmk::types::action::Action::Special(::rmk::types::keycode::SpecialKey::#key_ident)
        };
    }

    // Default: try to use as HID keycode
    let ident = get_key_with_alias(key.to_string());
    quote! {
        ::rmk::types::action::Action::Key(::rmk::types::keycode::KeyCode::Hid(::rmk::types::keycode::HidKeyCode::#ident))
    }
}

/// Parse the key string at a single position into a [`KeyAction`] token stream.
///
/// Composite tap/hold/morse forms (`MT`/`TH`/`LT`/`TT`/`TD`) and the
/// `Transparent`/`No` variants are handled here; every other form is a single
/// [`Action`] parsed by [`parse_action`] and wrapped in `KeyAction::Single`.
/// The tap/hold slots of `MT`/`TH`/`LT` accept any single-action form, so e.g.
/// `MT(WM(P, RAlt), LShift, HRM)` is valid.
pub(crate) fn parse_key(
    key: String,
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> TokenStream2 {
    if !key.is_empty() && (key.trim_start_matches("_").is_empty() || key.to_lowercase() == "trns") {
        return quote! { ::rmk::a!(Transparent) };
    } else if !key.is_empty() && key == "No" {
        return quote! { ::rmk::a!(No) };
    }

    let lower = key.to_lowercase();

    if lower.starts_with("mt(") {
        let keys = split_top_level(strip_call(&key));
        if keys.len() < 2 || keys.len() > 3 {
            panic!(
                "\n\u{274c} keyboard.toml: MT(key, modifier) invalid, please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        let tap = parse_action(&keys[0]);
        let modifiers = parse_modifiers(&keys[1]);
        if modifiers.is_empty() {
            panic!(
                "\n\u{274c} keyboard.toml: modifier in MT(key, modifier) is not valid! Please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        let profile = morse_profile(keys.get(2), profiles);
        quote! {
            ::rmk::types::action::KeyAction::TapHold(#tap, ::rmk::types::action::Action::Modifier(#modifiers), #profile)
        }
    } else if lower.starts_with("th(") {
        let keys = split_top_level(strip_call(&key));
        if keys.len() < 2 || keys.len() > 3 {
            panic!(
                "\n\u{274c} keyboard.toml: TH(key_tap, key_hold) invalid, please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        let tap = parse_action(&keys[0]);
        let hold = parse_action(&keys[1]);
        let profile = morse_profile(keys.get(2), profiles);
        quote! { ::rmk::types::action::KeyAction::TapHold(#tap, #hold, #profile) }
    } else if lower.starts_with("lt(") {
        let keys = split_top_level(strip_call(&key));
        if keys.len() < 2 || keys.len() > 3 {
            panic!(
                "\n\u{274c} keyboard.toml: LT(layer, key) invalid, please check the documentation: https://rmk.rs/docs/features/configuration/layout.html"
            );
        }
        let layer = keys[0].parse::<u8>().unwrap();
        let tap = parse_action(&keys[1]);
        let profile = morse_profile(keys.get(2), profiles);
        quote! {
            ::rmk::types::action::KeyAction::TapHold(#tap, ::rmk::types::action::Action::LayerOn(#layer), #profile)
        }
    } else if lower.starts_with("tt(") {
        let layer = parse_layer(&key);
        quote! { ::rmk::tt!(#layer) }
    } else if lower.starts_with("td(") || lower.starts_with("morse(") {
        let index = strip_call(&key).trim().parse::<u8>().unwrap();
        quote! { ::rmk::types::action::KeyAction::Morse(#index) }
    } else {
        let action = parse_action(&key);
        quote! { ::rmk::types::action::KeyAction::Single(#action) }
    }
}

/// Named profiles sorted by name, giving each a stable index into the runtime
/// morse profile table: a name at sorted position `i` is table index `i`.
pub(crate) fn sorted_profile_names(
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> Vec<String> {
    match profiles {
        Some(p) => {
            let mut names: Vec<String> = p.keys().cloned().collect();
            names.sort();
            names
        }
        None => Vec::new(),
    }
}

/// Expand the optional trailing profile argument of a tap-hold action into its
/// morse profile table index. When omitted, emit `u8::MAX`: an index with no
/// table entry falls back to the default profile at runtime (the table
/// capacity is validated to be ≤ 255, so `u8::MAX` is always vacant).
fn morse_profile(
    profile_name: Option<&String>,
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> TokenStream2 {
    let Some(name) = profile_name else {
        return quote! { ::core::primitive::u8::MAX };
    };
    let idx = match sorted_profile_names(profiles)
        .iter()
        .position(|n| n == name)
    {
        Some(pos) => pos as u8,
        None => panic!(
            "\n\u{274c} `{:?}` profile name is not found in behavior.morse.profiles",
            name
        ),
    };
    quote! { #idx }
}

/// Parse the single layer-index argument of a call-form layer action, e.g. `MO(1)`.
fn parse_layer(key: &str) -> u8 {
    strip_call(key).trim().parse::<u8>().unwrap()
}

pub(crate) fn get_key_with_alias(key: String) -> Ident {
    format_ident!("{}", resolve_alias(&key))
}

/// The `HidKeyCode` variant a key string names, or `None` when it names a richer
/// action such as `WM(A, LCtrl)` or `PDF(1)`.
///
/// Callers that can only carry an 8-bit keycode — a macro's compact
/// `Tap`/`Press`/`Release` operations — use this to tell the two apart;
/// [`parse_action`] handles both but yields the wider `Action`.
pub(crate) fn as_hid_keycode(key: &str) -> Option<Ident> {
    let key = resolve_alias(key);
    rmk_types::keycode::HidKeyCode::VARIANTS
        .contains(&key)
        .then(|| format_ident!("{key}"))
}

fn resolve_alias(key: &str) -> &str {
    match KEYCODE_ALIAS.get(key.to_lowercase().as_str()) {
        Some(resolved) => resolved,
        None => key,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmk_config::resolved::behavior::MorseProfile;

    fn expand(key: &str) -> String {
        parse_key(key.to_string(), &None).to_string()
    }

    fn profile(enable_flow_tap: Option<bool>) -> MorseProfile {
        MorseProfile {
            enable_flow_tap,
            unilateral_tap: Some(true),
            permissive_hold: None,
            hold_on_other_press: None,
            normal_mode: Some(true),
            hold_timeout_ms: Some(250),
            gap_timeout_ms: Some(250),
            quick_tap_timeout_ms: None,
            hold_trigger_key_positions: Vec::new(),
        }
    }

    // Normalize away the whitespace that `TokenStream::to_string` inserts so
    // assertions can match the structure without being brittle about spacing.
    fn squash(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    #[test]
    fn expand_profile_emits_flow_tap_override() {
        let disabled = expand_profile(&profile(Some(false))).to_string();
        assert!(disabled.contains("with_enable_flow_tap"));
        assert!(disabled.contains("Option :: Some (false)"));

        let enabled = expand_profile(&profile(Some(true))).to_string();
        assert!(enabled.contains("with_enable_flow_tap"));
        assert!(enabled.contains("Option :: Some (true)"));

        let inherit = expand_profile(&profile(None)).to_string();
        assert!(inherit.contains("with_enable_flow_tap"));
        assert!(inherit.contains("Option :: None"));
    }

    #[test]
    fn expand_profile_emits_quick_tap_timeout() {
        let explicit = expand_profile(&MorseProfile {
            quick_tap_timeout_ms: Some(200),
            ..profile(None)
        })
        .to_string();
        assert!(explicit.contains("with_quick_tap_timeout_ms"));
        assert!(explicit.contains("Option :: Some (200u16)"));

        let inherit = expand_profile(&profile(None)).to_string();
        assert!(inherit.contains("with_quick_tap_timeout_ms"));
        assert!(inherit.contains("Option :: None"));
    }

    #[test]
    fn expand_profile_accepts_max_timeouts() {
        let out = expand_profile(&MorseProfile {
            hold_timeout_ms: Some(8191),
            gap_timeout_ms: Some(8191),
            quick_tap_timeout_ms: Some(8191),
            ..profile(None)
        })
        .to_string();
        assert!(out.contains("8191u16"));
    }

    #[test]
    #[should_panic(expected = "behavior.morse.hold_timeout = 8192ms exceeds the maximum of 8191ms")]
    fn expand_profile_rejects_hold_timeout_over_max() {
        let _ = expand_profile(&MorseProfile {
            hold_timeout_ms: Some(8192),
            ..profile(None)
        });
    }

    #[test]
    #[should_panic(expected = "behavior.morse.gap_timeout = 8192ms exceeds the maximum of 8191ms")]
    fn expand_profile_rejects_gap_timeout_over_max() {
        let _ = expand_profile(&MorseProfile {
            gap_timeout_ms: Some(8192),
            ..profile(None)
        });
    }

    #[test]
    #[should_panic(
        expected = "behavior.morse.quick_tap_timeout = 8192ms exceeds the maximum of 8191ms"
    )]
    fn expand_profile_rejects_quick_tap_timeout_over_max() {
        let _ = expand_profile(&MorseProfile {
            quick_tap_timeout_ms: Some(8192),
            ..profile(None)
        });
    }

    #[test]
    fn plain_and_call_forms_wrap_in_single() {
        // Plain keycode.
        assert!(
            squash(&expand("A")).contains("KeyAction::Single(::rmk::types::action::Action::Key")
        );
        // Call-form single actions route through the shared parser, still wrapped in Single.
        assert!(
            squash(&expand("MO(1)"))
                .contains("KeyAction::Single(::rmk::types::action::Action::LayerOn(1u8))")
        );
        assert!(squash(&expand("WM(C,LCtrl)")).contains("Action::KeyWithModifier"));
        assert!(squash(&expand("MOD(LCtrl | LAlt | LGui)")).contains("Action::Modifier"));
        assert!(squash(&expand("OSM(LShift)")).contains("Action::OneShotModifier"));
    }

    #[test]
    fn mt_accepts_nested_with_modifier_tap() {
        let out = squash(&expand("MT(WM(P, RAlt), LShift)"));
        // Tap slot is a KeyWithModifier, hold slot is a Modifier combination.
        assert!(out.contains("KeyAction::TapHold(::rmk::types::action::Action::KeyWithModifier"));
        assert!(out.contains("::rmk::types::action::Action::Modifier("));
        // The nested key resolves to P with the right-Alt modifier.
        assert!(out.contains("HidKeyCode::P"));
    }

    #[test]
    fn th_accepts_nested_actions_in_both_slots() {
        let out = squash(&expand("TH(WM(A, LShift), MO(2))"));
        assert!(out.contains("Action::KeyWithModifier"));
        assert!(out.contains("Action::LayerOn(2u8)"));
    }

    #[test]
    fn lt_tap_slot_accepts_nested_action() {
        let out = squash(&expand("LT(1, WM(Q, LGui))"));
        assert!(out.contains("KeyAction::TapHold(::rmk::types::action::Action::KeyWithModifier"));
        assert!(out.contains("Action::LayerOn(1u8)"));
    }

    #[test]
    fn plain_mt_th_lt_still_expand() {
        assert!(
            squash(&expand("MT(A, LShift)")).contains("TapHold(::rmk::types::action::Action::Key")
        );
        assert!(
            squash(&expand("TH(Space, Backspace)"))
                .contains("TapHold(::rmk::types::action::Action::Key")
        );
        assert!(squash(&expand("LT(2, Enter)")).contains("Action::LayerOn(2u8)"));
    }
}
