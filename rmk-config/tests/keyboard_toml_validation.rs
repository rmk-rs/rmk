//! Keyboard TOML validation through the same public config views
//! `#[rmk_keyboard]` consumes. Every bundled `use_config` example must parse
//! and resolve, which guards the whole authoring surface:
//! unknown keys anywhere trip `deny_unknown_fields`, a stale legacy
//! `keymap = [[[…]]]` is rejected, and a mis-sized `map` fails keymap
//! resolution.

use std::path::Path;

use rmk_config::KeyboardTomlConfig;

const MINIMAL_KEYBOARD_TOML: &str = r#"
[keyboard]
name = "RMK Test"
vendor_id = 0x4c4b
product_id = 0x4643
chip = "rp2040"

[matrix]
row_pins = ["PIN_0", "PIN_1"]
col_pins = ["PIN_2", "PIN_3"]

[layout]
rows = 2
cols = 2
"#;

fn write_temp_keyboard_toml(name: &str, extra_toml: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rmk-{name}-{}-{}.toml",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, format!("{MINIMAL_KEYBOARD_TOML}\n{extra_toml}")).unwrap();
    path
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    payload
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| payload.downcast_ref::<&str>().copied())
        .unwrap_or("<panic>")
        .to_string()
}

#[test]
fn all_use_config_examples_resolve() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/use_config");

    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .expect("read examples/use_config")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.join("keyboard.toml").exists())
        .collect();
    dirs.sort();

    // `new_from_toml_path` panics on bad config, so collect per-example results
    // to report every failure at once instead of aborting on the first.
    std::panic::set_hook(Box::new(|_| {}));
    let mut failures = Vec::new();
    for dir in dirs {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let toml = dir.join("keyboard.toml");
        let outcome = std::panic::catch_unwind(|| {
            let config = KeyboardTomlConfig::new_from_toml_path(&toml);
            config.identity().unwrap_or_else(|e| panic!("identity(): {e}"));
            config.hardware().unwrap_or_else(|e| panic!("hardware(): {e}"));
            config.behavior().unwrap_or_else(|e| panic!("behavior(): {e}"));
            config.keymap().unwrap_or_else(|e| panic!("keymap(): {e}"));
            config.layout().unwrap_or_else(|e| panic!("layout(): {e}"));
            config.host();
        });
        if let Err(payload) = outcome {
            let msg = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("<panic>");
            failures.push(format!("{name}: {msg}"));
        }
    }
    let _ = std::panic::take_hook();

    assert!(
        failures.is_empty(),
        "examples failed to resolve:\n{}",
        failures.join("\n")
    );
}

#[test]
fn host_unlock_keys_reject_too_many_entries() {
    let path = write_temp_keyboard_toml(
        "host-unlock-too-many",
        r#"
[host]
unlock_keys = [[0, 0], [0, 1], [1, 0], [1, 1], [0, 0]]
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path(&path);

    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(|| config.host());
    let _ = std::panic::take_hook();
    std::fs::remove_file(path).ok();

    let Err(payload) = result else {
        panic!("host unlock_keys over max must panic");
    };
    let msg = panic_message(payload);
    assert!(
        msg.contains("[host].unlock_keys has 5 entries") && msg.contains("max is 4"),
        "unexpected error: {msg}"
    );
}

#[test]
fn dfu_unlock_keys_reject_too_many_entries() {
    let path = write_temp_keyboard_toml(
        "dfu-unlock-too-many",
        r#"
[dfu]
unlock_keys = [[0, 0], [0, 1], [1, 0], [1, 1], [0, 0]]
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path(&path);
    let result = config.hardware();
    std::fs::remove_file(path).ok();

    let Err(msg) = result else {
        panic!("dfu unlock_keys over max must fail hardware resolution");
    };
    assert!(
        msg.contains("[dfu].unlock_keys has 5 entries") && msg.contains("max is 4"),
        "unexpected error: {msg}"
    );
}

#[test]
fn dfu_unlock_keys_reject_positions_outside_layout() {
    let path = write_temp_keyboard_toml(
        "dfu-unlock-outside-layout",
        r#"
[dfu]
unlock_keys = [[0, 0], [2, 0]]
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path(&path);
    let result = config.hardware();
    std::fs::remove_file(path).ok();

    let Err(msg) = result else {
        panic!("dfu unlock_keys outside layout must fail hardware resolution");
    };
    assert!(
        msg.contains("[dfu].unlock_keys position (2, 0)") && msg.contains("outside the 2x2 matrix"),
        "unexpected error: {msg}"
    );
}

/// Unknown keys in the sections users edit most must be rejected, not
/// silently dropped (pre-fix they surfaced as a misleading "X is required"
/// error that never named the typo).
#[test]
fn unknown_keys_are_rejected() {
    let cases = [
        ("top-level section typo", "[matirx]\nrow_pins = []\n", "matirx"),
        ("[matrix] field typo", "[matrix]\nrow_pin = [\"P0_01\"]\n", "row_pin"),
        (
            "[keyboard] field typo",
            "[keyboard]\nname = \"x\"\nvendor_di = 1\n",
            "vendor_di",
        ),
    ];

    std::panic::set_hook(Box::new(|_| {}));
    for (case, toml, typo) in cases {
        let path = std::env::temp_dir().join(format!("rmk-deny-{}-{typo}.toml", std::process::id()));
        std::fs::write(&path, toml).unwrap();
        let result = std::panic::catch_unwind(|| KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path));
        std::fs::remove_file(&path).ok();

        let payload = result.err().unwrap_or_else(|| panic!("{case}: accepted silently"));
        let msg = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("<panic>");
        assert!(
            msg.contains("unknown field") && msg.contains(typo),
            "{case}: error should name `{typo}`, got: {msg}"
        );
    }
    let _ = std::panic::take_hook();
}

#[test]
fn alias_keys_reject_delimiter_characters() {
    let path = write_temp_keyboard_toml(
        "alias-bad-key",
        r#"
[aliases]
"bad(name" = "A"

[keymap]

[[keymap.layer]]
keys = "A A A A"
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path(&path);
    let result = config.keymap();
    std::fs::remove_file(path).ok();

    let Err(msg) = result else {
        panic!("alias key with a delimiter must fail keymap resolution");
    };
    assert!(
        msg.contains("bad(name") && msg.contains("must not contain"),
        "unexpected error: {msg}"
    );
}

#[test]
fn dfu_storage_conflict_reports_explicit_storage_keys() {
    let cases = [
        (
            "dfu-start-addr",
            "[dfu]\n\n[storage]\nstart_addr = 0x100000\n",
            true,
            false,
        ),
        ("dfu-num-sectors", "[dfu]\n\n[storage]\nnum_sectors = 8\n", false, true),
        (
            "dfu-both",
            "[dfu]\n\n[storage]\nstart_addr = 0x100000\nnum_sectors = 8\n",
            true,
            true,
        ),
    ];
    for (name, extra, expect_start, expect_sectors) in cases {
        let path = write_temp_keyboard_toml(name, extra);
        let config = KeyboardTomlConfig::new_from_toml_path(&path);
        std::fs::remove_file(path).ok();

        let conflict = config
            .dfu_storage_conflict()
            .unwrap_or_else(|| panic!("{name}: expected a conflict"));
        assert_eq!(
            (conflict.start_addr_set, conflict.num_sectors_set),
            (expect_start, expect_sectors),
            "{name}: unexpected conflict flags"
        );
    }
}

#[test]
fn dfu_storage_conflict_absent_without_user_storage() {
    let cases = [
        ("dfu-only", "[dfu]\n"),
        ("dfu-empty-storage", "[dfu]\n\n[storage]\n"),
        ("storage-only", "[storage]\nstart_addr = 0x100000\nnum_sectors = 8\n"),
    ];
    for (name, extra) in cases {
        let path = write_temp_keyboard_toml(name, extra);
        let config = KeyboardTomlConfig::new_from_toml_path(&path);
        std::fs::remove_file(path).ok();

        assert!(config.dfu_storage_conflict().is_none(), "{name}: expected no conflict");
    }
}

#[test]
fn split_side_dfu_replaces_global_per_side() {
    let path = write_temp_keyboard_toml(
        "split-side-dfu",
        r#"
[split]
connection = "serial"

[split.central]
rows = 1
cols = 2
row_offset = 0
col_offset = 0
[split.central.matrix]
matrix_type = "normal"
row_pins = ["PIN_0"]
col_pins = ["PIN_1"]

[[split.peripheral]]
rows = 1
cols = 1
row_offset = 1
col_offset = 2
[split.peripheral.matrix]
matrix_type = "normal"
row_pins = ["PIN_2"]
col_pins = ["PIN_3"]

[dfu]
led = "PIN_4"
[dfu.external_flash]
driver = "w25q"
flash_size = 8388608
spi = { instance = "SPI0", sck = "PIN_5", mosi = "PIN_6", miso = "PIN_7", cs = "PIN_8" }

[split.peripheral.dfu]
led = "PIN_9"
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path(&path);
    std::fs::remove_file(path).ok();

    // Central: global [dfu] with external flash.
    let central = config.split_side_dfu(None).unwrap().unwrap();
    assert_eq!(central.led.map(|l| l.pin), Some("PIN_4".into()));
    assert!(
        central.external_flash.is_some(),
        "central should keep the external flash"
    );

    // Peripheral: own section completely replaces the global one.
    let peripheral = config.split_side_dfu(Some(0)).unwrap().unwrap();
    assert_eq!(peripheral.led.map(|l| l.pin), Some("PIN_9".into()));
    assert!(
        peripheral.external_flash.is_none(),
        "peripheral's own [split.peripheral.dfu] must drop the external flash"
    );
}

#[test]
fn split_side_dfu_falls_back_to_global() {
    let path = write_temp_keyboard_toml(
        "split-side-dfu-fallback",
        r#"
[split]
connection = "serial"

[split.central]
rows = 1
cols = 2
row_offset = 0
col_offset = 0
[split.central.matrix]
matrix_type = "normal"
row_pins = ["PIN_0"]
col_pins = ["PIN_1"]

[[split.peripheral]]
rows = 1
cols = 1
row_offset = 1
col_offset = 2
[split.peripheral.matrix]
matrix_type = "normal"
row_pins = ["PIN_2"]
col_pins = ["PIN_3"]

[dfu]
led = "PIN_4"
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path(&path);
    std::fs::remove_file(path).ok();

    // No per-side section: both sides use the global [dfu].
    let central = config.split_side_dfu(None).unwrap().unwrap();
    let peripheral = config.split_side_dfu(Some(0)).unwrap().unwrap();
    assert_eq!(central.led.map(|l| l.pin), Some("PIN_4".into()));
    assert_eq!(peripheral.led.map(|l| l.pin), Some("PIN_4".into()));
}

#[test]
fn split_central_dfu_replaces_global_for_central_only() {
    let path = write_temp_keyboard_toml(
        "split-central-dfu",
        r#"
[split]
connection = "serial"

[split.central]
rows = 1
cols = 2
row_offset = 0
col_offset = 0
[split.central.matrix]
matrix_type = "normal"
row_pins = ["PIN_0"]
col_pins = ["PIN_1"]

[[split.peripheral]]
rows = 1
cols = 1
row_offset = 1
col_offset = 2
[split.peripheral.matrix]
matrix_type = "normal"
row_pins = ["PIN_2"]
col_pins = ["PIN_3"]

[dfu]
led = "PIN_4"
[dfu.external_flash]
driver = "w25q"
flash_size = 8388608
spi = { instance = "SPI0", sck = "PIN_5", mosi = "PIN_6", miso = "PIN_7", cs = "PIN_8" }

[split.central.dfu]
led = "PIN_9"
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path(&path);
    std::fs::remove_file(path).ok();

    // Central: own [split.central.dfu] completely replaces the global one.
    let central = config.split_side_dfu(None).unwrap().unwrap();
    assert_eq!(central.led.map(|l| l.pin), Some("PIN_9".into()));
    assert!(
        central.external_flash.is_none(),
        "central's own [split.central.dfu] must drop the external flash"
    );

    // Peripheral: no own section, falls back to the global [dfu].
    let peripheral = config.split_side_dfu(Some(0)).unwrap().unwrap();
    assert_eq!(peripheral.led.map(|l| l.pin), Some("PIN_4".into()));
    assert!(
        peripheral.external_flash.is_some(),
        "peripheral should keep the global external flash"
    );
}
