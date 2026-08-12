use std::path::Path;
use std::{env, fs};

use rmk_config::resolved::BuildConstants;
use rmk_config::{KeyboardTomlConfig, protocol_limits};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=KEYBOARD_TOML_PATH");
    println!("cargo:rerun-if-env-changed=VIAL_JSON_PATH");

    // Build-time constants only need [rmk] + [event] + [layout], so load event
    // defaults without requiring [keyboard.board]/[keyboard.chip].
    let toml_path = std::env::var("KEYBOARD_TOML_PATH").ok();
    let config: KeyboardTomlConfig = if let Some(toml_path) = &toml_path {
        println!("cargo:rerun-if-changed={toml_path}");
        KeyboardTomlConfig::new_from_toml_path_with_event_defaults(toml_path)
    } else {
        toml::from_str("").expect("Failed to parse empty keyboard config\n")
    };

    // Enabled features drive constant resolution (notably event subscriber counts).
    let active_features = collect_active_features();
    let feature_refs: Vec<&str> = active_features.iter().map(|s| s.as_str()).collect();

    let bc = config
        .build_constants(&feature_refs)
        .unwrap_or_else(|err| panic!("Failed to resolve build constants: {err}"));
    let output = generate_constants(&bc, &config);

    let out_dir = env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("constants.rs");
    fs::write(&dest_path, output).expect("Failed to write constants.rs file");
}

fn generate_constants(bc: &BuildConstants, config: &KeyboardTomlConfig) -> String {
    let mut lines = Vec::new();

    // Direct constants
    lines.push(format!(
        "pub const MOUSE_KEY_INTERVAL: u16 = {};",
        bc.mouse_key_interval
    ));
    lines.push(format!(
        "pub const MOUSE_WHEEL_INTERVAL: u16 = {};",
        bc.mouse_wheel_interval
    ));
    lines.push(format!("pub const COMBO_MAX_NUM: usize = {};", bc.combo_max_num));
    lines.push(format!("pub const COMBO_MAX_LENGTH: usize = {};", bc.combo_max_length));
    lines.push(format!("pub const MACRO_SPACE_SIZE: usize = {};", bc.macro_space_size));
    lines.push(format!("pub const FORK_MAX_NUM: usize = {};", bc.fork_max_num));
    lines.push(format!("pub const DEBOUNCE_THRESHOLD: u16 = {};", bc.debounce_time));
    lines.push(format!(
        "pub const REPORT_CHANNEL_SIZE: usize = {};",
        bc.report_channel_size
    ));
    lines.push(format!(
        "pub const VIAL_CHANNEL_SIZE: usize = {};",
        bc.vial_channel_size
    ));
    lines.push(format!(
        "pub const FLASH_CHANNEL_SIZE: usize = {};",
        bc.flash_channel_size
    ));
    lines.push(format!(
        "pub const SPLIT_PERIPHERALS_NUM: usize = {};",
        bc.split_peripherals_num
    ));
    lines.push(format!("pub const NUM_BLE_PROFILE: usize = {};", bc.ble_profiles_num));
    lines.push(format!(
        "pub const SPLIT_CENTRAL_SLEEP_TIMEOUT_SECONDS: u32 = {};",
        bc.split_central_sleep_timeout_seconds
    ));
    lines.push(format!("pub const MORSE_MAX_NUM: usize = {};", bc.morse_max_num));
    lines.push(format!(
        "pub const MORSE_PROFILE_MAX_NUM: usize = {};",
        bc.morse_profile_max_num
    ));
    lines.push(format!(
        "pub const AUTO_MOUSE_LAYER_MAX_NUM: usize = {};",
        bc.auto_mouse_layer_max_num
    ));
    lines.push(format!(
        "pub const MAX_PATTERNS_PER_KEY: usize = {};",
        bc.max_patterns_per_key
    ));

    // Host uses protocol ceilings; firmware uses keyboard.toml/default capacities.
    let is_host = env::var("CARGO_FEATURE_HOST").is_ok();

    // Protocol ceilings — always emitted so rmk-types source code can reference them.
    lines.push(format!(
        "pub const MAX_COMBO_SIZE: usize = {};",
        protocol_limits::MAX_COMBO_SIZE
    ));
    lines.push(format!(
        "pub const MAX_MORSE_SIZE: usize = {};",
        protocol_limits::MAX_MORSE_SIZE
    ));
    lines.push(format!(
        "pub const MAX_MACRO_DATA_SIZE: usize = {};",
        protocol_limits::MAX_MACRO_DATA_SIZE
    ));

    if is_host {
        // Host: Vec capacities equal protocol ceilings for wire compatibility with any firmware.
        lines.push(format!(
            "pub const COMBO_SIZE: usize = {};",
            protocol_limits::MAX_COMBO_SIZE
        ));
        lines.push(format!(
            "pub const MORSE_SIZE: usize = {};",
            protocol_limits::MAX_MORSE_SIZE
        ));
        lines.push(format!(
            "pub const MACRO_DATA_SIZE: usize = {};",
            protocol_limits::MAX_MACRO_DATA_SIZE
        ));
    } else {
        // Firmware: per-item constants from keyboard.toml / defaults.
        lines.push(format!("pub const COMBO_SIZE: usize = {};", bc.combo_max_length));
        lines.push(format!("pub const MORSE_SIZE: usize = {};", bc.max_patterns_per_key));
        lines.push(format!(
            "pub const MACRO_DATA_SIZE: usize = {};",
            bc.protocol_macro_chunk_size
        ));
        // Firmware Vec sizes must not exceed protocol ceilings (rynk builds only).
        if env::var("CARGO_FEATURE_RYNK").is_ok() {
            lines.push("const _: () = assert!(COMBO_SIZE <= MAX_COMBO_SIZE, \"firmware COMBO_SIZE exceeds protocol ceiling MAX_COMBO_SIZE\");".to_string());
            lines.push("const _: () = assert!(MORSE_SIZE <= MAX_MORSE_SIZE, \"firmware MORSE_SIZE exceeds protocol ceiling MAX_MORSE_SIZE\");".to_string());
            lines.push("const _: () = assert!(MACRO_DATA_SIZE <= MAX_MACRO_DATA_SIZE, \"firmware MACRO_DATA_SIZE exceeds protocol ceiling MAX_MACRO_DATA_SIZE\");".to_string());
        }
    }

    // The exact RAM of each rynk frame buffer; the payload budget and bulk
    // capacities derive from it in `protocol::rynk`.
    if env::var("CARGO_FEATURE_RYNK").is_ok() {
        lines.push(format!("pub const RYNK_BUFFER_SIZE: usize = {};", bc.rynk_buffer_size));

        // The physical layout served over `GetLayout`, baked here so that firmware
        // reads it from `RmkConfig::default()`. Empty without a `[layout]` section.
        let blob = config
            .layout()
            .unwrap_or_else(|err| panic!("Failed to resolve the layout blob: {err}"))
            .blob;
        lines.push(format!("pub const LAYOUT_BLOB: &[u8] = b\"{}\";", blob.escape_ascii()));
    }

    // How long one dongle pairing window lasts. It bonds exactly one keyboard,
    // so there is nothing else about the relay to size.
    if env::var("CARGO_FEATURE_DONGLE").is_ok() {
        lines.push(format!(
            "pub const DONGLE_PAIRING_WINDOW_SECS: u32 = {};",
            bc.dongle_pairing_window_secs
        ));
    }

    // Event channels
    for ev in &bc.events {
        let upper = ev.name.to_uppercase();
        lines.push(format!(
            "pub const {upper}_EVENT_CHANNEL_SIZE: usize = {};",
            ev.channel_size
        ));
        lines.push(format!("pub const {upper}_EVENT_PUB_SIZE: usize = {};", ev.pubs));
        lines.push(format!("pub const {upper}_EVENT_SUB_SIZE: usize = {};", ev.subs));
    }

    // Passkey (feature-gated)
    if env::var("CARGO_FEATURE_PASSKEY_ENTRY").is_ok() {
        if let Some(passkey) = &bc.passkey {
            lines.push(format!("pub const PASSKEY_ENTRY_ENABLED: bool = {};", passkey.enabled));
            lines.push(format!(
                "pub const PASSKEY_ENTRY_TIMEOUT_SECS: u32 = {};",
                passkey.timeout_secs
            ));
        } else {
            // No [ble] section but passkey_entry feature enabled: use defaults
            lines.push("pub const PASSKEY_ENTRY_ENABLED: bool = false;".to_string());
            lines.push(format!(
                "pub const PASSKEY_ENTRY_TIMEOUT_SECS: u32 = {};",
                rmk_config::DEFAULT_PASSKEY_ENTRY_TIMEOUT_SECS
            ));
        }
    }

    lines.join("\n")
}

/// Active Cargo feature flags, lowercased to match `subscriber_default.toml`.
///
/// Cargo exposes each enabled feature as `CARGO_FEATURE_<NAME>` (uppercased,
/// `-` → `_`); we reverse that.
fn collect_active_features() -> Vec<String> {
    env::vars()
        .filter_map(|(key, _)| key.strip_prefix("CARGO_FEATURE_").map(|f| f.to_lowercase()))
        .collect()
}
