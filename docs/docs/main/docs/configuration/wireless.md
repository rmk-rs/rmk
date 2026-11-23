# Wireless/Bluetooth

### `[ble]`

To enable BLE, add `enabled = true` under the `[ble]` section.

```toml
# Ble configuration
# To use the default configuration, ignore this section completely
[ble]
# Whether to enable BLE feature
enabled = true
# Set the BLE tx power; higher means better signal but more power consumption. For nRF52840 the maximum tx power is 8.
# nRF52 only, ignored on other chips
default_tx_power = 0
# Host connection PHY: true = 2M (default), false = 1M.
use_2m_phy = true
# Enable or disable passkey entry, defaults to false
passkey_entry = false
# Timeout in seconds for passkey entry, defaults to 120
passkey_entry_timeout = 120
```

### Host connection PHY

`[ble].use_2m_phy` requests 2M (`true`, default) or 1M (`false`) for host connections on all supported BLE chips.

To request 1M:

```toml
[ble]
enabled = true
use_2m_phy = false
```

The `use_1m_phy` Cargo feature overrides this setting to request 1M. Dongle and split links always request 2M.

### Passkey entry

RMK supports typing a BLE passkey directly on the keyboard during pairing. This is disabled by default, and requires the `passkey_entry` Cargo feature of the `rmk` crate in addition to the configuration below.

```toml
[ble]
# Enable or disable passkey entry (default: false)
# When disabled, passkey pairing requests from the host are automatically rejected.
passkey_entry = true
# Timeout in seconds for passkey entry (default: 120, minimum: 30)
# If the user does not finish entering the passkey within this time, pairing is cancelled.
# Setting this below 30 will cause a build error.
passkey_entry_timeout = 120
```

During passkey mode, the keyboard intercepts all keypresses. Only the following keys are recognized:

| Key                         | Action                     |
| --------------------------- | -------------------------- |
| `0`–`9` (top row or numpad) | Enter a digit              |
| `Enter` / `Numpad Enter`    | Submit the 6-digit passkey |
| `Escape`                    | Cancel pairing             |
| `Backspace`                 | Delete the last digit      |

All other keys are silently discarded while passkey mode is active.

## Battery configuration

Battery configuration controls voltage measurement, charging-state detection, and an optional indicator LED. These features also work with BLE disabled.

Choose the table for the board you are configuring:

| Board            | Table                                                                |
| ---------------- | -------------------------------------------------------------------- |
| Unibody keyboard | `[battery]`                                                          |
| Split central    | `[split.central.battery]`                                            |
| Split peripheral | `[split.peripheral.battery]`, below its `[[split.peripheral]]` entry |

Configure each split board separately. Use only the split battery tables for a split keyboard; the top-level `[battery]` table is not accepted.

### Battery fields

All three tables accept the same fields. Omit inputs and outputs that your board does not have.

| Field                      | Description                                                                                                                                                             | Default                                                  |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- |
| `battery_adc_pin`          | ADC pin connected to the battery voltage divider, or `"vddh"` for the internal VDDH input on nRF52840/nRF52833.                                                         | No voltage measurement                                   |
| `adc_divider_measured`     | Resistance between the ADC input and ground. Requires `battery_adc_pin`.                                                                                                | `1`                                                      |
| `adc_divider_total`        | Total resistance of the voltage divider. Requires `battery_adc_pin`.                                                                                                    | `1`                                                      |
| `charge_state`             | Charging-status input as `{ pin, low_active }`. Set `low_active = true` if the charger drives the pin low while charging.                                               | No charging-status input                                 |
| `charge_led`               | Indicator output as `{ pin, low_active }`. Set `low_active = true` if driving the pin low turns the LED on. Requires an ADC or charging-status input on the same board. | No indicator LED                                         |
| `battery_user_description` | Battery name exposed over BLE. A peripheral requires `battery_adc_pin` to expose its battery service.                                                                   | `"Central"` or `"Peripheral N"`, where `N` starts at `0` |

For unibody keyboards, omitted fields use the board preset; an empty `[battery]` table keeps those defaults.

For split keyboards, an omitted side table uses the board preset. An explicit table replaces it entirely; an empty table disables that side's battery inputs.

Set both divider values in the same units. For example, a divider with 806 kΩ between the battery and ADC input and 2 MΩ between the input and ground uses `2000` and `2806`. Both values must be greater than zero. With `battery_adc_pin = "vddh"`, RMK uses the internal 1:5 divider and ignores these two fields.

On nRF52 chips with an ADC, RMK sets up voltage measurement automatically. The examples below use nRF52840 pins; replace them and the divider values to match your board.

With only `charge_state` configured, RMK reports charging state without a battery percentage. The indicator LED stays on while charging, blinks below 10% when not charging, and stays off otherwise.

### Unibody keyboard

Add `[battery]` to `keyboard.toml`. This example measures voltage through an external divider and reads a charging-status pin:

```toml
[battery]
battery_adc_pin = "P0_05"
adc_divider_measured = 2000
adc_divider_total = 2806
charge_state = { pin = "P0_20", low_active = true }
# Optional indicator LED
charge_led = { pin = "P0_21", low_active = false }
battery_user_description = "Main"
```

### Split battery ADC configuration

Add a battery table to override the preset for that side. Place each `[split.peripheral.battery]` table after the corresponding `[[split.peripheral]]` entry and before the next peripheral entry. Keep each board's existing matrix and connection settings.

```toml
[split.central.battery]
battery_adc_pin = "P0_01"
adc_divider_measured = 2000
adc_divider_total = 2806
battery_user_description = "Left"

[[split.peripheral]]
# Keep this peripheral's existing board settings here.

[split.peripheral.battery]
battery_adc_pin = "P0_02"
adc_divider_measured = 2000
adc_divider_total = 2806
battery_user_description = "Right"
```

For an existing configuration with battery fields under `[ble]` or directly under a split board, see [Migrate battery configuration](../migration/v09_v10#battery-configuration-tables).

### RP2040 battery input

Set `battery_adc_pin` to an ADC-capable pin, such as `PIN_26`, and set the divider values to match your board. RP2040 voltage measurement assumes a 3.3 V ADC reference.

For a unibody board with a 1:2 divider:

```toml
[battery]
battery_adc_pin = "PIN_26"
adc_divider_measured = 1
adc_divider_total = 2
```

For a split board, put these fields in its central or peripheral battery table. RMK waits 30 seconds before each sample, including the first. If a read fails, it keeps the previous reading and retries at the next sampling interval. Boards with a different ADC reference need a custom Rust reader that publishes ADC input millivolts.

### ESP32 battery input

Set `battery_adc_pin` to an ADC1-capable GPIO for your chip and set the divider values to match your board. RMK uses calibrated voltage readings and waits 30 seconds before each sample, including the first.

For an ESP32-C3 unibody board with a 1:2 divider:

```toml
[battery]
battery_adc_pin = "GPIO0"
adc_divider_measured = 1
adc_divider_total = 2
```

For a split board, put these fields in its central or peripheral battery table. GPIO numbering and ADC support vary by chip; the pin in this example is for ESP32-C3.

Battery measurement reserves ADC1. This configuration requires RMK's ESP32 BLE initialization, which enables the radio before using the hardware random-number generator.

### Peripheral battery reporting over BLE GATT

The central exposes its own battery first using the standard Battery Service (`0x180F`) and Battery Level (`0x2A19`). On a two-board keyboard that measures both batteries, set `central_side = "left"` or `"right"` in `[split]` to report **Left** and **Right** in their presentation descriptors (`0x2904`). The readable names (`0x2901`) default to `Central` and `Peripheral 0` and honor `battery_user_description`. A host that uses the first discovered service sees the central, including on right-central keyboards.

Omitting `central_side`, measuring fewer than two batteries, or configuring three or more boards exposes **Main** as the sole standard battery. Each peripheral with a configured battery ADC exposes one top-level RMK Peripheral Battery service (`061686b6-6a06-4d02-8a8f-c514225bbdb3`). Read or subscribe to its level characteristic (`cd3259e5-a755-4bf0-9f36-690cddccdb51`) for that board's unsigned 8-bit percentage. For peripheral `N` (zero-based), the description field of its presentation descriptor (`0x2904`) is `N + 1`. Its user description (`0x2901`) defaults to `Peripheral N`.

After upgrading a previously paired multi-board keyboard, a host that cached the old battery services may need to rediscover them.

The split feature uses trouble-host's default client ATT table size. To reserve more space for client-specific attributes such as CCCDs, set `TROUBLE_HOST_CLIENT_ATT_TABLE_SIZE` in the project environment, for example in `.cargo/config.toml`:

```toml
[env]
TROUBLE_HOST_CLIENT_ATT_TABLE_SIZE = "128"
```

This project-wide override takes precedence over trouble-host Cargo feature settings and can be set to the size required by the enabled services.
