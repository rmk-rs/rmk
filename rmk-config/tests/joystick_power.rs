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
    assert_eq!(c.power_on_duty, 100);
    assert_eq!(c.idle_polling_rate_hz, 10);
    assert_eq!(c.idle_power_on_duty, 20);
    assert_eq!(c.sample_settle_us, 20);
    assert_eq!(c.boot_settle_ms, 2);
    assert_eq!(c.deadzone, 0);
    assert!(c.power_pin.is_none());
    assert!(c.validate_power_config().unwrap().is_empty());
}

#[test]
fn impossible_duty_is_a_warning_not_an_error() {
    let c = joystick("power_pin = 'P0_10'\npower_on_duty = 1");
    let warnings = c.validate_power_config().unwrap();
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("20 us"));
}

#[test]
fn both_modes_warn_when_wait_exceeds_period() {
    let c = joystick("power_pin = 'P0_10'\nsample_settle_us = 100000");
    let warnings = c.validate_power_config().unwrap();
    assert_eq!(warnings.len(), 4);
    assert!(warnings.iter().any(|w| w.contains("active")));
    assert!(warnings.iter().any(|w| w.contains("idle")));
}

#[test]
fn invalid_ranges_fail_validation() {
    for extra in [
        "polling_rate_hz = 0",
        "idle_polling_rate_hz = 0",
        "power_on_duty = 0",
        "idle_power_on_duty = 1001",
        "idle_polling_rate_hz = 51",
        "deadzone = 32768",
    ] {
        assert!(joystick(extra).validate_power_config().is_err(), "{extra}");
    }
}

#[test]
fn continuous_supply_is_valid() {
    let c = joystick("power_pin = 'P0_10'\npower_on_duty = 1000\nidle_power_on_duty = 1000");
    assert!(c.validate_power_config().unwrap().is_empty());
}
