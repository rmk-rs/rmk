//! Initialize default keymap from config
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use rmk_config::resolved::{Behavior, Keymap};

use super::action_parser::{ProfileNames, parse_key};

/// Read the default keymap setting in `keyboard.toml` and add as a `get_default_keymap` function
/// Also add `get_default_encoder_map`
pub(crate) fn expand_default_keymap(keymap: &Keymap, behavior: &Behavior) -> TokenStream2 {
    let names = super::behavior::profile_names(behavior);

    let num_encoder = keymap.num_encoder;

    let mut layers = vec![];
    let mut encoder_map = vec![];

    for layer in &keymap.keymap {
        layers.push(expand_layer(layer.clone(), &names));
    }

    for encoder_layer in &keymap.encoder_map {
        encoder_map.push(expand_encoder_layer(
            encoder_layer.clone(),
            num_encoder,
            &names,
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
pub(crate) fn expand_layer(layer: Vec<Vec<String>>, names: &ProfileNames) -> TokenStream2 {
    let mut rows = vec![];
    for row in layer {
        rows.push(expand_row(row, names));
    }
    quote! { [#(#rows), *] }
}

/// Expand a row for keymap
fn expand_row(row: Vec<String>, names: &ProfileNames) -> TokenStream2 {
    let mut keys = vec![];
    for key in row {
        keys.push(parse_key(key, names));
    }
    quote! { [#(#keys), *] }
}

/// Expand a layer for encoder map
pub(crate) fn expand_encoder_layer(
    encoder_layer: Vec<[String; 2]>,
    num_encoder: usize,
    names: &ProfileNames,
) -> TokenStream2 {
    let mut encoders = vec![];

    for encoder in encoder_layer {
        let cw_action = parse_key(encoder[0].clone(), names);
        let ccw_action = parse_key(encoder[1].clone(), names);
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
