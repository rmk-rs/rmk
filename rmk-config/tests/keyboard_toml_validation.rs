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

#[test]
fn battery_adc_rejects_zero_divider_total() {
    let path = write_temp_keyboard_toml(
        "battery-zero-divider-total",
        r#"
[ble]
enabled = true
battery_adc_pin = "P0_02"
adc_divider_total = 0
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path(&path);
    let result = config.hardware();
    std::fs::remove_file(path).ok();

    let Err(msg) = result else {
        panic!("zero battery ADC divider total must fail hardware resolution");
    };
    assert!(
        msg.contains("adc_divider_total") && msg.contains("greater than zero"),
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

#[test]
fn sticky_profiles_resolve_inheritance_and_explicit_empty_values() {
    use rmk_config::resolved::behavior::StickyReleaseCondition;
    let path = write_temp_keyboard_toml(
        "sticky-inheritance",
        r#"
[behavior.sticky_key]
wait_timeout = "750ms"
hold_timeout = "300ms"
ignore = ["Tab"]
release_on = ["before_next_press", "after_next_press", "after_next_release", "layer_enter", "layer_exit"]
[behavior.sticky_key.profiles.z_inherited]
[behavior.sticky_key.profiles.a_overridden]
wait_timeout = "0ms"
hold_timeout = "0ms"
ignore = []
release_on = []
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
    std::fs::remove_file(&path).unwrap();
    let sticky = config.behavior().unwrap().sticky_key.unwrap();
    assert_eq!(sticky.default.wait_timeout_ms, 750);
    assert_eq!(sticky.default.hold_timeout_ms, 300);
    assert_eq!(sticky.default.ignore, ["Tab"]);
    assert_eq!(
        sticky.default.release_on,
        [
            StickyReleaseCondition::BeforeNextPress,
            StickyReleaseCondition::AfterNextPress,
            StickyReleaseCondition::AfterNextRelease,
            StickyReleaseCondition::LayerEnter,
            StickyReleaseCondition::LayerExit
        ]
    );
    assert_eq!(sticky.profiles[0].0, "a_overridden");
    assert_eq!(sticky.profiles[1].0, "z_inherited");
    assert_eq!(sticky.profiles[1].1, sticky.default);
    let overridden = &sticky.profiles[0].1;
    assert_eq!(overridden.wait_timeout_ms, 0);
    assert_eq!(overridden.hold_timeout_ms, 0);
    assert!(overridden.ignore.is_empty());
    assert!(overridden.release_on.is_empty());
}

#[test]
fn sticky_empty_profiles_resolve_builtin_defaults() {
    use rmk_config::resolved::behavior::{
        DEFAULT_STICKY_HOLD_TIMEOUT_MS, DEFAULT_STICKY_WAIT_TIMEOUT_MS, StickyReleaseCondition,
    };
    for rmk_section in ["", "[rmk]\n"] {
        let path = write_temp_keyboard_toml(
            "sticky-defaults",
            &format!("{rmk_section}[behavior.sticky_key.profiles.empty]"),
        );
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        std::fs::remove_file(&path).unwrap();
        let constants = config.build_constants(&[]).unwrap();
        assert_eq!(constants.sticky_max_active, 8);
        assert_eq!(constants.sticky_profile_max_num, 8);
        assert_eq!(constants.sticky_ignore_max, 4);
        let sticky = config.behavior().unwrap().sticky_key.unwrap();
        assert_eq!(sticky.default.wait_timeout_ms, DEFAULT_STICKY_WAIT_TIMEOUT_MS);
        assert_eq!(sticky.default.hold_timeout_ms, DEFAULT_STICKY_HOLD_TIMEOUT_MS);
        assert!(sticky.default.ignore.is_empty());
        assert_eq!(sticky.default.release_on, [StickyReleaseCondition::AfterNextRelease]);
        assert_eq!(sticky.profiles[0].1, sticky.default);
    }
}

#[test]
fn sticky_rejects_duration_truncation_and_ignore_overflow() {
    for (index, (table, field)) in [
        ("behavior.sticky_key", "wait_timeout"),
        ("behavior.sticky_key", "hold_timeout"),
        ("behavior.sticky_key.profiles.named", "wait_timeout"),
        ("behavior.sticky_key.profiles.named", "hold_timeout"),
    ]
    .into_iter()
    .enumerate()
    {
        let path = write_temp_keyboard_toml(
            &format!("sticky-duration-{index}"),
            &format!("[{table}]\n{field} = \"65536ms\""),
        );
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        std::fs::remove_file(path).unwrap();
        assert!(
            config
                .behavior()
                .err()
                .unwrap()
                .contains(&format!("{table}.{field} must be between 0ms and 65535ms"))
        );
    }
    for (index, table) in ["behavior.sticky_key", "behavior.sticky_key.profiles.named"]
        .into_iter()
        .enumerate()
    {
        let path = write_temp_keyboard_toml(
            &format!("sticky-ignore-{index}"),
            &format!("[rmk]\nsticky_ignore_max = 4\n[{table}]\nignore = [\"A\", \"B\", \"C\", \"D\", \"E\"]"),
        );
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        std::fs::remove_file(path).unwrap();
        assert!(config.behavior().err().unwrap().contains("sticky_ignore_max is 4"));
    }
}

#[test]
fn sticky_capacities_grow_only_when_omitted() {
    let profiles = (0..9)
        .map(|i| format!("[behavior.sticky_key.profiles.p{i}]\n"))
        .collect::<String>();
    let behavior = format!(
        "[behavior.sticky_key]\nignore = [\"A\", \"B\", \"C\", \"D\", \"E\"]\n{profiles}\n[behavior.sticky_key.profiles.long]\nignore = [\"A\", \"B\", \"C\", \"D\", \"E\", \"F\"]"
    );
    for (rmk_section, expected) in [
        ("", (10, 6)),
        ("[rmk]\n", (10, 6)),
        ("[rmk]\nsticky_profile_max_num = 12\nsticky_ignore_max = 7\n", (12, 7)),
    ] {
        let path = write_temp_keyboard_toml("sticky-auto-capacities", &format!("{rmk_section}{behavior}"));
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        std::fs::remove_file(path).unwrap();
        let constants = config.build_constants(&[]).unwrap();
        assert_eq!(
            (constants.sticky_profile_max_num, constants.sticky_ignore_max),
            expected
        );
        let sticky = config.behavior().unwrap().sticky_key.unwrap();
        assert_eq!(sticky.profiles.len(), 10);
        assert_eq!(sticky.profiles[0].1.ignore.len(), 6);
        assert_eq!(sticky.profiles[1].1.ignore, sticky.default.ignore);
    }
    for (setting, limit) in [("sticky_profile_max_num", 8), ("sticky_ignore_max", 4)] {
        let path = write_temp_keyboard_toml(
            "sticky-explicit-capacity",
            &format!("[rmk]\n{setting} = {limit}\n{behavior}"),
        );
        let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
        std::fs::remove_file(path).unwrap();
        assert!(config.behavior().err().unwrap().contains(setting));
    }
    let path = write_temp_keyboard_toml(
        "sticky-default-ignore-capacity",
        "[behavior.sticky_key]\nignore = [\"A\", \"B\", \"C\", \"D\", \"E\"]\n[behavior.sticky_key.profiles.empty]\nignore = []",
    );
    let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
    std::fs::remove_file(path).unwrap();
    assert_eq!(config.build_constants(&[]).unwrap().sticky_ignore_max, 5);
    let sticky = config.behavior().unwrap().sticky_key.unwrap();
    assert!(sticky.profiles[0].1.ignore.is_empty());
}

#[test]
fn sticky_duration_and_capacity_boundaries() {
    let path = write_temp_keyboard_toml(
        "sticky-limits",
        r#"
[rmk]
sticky_max_active = 256
sticky_profile_max_num = 255
sticky_ignore_max = 2
[behavior.sticky_key]
wait_timeout = "65535ms"
hold_timeout = "0ms"
ignore = ["Tab", "LShift"]
[behavior.sticky_key.profiles.empty]
ignore = []
"#,
    );
    let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
    std::fs::remove_file(path).unwrap();
    let constants = config.build_constants(&[]).unwrap();
    assert_eq!(constants.sticky_max_active, 256);
    assert_eq!(constants.sticky_profile_max_num, 255);
    let sticky = config.behavior().unwrap().sticky_key.unwrap();
    assert_eq!(sticky.default.wait_timeout_ms, 65535);
    assert!(sticky.profiles[0].1.ignore.is_empty());

    let path = write_temp_keyboard_toml("sticky-active-overflow", "[rmk]\nsticky_max_active = 257");
    let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
    std::fs::remove_file(path).unwrap();
    assert_eq!(
        config.build_constants(&[]).err().unwrap(),
        "sticky_max_active must be at most 256"
    );

    let path = write_temp_keyboard_toml("sticky-profile-overflow", "[rmk]\nsticky_profile_max_num = 256");
    let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
    std::fs::remove_file(path).unwrap();
    assert!(
        config
            .build_constants(&[])
            .err()
            .unwrap()
            .contains("sticky_profile_max_num must be at most 255")
    );

    let profiles = (0..255)
        .map(|i| format!("[behavior.sticky_key.profiles.p{i}]\n"))
        .collect::<String>();
    let path = write_temp_keyboard_toml("sticky-inferred-profile-limit", &profiles);
    let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
    std::fs::remove_file(path).unwrap();
    assert_eq!(config.build_constants(&[]).unwrap().sticky_profile_max_num, 255);
    assert_eq!(config.behavior().unwrap().sticky_key.unwrap().profiles.len(), 255);

    let profiles = format!("{profiles}[behavior.sticky_key.profiles.overflow]\n");
    let path = write_temp_keyboard_toml("sticky-inferred-profile-overflow", &profiles);
    let config = KeyboardTomlConfig::new_from_toml_path_with_event_defaults(&path);
    std::fs::remove_file(path).unwrap();
    assert!(
        config
            .build_constants(&[])
            .err()
            .unwrap()
            .contains("sticky_profile_max_num must be at most 255")
    );
    assert!(
        config
            .behavior()
            .err()
            .unwrap()
            .contains("sticky_profile_max_num must be at most 255")
    );
}
