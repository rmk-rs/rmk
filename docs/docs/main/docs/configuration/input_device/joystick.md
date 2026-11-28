# Joysticks

A joystick controls the mouse pointer through analog inputs. Configuration through `keyboard.toml` supports nRF52 chips with an SAADC peripheral. Use a debug probe to calibrate the joystick for your hardware.

## TOML configuration

Add one entry per joystick to `keyboard.toml`:

```toml
[[input_device.joystick]]
name = "default"
# id = 0
pin_x = "P0_31"
pin_y = "P0_29"
pin_z = "_"
transform = [[80, 0], [0, 80]]
bias = [29130, 29365]
resolution = 6
```

| Field                     | Description                                                                                                         |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `name`                    | Unique joystick name.                                                                                               |
| `id`                      | Device ID matched by `JoystickProcessor`. Defaults to sequential IDs starting at 0.                                 |
| `pin_x`, `pin_y`, `pin_z` | ADC pins in axis order. Use `_` for an unused Z axis.                                                               |
| `bias`                    | Offset added to each axis to center its resting value at zero.                                                      |
| `transform`               | Divisors mapping input axes (columns) to output axes (rows). A zero entry ignores that contribution.                |
| `resolution`              | Positive movement step used to reduce small fluctuations. Values are rounded toward zero to multiples of this step. |

Match the dimensions of `bias` and `transform` to the number of configured axes. The example uses two axes; larger positive diagonal divisors reduce pointer speed.

### Calibration

1. For the two-axis example, start with `bias = [0, 0]`, `transform = [[1, 0], [0, 1]]`, and `resolution = 1`.
2. Enable debug logging and read `JoystickProcessor::generate_report: record = [...]` with the joystick released. Set each bias to the negative of that axis's resting value.
3. Increase the diagonal transform values to reduce pointer speed.
4. Increase `resolution` if the pointer jitters at rest.

## Sampling

RMK reads the configured axes in one ADC scan. It treats the first scan as the joystick's resting position. After the joystick has been still and near that position for 1.2 seconds, it uses the idle interval; holding the joystick deflected keeps active sampling. Battery voltage changes do not trigger fast joystick sampling. When battery measurement shares the ADC with a joystick, battery voltage is reported at most once every 30 seconds.

## Rust configuration

Use one `NrfAdc` for the joystick and battery channels. In your firmware initialization, list events in ADC channel order and match the joystick device ID to its processor. This example assumes `p`, `matrix`, and `keymap` are already initialized.

`polling_interval` sets the active scan interval. `light_sleep` sets the initial and idle interval; `None` uses `polling_interval` throughout.

```rust
use embassy_nrf::saadc::{self, Input as _};
use embassy_time::Duration;
use rmk::input_device::{
    adc::{AnalogEventType, NrfAdc},
    battery::BatteryProcessor,
    joystick::JoystickProcessor,
};

embassy_nrf::bind_interrupts!(struct AdcIrqs {
    SAADC => saadc::InterruptHandler;
});

let saadc_config = saadc::Config::default();
let adc = saadc::Saadc::new(p.SAADC, AdcIrqs, saadc_config,
    [
        saadc::ChannelConfig::single_ended(saadc::VddhDiv5Input.degrade_saadc()),
        saadc::ChannelConfig::single_ended(p.P0_31.degrade_saadc()),
        saadc::ChannelConfig::single_ended(p.P0_29.degrade_saadc())
    ],
);
adc.calibrate().await;
let mut adc_dev = NrfAdc::new(
    adc,
    [AnalogEventType::Battery, AnalogEventType::Joystick(2)],
    [0, 0], // device id per event; unused for battery events
    Duration::from_millis(20), // polling interval
    Some(Duration::from_millis(350)), // light sleep interval
);
let mut batt_proc = BatteryProcessor::new(1, 5);
let mut joy_proc = JoystickProcessor::new(0, [[80, 0], [0, 80]], [29130, 29365], 6, &keymap);
rmk::run_all!(matrix, adc_dev, joy_proc, batt_proc).await;
```
