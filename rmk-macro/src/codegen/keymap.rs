//! Initialize default keymap from config
use std::collections::HashMap;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use rmk_config::resolved::behavior::{MorseProfile, StickyKeyProfile};
use rmk_config::resolved::{Behavior, Keymap};

use super::action_parser::parse_key;

/// Read the default keymap setting in `keyboard.toml` and add as a `get_default_keymap` function
/// Also add `get_default_encoder_map`
pub(crate) fn expand_default_keymap(keymap: &Keymap, behavior: &Behavior) -> TokenStream2 {
    let profiles: Option<HashMap<String, MorseProfile>> = behavior
        .morse
        .as_ref()
        .map(|m| m.profiles.clone())
        .filter(|p| !p.is_empty());
    let sticky_profiles: Option<HashMap<String, StickyKeyProfile>> = behavior
        .sticky_key
        .as_ref()
        .map(|sticky| sticky.profiles.clone())
        .filter(|profiles| !profiles.is_empty());

    let num_encoder = keymap.num_encoder;

    let mut layers = vec![];
    let mut encoder_map = vec![];

    for layer in &keymap.keymap {
        layers.push(expand_layer(layer.clone(), &profiles, &sticky_profiles));
    }

    for encoder_layer in &keymap.encoder_map {
        encoder_map.push(expand_encoder_layer(
            encoder_layer.clone(),
            num_encoder,
            &profiles,
            &sticky_profiles,
        ));
    }
    encoder_map.resize(
        keymap.keymap.len(),
        quote! { [::rmk::encoder!(::rmk::k!(No), ::rmk::k!(No)); NUM_ENCODER] },
    );

    quote! {
        pub const fn get_default_keymap() -> [[[::rmk::types::action::KeyAction; COL]; ROW]; NUM_LAYER] {
            [#(#layers), *]
        }

        pub const fn get_default_encoder_map() -> [[::rmk::types::action::EncoderAction; NUM_ENCODER]; NUM_LAYER] {
            [#(#encoder_map), *]
        }
    }
}

/// Expand a layer for keymap
pub(crate) fn expand_layer(
    layer: Vec<Vec<String>>,
    profiles: &Option<HashMap<String, MorseProfile>>,
    sticky_profiles: &Option<HashMap<String, StickyKeyProfile>>,
) -> TokenStream2 {
    let mut rows = vec![];
    for row in layer {
        rows.push(expand_row(row, profiles, sticky_profiles));
    }
    quote! { [#(#rows), *] }
}

/// Expand a row for keymap
fn expand_row(
    row: Vec<String>,
    profiles: &Option<HashMap<String, MorseProfile>>,
    sticky_profiles: &Option<HashMap<String, StickyKeyProfile>>,
) -> TokenStream2 {
    let mut keys = vec![];
    for key in row {
        keys.push(parse_key(key, profiles, sticky_profiles));
    }
    quote! { [#(#keys), *] }
}

/// Expand a layer for encoder map
pub(crate) fn expand_encoder_layer(
    encoder_layer: Vec<[String; 2]>,
    num_encoder: usize,
    profiles: &Option<HashMap<String, MorseProfile>>,
    sticky_profiles: &Option<HashMap<String, StickyKeyProfile>>,
) -> TokenStream2 {
    let mut encoders = vec![];

    for encoder in encoder_layer {
        let cw_action = parse_key(encoder[0].clone(), profiles, sticky_profiles);
        let ccw_action = parse_key(encoder[1].clone(), profiles, sticky_profiles);
        encoders.push(quote! { ::rmk::encoder!(#cw_action, #ccw_action) });
    }

    // Make sure it configures correct number of encoders
    encoders.resize(
        num_encoder,
        quote! { ::rmk::encoder!(::rmk::k!(No), ::rmk::k!(No)) },
    );

    if encoders.is_empty() {
        // An empty `[]` literal has no element type for Rust to infer when
        // `num_encoder == 0`. Emit the typed `[expr; N]` form instead.
        quote! { [::rmk::encoder!(::rmk::k!(No), ::rmk::k!(No)); NUM_ENCODER] }
    } else {
        quote! { [#(#encoders), *] }
    }
}
