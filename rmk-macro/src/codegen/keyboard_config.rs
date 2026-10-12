use quote::quote;
use rmk_config::KeyboardTomlConfig;
use rmk_config::resolved::{Host, Identity, Keymap};

pub(crate) fn read_keyboard_toml_config() -> KeyboardTomlConfig {
    // Get the path of the keyboard config file from the environment variable
    let config_toml_path = std::env::var("KEYBOARD_TOML_PATH")
        .expect("[ERROR]: KEYBOARD_TOML_PATH should be set in `.cargo/config.toml`");

    KeyboardTomlConfig::new_from_toml_path(&config_toml_path)
}

pub(crate) fn expand_keyboard_info(
    identity: &Identity,
    keymap: &Keymap,
) -> proc_macro2::TokenStream {
    let pid = identity.product_id;
    let vid = identity.vendor_id;
    let product_name = identity.product_name.clone();
    let manufacturer = identity.manufacturer.clone();
    let serial_number_tokens = match &identity.serial_number {
        Some(s) => quote! { #s },
        None => quote! { ::rmk::config::RMK_BUILD_INFO },
    };

    let num_col = keymap.cols as usize;
    let num_row = keymap.rows as usize;
    let num_layer = keymap.layers as usize;
    let total_num_encoder = keymap.num_encoder;
    quote! {
        pub(crate) const COL: usize = #num_col;
        pub(crate) const ROW: usize = #num_row;
        pub(crate) const NUM_LAYER: usize = #num_layer;
        pub(crate) const NUM_ENCODER: usize = #total_num_encoder;
        const KEYBOARD_DEVICE_CONFIG: ::rmk::config::DeviceConfig = ::rmk::config::DeviceConfig {
            vid: #vid,
            pid: #pid,
            manufacturer: #manufacturer,
            product_name: #product_name,
            serial_number: #serial_number_tokens,
        };
    }
}

/// The `[host].unlock_keys` list as a `&'static [(u8, u8)]` literal (empty when unset).
fn unlock_keys_tokens(host: &Host) -> proc_macro2::TokenStream {
    if host.unlock_keys.is_empty() {
        return quote! { &[] };
    }
    let keys_expr = host.unlock_keys.iter().map(|key| {
        let row = key[0];
        let col = key[1];
        quote! { (#row, #col) }
    });
    quote! { &[#(#keys_expr),*] }
}

pub(crate) fn expand_vial_config(host: &Host) -> proc_macro2::TokenStream {
    if !host.vial_enabled {
        return quote! {};
    }
    let unlock_keys = unlock_keys_tokens(host);
    let insecure = host.insecure;
    quote! {
        include!(concat!(env!("OUT_DIR"), "/config_generated.rs"));
        const VIAL_CONFIG: ::rmk::config::VialConfig = ::rmk::config::VialConfig {
            vial_keyboard_id: &VIAL_KEYBOARD_ID,
            vial_keyboard_def: &VIAL_KEYBOARD_DEF,
            unlock_keys: #unlock_keys,
            insecure: #insecure,
        };
    }
}

pub(crate) fn expand_lock_config(host: &Host) -> proc_macro2::TokenStream {
    if !host.rynk_enabled {
        return quote! {};
    }
    let unlock_keys = unlock_keys_tokens(host);
    let insecure = host.insecure;
    let write_requires_unlock = host.write_requires_unlock;
    let bootloader_requires_unlock = host.bootloader_requires_unlock;
    quote! {
        const LOCK_CONFIG: ::rmk::config::LockConfig = ::rmk::config::LockConfig {
            unlock_keys: #unlock_keys,
            insecure: #insecure,
            write_requires_unlock: #write_requires_unlock,
            bootloader_requires_unlock: #bootloader_requires_unlock,
        };
    }
}
