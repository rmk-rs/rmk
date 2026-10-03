# Wireless and Bluetooth

Enable Bluetooth in `keyboard.toml`, then configure the battery inputs and pairing options your board needs.

## Enable Bluetooth

Add or update the `[ble]` section:

```toml
[ble]
enabled = true
```

On nRF52 boards, you can also set the radio's transmit power and enable 2M PHY:

```toml
[ble]
enabled = true
default_tx_power = 0
use_2m_phy = true
```

`default_tx_power` is in dBm. The supported range depends on the chip. These two radio settings apply to nRF52 boards.

If a legacy host adapter cannot connect at 2M PHY, enable RMK's `use_1m_phy` Cargo feature. This changes the host connection; dongle and split links continue to use 2M PHY.

## Configure battery monitoring

The TOML battery inputs described here are supported on nRF52 BLE boards. Choose unused GPIO pins that match your board's wiring.

| Setting                                        | Purpose                                                                                                                |
| ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| `battery_adc_pin`                              | Measure battery voltage through an ADC pin, or use `"vddh"` for the internal nRF VDDH input.                           |
| `adc_divider_measured` and `adc_divider_total` | Describe the voltage divider connected to a GPIO ADC input.                                                            |
| `charge_state`                                 | Read the charger's status. Works with or without an ADC input.                                                         |
| `charge_led`                                   | Light an LED while charging and blink it below 10% battery. Requires an ADC or charging-state input on the same board. |
| `battery_user_description`                     | Name the battery in its BLE Battery Service.                                                                           |

### Measure battery voltage

For an external voltage divider, set the ADC pin and the divider values. Use the same units for both values:

```toml
[ble]
enabled = true
battery_adc_pin = "P0_05"
adc_divider_measured = 2000
adc_divider_total = 2806
```

`adc_divider_measured` is the resistance across which the ADC measures voltage. `adc_divider_total` is the total resistance of the divider. For example, an ADC measuring across a 2 MΩ resistor in series with an 806 kΩ resistor uses `2000` and `2806`. Both values must be greater than zero; omitted values default to `1`.

For the internal VDDH input on nRF52833 or nRF52840, use this configuration instead:

```toml
[ble]
enabled = true
battery_adc_pin = "vddh"
```

RMK applies the internal 1:5 divider automatically. Omit the divider settings when using `"vddh"`; they are ignored for this input.

### Add a charging indicator

Add the charger's status pin and an optional LED to the same section as your battery configuration:

```toml
[ble]
enabled = true
charge_state = { pin = "P0_20", low_active = true }
charge_led = { pin = "P0_21", low_active = false }
```

For `charge_state`, `low_active = true` means a low pin level indicates charging. For `charge_led`, it means RMK drives the pin low to turn on the LED.

This example works without ADC measurement: the LED follows the charging state, while the percentage remains unknown. Add the ADC settings from the previous section to enable percentage reporting and low-battery blinking.

### Split battery configuration

Add battery fields to the existing `[split.central]` and `[[split.peripheral]]` sections. Each board uses its own pins and divider:

```toml
[split.central]
battery_adc_pin = "P0_05"
adc_divider_measured = 2000
adc_divider_total = 2806
battery_user_description = "Left"
charge_state = { pin = "P0_20", low_active = true }

[[split.peripheral]]
battery_adc_pin = "P0_05"
adc_divider_measured = 2000
adc_divider_total = 2806
battery_user_description = "Right"
charge_state = { pin = "P0_20", low_active = true }
charge_led = { pin = "P0_21", low_active = false }
```

| Settings                        | Central                                                                                      | Peripheral                            |
| ------------------------------- | -------------------------------------------------------------------------------------------- | ------------------------------------- |
| ADC pin and divider             | Uses the central's ADC settings when `battery_adc_pin` is set there; otherwise uses `[ble]`. | Uses only that peripheral's settings. |
| `charge_state` and `charge_led` | Each unset key falls back to `[ble]` independently.                                          | Uses only that peripheral's settings. |

Setting a central ADC pin replaces the whole ADC configuration, including the divider. Divider values omitted from `[split.central]` default to `1`; they do not inherit the `[ble]` values. Peripherals never inherit battery hardware settings from `[ble]`.

A charging-only peripheral can report its charging state to the central. To expose its battery percentage to the BLE host, configure `battery_adc_pin` on that peripheral.

### Battery readings and indicators

BLE reports the last measured percentage for each board, including after a host reconnects. These measurements are retained until the central restarts. While charging, RMK pauses percentage updates. When charging ends, the current percentage becomes unknown until the next ADC sample; the host can still read the previous measurement.

Before the first sample, no measured percentage is available. Depending on how the host reads the service, it may display no percentage or `0%`. A charging-state pin alone does not measure battery level.

The default OLED renderer leaves the battery bars empty when the percentage is unknown. Where space allows, it also shows these labels:

| State                            | Label                                            |
| -------------------------------- | ------------------------------------------------ |
| No battery status available      | `N/A`                                            |
| Percentage unknown, not charging | `?`                                              |
| Percentage unknown, charging     | `CHG`                                            |
| Percentage known                 | The percentage, with `+` appended while charging |

### Using the Rust API

After initializing the status pin, LED pin, keyboard, and BLE transport, construct the battery components and add them to your existing task list:

```rust
use rmk::input_device::battery::{BatteryProcessor, ChargingStateReader};
use rmk::processor::builtin::battery_led::BatteryLedProcessor;
use rmk::run_all;

let mut charging = ChargingStateReader::new(status_pin, true);
let mut battery = BatteryProcessor::charging_only();
let mut led = BatteryLedProcessor::new(led_pin, false);

run_all!(charging, battery, led, keyboard, ble_transport).await;
```

With ADC measurement, replace `charging_only()` with `BatteryProcessor::new(measured, total)` and also run your ADC input device. Use `BatteryProcessor::new(1, 5)` for VDDH. The percentage calculation always uses the supplied divider.

`BleBatteryConfig` controls BLE reporting. It does not configure GPIO pins or start these local components.

## Peripheral battery reporting over BLE GATT

A split central exposes a standard BLE Battery Service (UUID `0x180F`) for its own level and for each peripheral that configures `battery_adc_pin`. A host that supports multiple Battery Services can read each board independently.

Set `battery_user_description` to give a service a readable name. The defaults are `Central` and `Peripheral 0`, `Peripheral 1`, and so on. A name in `[split.central]` overrides the name in `[ble]`. Peripheral names must be set on their own sections and require `battery_adc_pin`.

Peripheral IDs are also exposed in the Characteristic Presentation Format descriptor: IDs `0`, `1`, and `2` map to the Bluetooth ordinal descriptions `first`, `second`, and `third`.

If additional services need more client attribute slots, set the capacity in your project's `.cargo/config.toml`:

```toml
[env]
TROUBLE_HOST_CLIENT_ATT_TABLE_SIZE = "128"
```

This environment setting overrides trouble-host's Cargo feature settings.

## Passkey entry

To enter a BLE pairing passkey on the keyboard, enable RMK's `passkey_entry` Cargo feature and configure:

```toml
[ble]
enabled = true
passkey_entry = true
passkey_entry_timeout = 120
```

Passkey entry is disabled by default. The timeout defaults to 120 seconds and must be at least 30 seconds. RMK rejects smaller values at build time and cancels pairing if entry is not completed before the timeout.

While entering a passkey, the keyboard accepts these keys:

| Key                              | Action                        |
| -------------------------------- | ----------------------------- |
| `0`–`9` on the top row or numpad | Enter a digit.                |
| `Enter` or `Numpad Enter`        | Submit the six-digit passkey. |
| `Escape`                         | Cancel pairing.               |
| `Backspace`                      | Delete the last digit.        |

Other keys are ignored until passkey entry finishes.

## Related documentation

- [Upgrade from v0.9 to v0.10](../migration/v09_v10#battery-setup): migrate GPIO-based battery configuration.
