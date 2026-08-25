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

#[derive(Default)]
struct ModifierCombinationMacro {
    right: bool,
    gui: bool,
    alt: bool,
    shift: bool,
    ctrl: bool,
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
    const MODIFIERS: &SetterTable<ModifierCombinationMacro> = &[
        ("LShift", |c| c.shift = true),
        ("LCtrl", |c| c.ctrl = true),
        ("LAlt", |c| c.alt = true),
        ("LGui", |c| c.gui = true),
        ("RShift", |c| {
            c.right = true;
            c.shift = true;
        }),
        ("RCtrl", |c| {
            c.right = true;
            c.ctrl = true;
        }),
        ("RAlt", |c| {
            c.right = true;
            c.alt = true;
        }),
        ("RGui", |c| {
            c.right = true;
            c.gui = true;
        }),
    ];

    parse_name_list(
        modifiers_str,
        "modifier",
        "modifier combination",
        MODIFIERS,
        |w| {
            KEYCODE_ALIAS
                .get(w.to_lowercase().as_str())
                .copied()
                .unwrap_or(w)
        },
    )
}

/// A name -> setter lookup table for [`parse_name_list`]: each entry maps a
/// `|`-separated token to a setter applied to the accumulated value.
pub(crate) type SetterTable<T> = [(&'static str, fn(&mut T))];

/// Resolve each `|`-separated token of `input` against a name->setter table,
/// applying every match to the returned value. Panics on unknown tokens
/// (with closest-name hints) and on empty segments between separators.
pub(crate) fn parse_name_list<T>(
    input: &str,
    item: &str,
    kind: &str,
    table: &SetterTable<T>,
    resolve_alias: impl Fn(&str) -> &str,
) -> T
where
    T: Default,
{
    let mut value = T::default();
    let mut unknown_tokens = Vec::new();
    let mut has_empty_segment = false;
    input.split("|").for_each(|w| {
        let w = w.trim();
        if w.is_empty() {
            has_empty_segment = true;
            return;
        }
        match table.iter().find(|(name, _)| *name == resolve_alias(w)) {
            Some((_, set)) => set(&mut value),
            None => unknown_tokens.push(w.to_string()),
        }
    });

    if !unknown_tokens.is_empty() {
        let unknown = unknown_tokens
            .iter()
            .map(
                |u| match closest_name(u, table.iter().map(|(name, _)| *name)) {
                    Some(s) => format!("{u} (did you mean {s}?)"),
                    None => u.clone(),
                },
            )
            .collect::<Vec<_>>()
            .join(", ");
        panic!(
            "\n❌ keyboard.toml: unknown {item}(s) [{}] in {kind} '{}'",
            unknown,
            input.trim()
        );
    }

    if has_empty_segment {
        panic!(
            "\n❌ keyboard.toml: {kind} '{}' contains empty segments between '|' separators",
            input.trim()
        );
    }

    value
}

/// Suggest the most similar valid name for an unrecognized token, or `None`
/// when nothing is close enough to be a plausible typo of it.
pub(crate) fn closest_name<'a>(
    input: &str,
    names: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let lowercased = input.to_lowercase();
    let (distance, name) = names
        .map(|name| (strsim::levenshtein(&lowercased, &name.to_lowercase()), name))
        .min_by_key(|&(distance, _)| distance)?;
    // rustc-style cutoff: allow roughly one edit per three characters
    (distance <= 1 + input.len() / 3).then_some(name)
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
            panic_unknown_profile(profile_name, profiles.keys().map(String::as_str));
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
        return quote! { ::rmk::types::action::Action::Modifier(#modifiers) };
    } else if lower.starts_with("wm(") {
        let keys = split_top_level(strip_call(key));
        if keys.len() != 2 {
            panic!("\n\u{274c} keyboard.toml: WM(key, modifier) invalid");
        }
        let ident = get_key_with_alias(keys[0].clone());
        let modifiers = parse_modifiers(&keys[1]);
        return quote! {
            ::rmk::types::action::Action::KeyWithModifier(
                ::rmk::types::keycode::HidKeyCode::#ident,
                #modifiers,
            )
        };
    } else if lower.starts_with("osm(") {
        let modifiers = parse_modifiers(strip_call(key));
        return quote! { ::rmk::types::action::Action::OneShotModifier(#modifiers) };
    } else if lower.starts_with("lm(") {
        let keys = split_top_level(strip_call(key));
        if keys.len() != 2 {
            panic!("\n\u{274c} keyboard.toml: LM(layer, modifier) invalid");
        }
        let layer = parse_numeric_arg(&keys[0], "layer");
        let modifiers = parse_modifiers(&keys[1]);
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
        let index = parse_numeric_arg(strip_call(key), "macro");
        return quote! { ::rmk::types::action::Action::TriggerMacro(#index) };
    } else if lower.starts_with("shifted(") {
        let internal = strip_call(key);
        if internal.is_empty() {
            panic!("\n\u{274c} keyboard.toml: SHIFTED(key) invalid");
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
                "\n\u{274c} keyboard.toml: {} is not a valid user key! User keys are numbered 0-31.",
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
        let index = parse_numeric_arg(&key[5..], "macro");
        return quote! { ::rmk::types::action::Action::TriggerMacro(#index) };
    }

    // Check if it's a keyboard control, light control, or special key action
    // (case-insensitive), matching against each enum's variant names.
    if let Some(action_ident) = match_variant(rmk_types::action::KeyboardAction::VARIANTS, &lower) {
        return quote! {
            ::rmk::types::action::Action::KeyboardControl(::rmk::types::action::KeyboardAction::#action_ident)
        };
    }
    if let Some(action_ident) = match_variant(rmk_types::action::LightAction::VARIANTS, &lower) {
        return quote! {
            ::rmk::types::action::Action::Light(::rmk::types::action::LightAction::#action_ident)
        };
    }
    if let Some(key_ident) = match_variant(rmk_types::keycode::SpecialKey::VARIANTS, &lower) {
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
            panic!("\n\u{274c} keyboard.toml: MT(key, modifier) invalid");
        }
        let tap = parse_action(&keys[0]);
        let modifiers = parse_modifiers(&keys[1]);
        let profile = morse_profile(keys.get(2), profiles);
        quote! {
            ::rmk::types::action::KeyAction::TapHold(#tap, ::rmk::types::action::Action::Modifier(#modifiers), #profile)
        }
    } else if lower.starts_with("th(") {
        let keys = split_top_level(strip_call(&key));
        if keys.len() < 2 || keys.len() > 3 {
            panic!("\n\u{274c} keyboard.toml: TH(key_tap, key_hold) invalid");
        }
        let tap = parse_action(&keys[0]);
        let hold = parse_action(&keys[1]);
        let profile = morse_profile(keys.get(2), profiles);
        quote! { ::rmk::types::action::KeyAction::TapHold(#tap, #hold, #profile) }
    } else if lower.starts_with("lt(") {
        let keys = split_top_level(strip_call(&key));
        if keys.len() < 2 || keys.len() > 3 {
            panic!("\n\u{274c} keyboard.toml: LT(layer, key) invalid");
        }
        let layer = parse_numeric_arg(&keys[0], "layer");
        let tap = parse_action(&keys[1]);
        let profile = morse_profile(keys.get(2), profiles);
        quote! {
            ::rmk::types::action::KeyAction::TapHold(#tap, ::rmk::types::action::Action::LayerOn(#layer), #profile)
        }
    } else if lower.starts_with("tt(") {
        let layer = parse_layer(&key);
        quote! { ::rmk::tt!(#layer) }
    } else if lower.starts_with("td(") || lower.starts_with("morse(") {
        let index = parse_numeric_arg(strip_call(&key), "morse");
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
        None => panic_unknown_profile(
            name,
            sorted_profile_names(profiles).iter().map(String::as_str),
        ),
    };
    quote! { #idx }
}

/// Panic for a failed morse-profile lookup, suggesting the nearest defined
/// name when one plausibly matches.
fn panic_unknown_profile<'a>(name: &str, known: impl Iterator<Item = &'a str>) -> ! {
    let hint = match closest_name(name, known) {
        Some(s) => format!(" (did you mean {s}?)"),
        None => String::new(),
    };
    panic!(
        "\n\u{274c} keyboard.toml: `{:?}` profile name is not found in behavior.morse.profiles{hint}",
        name
    )
}

/// Parse the single layer-index argument of a call-form layer action, e.g. `MO(1)`.
fn parse_layer(key: &str) -> u8 {
    parse_numeric_arg(strip_call(key), "layer")
}

/// Parse a numeric action argument (layer or macro/morse index), rejecting
/// malformed values with context instead of an opaque ParseIntError.
fn parse_numeric_arg(raw: &str, kind: &str) -> u8 {
    raw.trim().parse::<u8>().unwrap_or_else(|_| {
        panic!("\n\u{274c} keyboard.toml: {kind} index must be a number, got '{raw}'")
    })
}

/// Case-insensitively match a token against an enum's variant names,
/// expanding the match to that variant's identifier.
fn match_variant(names: &[&str], lower: &str) -> Option<Ident> {
    names
        .iter()
        .find(|&&v| v.to_lowercase() == lower)
        .map(|v| format_ident!("{v}"))
}

pub(crate) fn get_key_with_alias(key: String) -> Ident {
    match as_hid_keycode(&key) {
        Some(ident) => ident,
        None => unknown_key_panic(&key),
    }
}

/// Panic for a key token matching no known form, suggesting the nearest valid
/// name from any vocabulary. Everything reaching this used to emit an
/// identifier that only failed deep inside the generated code.
fn unknown_key_panic(key: &str) -> ! {
    let names = rmk_types::action::KeyboardAction::VARIANTS
        .iter()
        .chain(rmk_types::action::LightAction::VARIANTS.iter())
        .chain(rmk_types::keycode::SpecialKey::VARIANTS.iter())
        .chain(rmk_types::keycode::HidKeyCode::VARIANTS.iter())
        .copied();
    let hint = match closest_name(key, names) {
        Some(s) => format!(" (did you mean {s}?)"),
        None => String::new(),
    };
    panic!("\n\u{274c} keyboard.toml: unknown key '{key}'{hint}")
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

    #[test]
    fn parse_modifiers_resolves_canonical_and_alias_names() {
        let m = parse_modifiers("LCtrl | lcmd | algr");
        assert!(m.ctrl);
        assert!(m.gui);
        assert!(m.alt);
        assert!(m.right);
        assert!(!m.shift);
    }

    #[test]
    #[should_panic(
        expected = "`\"home_roww\"` profile name is not found in behavior.morse.profiles (did you mean home_row?)"
    )]
    fn morse_profile_suggests_closest_defined_profile() {
        let profiles = Some(HashMap::from([("home_row".to_string(), profile(None))]));
        let _ = parse_key("TH(Space, Enter, home_roww)".to_string(), &profiles);
    }

    #[test]
    #[should_panic(expected = "unknown key 'Spacee' (did you mean Space?)")]
    fn unknown_keycode_suggests_closest_key() {
        let _ = expand("Spacee");
    }

    #[test]
    #[should_panic(expected = "unknown key 'Bootloade' (did you mean Bootloader?)")]
    fn unknown_action_suggests_closest_action() {
        let _ = expand("Bootloade");
    }

    #[test]
    #[should_panic(expected = "unknown key 'Spacee'")]
    fn composite_action_tap_slot_validates_its_key() {
        let _ = expand("WM(Spacee, RAlt)");
    }

    #[test]
    fn far_off_key_gets_no_hint() {
        let msg = std::panic::catch_unwind(|| expand("ZqZqZq99"))
            .unwrap_err()
            .downcast::<String>()
            .unwrap();
        assert!(msg.contains("unknown key 'ZqZqZq99'"), "{msg}");
        // Nothing within the cutoff, so no fabricated suggestion.
        assert!(!msg.contains("did you mean"), "{msg}");
    }

    #[test]
    fn alias_resolution_survives_validated_lookup() {
        assert!(squash(&expand("lcmd")).contains("HidKeyCode::LGui"));
    }

    #[test]
    #[should_panic(expected = "layer index must be a number, got 'two'")]
    fn invalid_layer_index_is_reported_in_context() {
        let _ = expand("MO(two)");
    }

    #[test]
    #[should_panic(expected = "morse index must be a number, got 'x'")]
    fn invalid_morse_index_is_reported_in_context() {
        let _ = expand("TD(x)");
    }

    #[test]
    #[should_panic(expected = "macro index must be a number, got '9x'")]
    fn invalid_macro_number_form_is_reported_in_context() {
        let _ = expand("Macro9x");
    }

    #[test]
    #[should_panic(expected = "unknown modifier(s) [LCtrol (did you mean LCtrl?)]")]
    fn parse_modifiers_rejects_unknown_name_in_list() {
        let _ = parse_modifiers("LShift | LCtrol");
    }

    #[test]
    #[should_panic(expected = "empty segment")]
    fn parse_modifiers_rejects_trailing_separator() {
        let _ = parse_modifiers("LShift |");
    }

    #[test]
    #[should_panic(expected = "empty segment")]
    fn parse_modifiers_rejects_doubled_separator() {
        let _ = parse_modifiers("LShift || LCtrl");
    }
}
