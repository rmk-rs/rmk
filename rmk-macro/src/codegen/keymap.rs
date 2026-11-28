//! Initialize default keymap from config
use std::collections::HashMap;

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use rmk_config::resolved::behavior::MorseProfile;
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

    let num_encoder = keymap.num_encoder;

    let mut layers = vec![];
    let mut encoder_map = vec![];

    for layer in &keymap.keymap {
        layers.push(expand_layer(layer.clone(), &profiles));
    }

    for encoder_layer in &keymap.encoder_map {
        encoder_map.push(expand_encoder_layer(
            encoder_layer.clone(),
            num_encoder,
            &profiles,
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
) -> TokenStream2 {
    let rows = layer.into_iter().map(|row| {
        let keys = row.into_iter().map(|key| parse_key(key, profiles));
        quote! { [#(#keys), *] }
    });
    quote! { [#(#rows), *] }
}

/// Expand a layer for encoder map
pub(crate) fn expand_encoder_layer(
    encoder_layer: Vec<[String; 2]>,
    num_encoder: usize,
    profiles: &Option<HashMap<String, MorseProfile>>,
) -> TokenStream2 {
    let mut encoders = vec![];

    for encoder in encoder_layer {
        let cw_action = parse_key(encoder[0].clone(), profiles);
        let ccw_action = parse_key(encoder[1].clone(), profiles);
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

#[cfg(all(test, feature = "_simulator"))]
mod tests {
    use super::*;

    #[test]
    fn named_layer_actions_expand_on_keys_and_encoders() {
        let cfg: rmk_config::KeyboardTomlConfig = toml::from_str(
            r#"
[layout]
rows = 1
cols = 1
map = "(0,0)"

[[keymap.layer]]
name = "base"
keys = "TO(nav)"
encoders = [["TO(nav)", "MO(base)"]]

[[keymap.layer]]
name = "nav"
keys = "B"

[[input_device.encoder]]
pin_a = "a"
pin_b = "b"
"#,
        )
        .unwrap();
        let keymap = cfg.keymap().unwrap();

        let key = expand_layer(keymap.keymap[0].clone(), &None).to_string();
        assert!(key.contains("LayerToggleOnly (1u8)"), "{key}");

        let encoder =
            expand_encoder_layer(keymap.encoder_map[0].clone(), keymap.num_encoder, &None)
                .to_string();
        assert!(encoder.contains("LayerToggleOnly (1u8)"), "{encoder}");
        assert!(encoder.contains("LayerOn (0u8)"), "{encoder}");
    }
}
