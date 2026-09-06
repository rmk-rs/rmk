use rmk_config::JoystickConfig;

fn joystick(extra: &str) -> JoystickConfig {
    toml::from_str(&format!(
        r#"
name = "test"
pin_x = "P0_31"
pin_y = "P0_29"
pin_z = "_"
transform = [[80, 0], [0, 80]]
bias = [29130, 29365]
resolution = 6
{extra}
"#
    ))
    .unwrap()
}

#[test]
fn defaults_and_old_config_remain_valid() {
    let c = joystick("");
    assert_eq!(c.polling_rate_hz, 50);
    assert_eq!(c.idle_polling_rate_hz, 10);
    assert_eq!(c.sample_settle_us, 20);
    assert_eq!(c.boot_settle_ms, 2);
    assert_eq!(c.deadzone, 0);
    assert!(c.power_pin.is_none());
    assert!(c.validate_power_config().is_ok());
}

#[test]
fn invalid_ranges_fail_validation() {
    for extra in [
        "polling_rate_hz = 0",
        "idle_polling_rate_hz = 0",
        "idle_polling_rate_hz = 51",
        "deadzone = 32768",
    ] {
        assert!(joystick(extra).validate_power_config().is_err(), "{extra}");
    }
}
