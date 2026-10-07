# Wireless/Bluetooth

### `[ble]`

To enable BLE, add `enabled = true` under the `[ble]` section.

Battery and charging inputs use the same board-local tables across chips. Automatic ADC availability depends on the backend; charging GPIOs and LEDs use the common pipeline.

```toml
# Ble configuration
# To use the default configuration, ignore this section completely
[ble]
# Whether to enable BLE feature
enabled = true
# Set the BLE tx power; higher means better signal but more power consumption. For nRF52840 the maximum tx power is 8.
# nRF52 only, ignored on other chips
default_tx_power = 0
# Whether to enable 2M PHY, defaults to true. nRF52 only, ignored on other chips
use_2m_phy = true
# Enable or disable passkey entry, defaults to false
passkey_entry = false
# Timeout in seconds for passkey entry, defaults to 120
passkey_entry_timeout = 120

[battery]
battery_user_description = "Main"
battery_adc_pin = "P0_05"
adc_divider_measured = 2000
adc_divider_total = 2806
charge_state = { pin = "P0_20", low_active = true }
charge_led = { pin = "P0_21", low_active = false }

```

Some legacy BLE adapters cannot connect to devices using 2M PHY at all. For those hosts, enable the `use_1m_phy` Cargo feature of the `rmk` crate, which makes the keyboard use 1M PHY for the host connection.
This only affects host connections. The dongle link and the split link between the halves always run at 2M PHY, so a keyboard built with both `dongle` and `use_1m_phy` keeps those links fast and still connects to a legacy adapter on its other BLE profiles.

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

### Split battery ADC configuration

For split keyboards, you can configure battery ADC separately for the central and each peripheral:

```toml
[split.central]


[split.central.battery]
battery_adc_pin = "P0_01"
battery_user_description = "Left"
adc_divider_measured = 2000
adc_divider_total = 2806

[[split.peripheral]]

[split.peripheral.battery]
battery_adc_pin = "P0_02"
battery_user_description = "Right"
adc_divider_measured = 2000
adc_divider_total = 2806

```

Notes:

- Each board uses its own battery table; split boards do not inherit `[battery]`.
- Peripherals do **not** fall back to `[ble]`; to enable peripheral battery reporting, set ADC values per peripheral.

### Peripheral battery reporting over BLE GATT

When peripherals are configured to sample their batteries (see above), their levels are forwarded to the central over the split BLE links and re-exposed to the host through standard Battery Service instances (UUID `0x180F`) on the central's GATT server. The host sees one Battery Service instance for:

- the central's own battery level, and
- each peripheral whose `[split.peripheral.battery]` defines `battery_adc_pin`.

Each peripheral's Battery Service uses its peripheral ID to set the description field in the Characteristic Presentation Format descriptor. Peripheral IDs `0`, `1`, and `2` use the Bluetooth SIG ordinal values `first`, `second`, and `third`, respectively. No host-side configuration is required; any host that already reads the central's Battery Level characteristic can discover the additional instances the same way.

Battery Level characteristics also expose a Characteristic User Description descriptor. The defaults are `Central` for the central and `Peripheral 0`, `Peripheral 1`, and so on for peripherals. Set `battery_user_description` in the corresponding battery table to provide a custom name.

The split feature uses trouble-host's default client ATT table size. To reserve more space for client-specific attributes such as CCCDs, set `TROUBLE_HOST_CLIENT_ATT_TABLE_SIZE` in the project environment, for example in `.cargo/config.toml`:

```toml
[env]
TROUBLE_HOST_CLIENT_ATT_TABLE_SIZE = "128"
```

This project-wide override takes precedence over trouble-host Cargo feature settings and can be set to the size required by the enabled services.

## Board battery tables

Use `[battery]` on a unibody keyboard, `[split.central.battery]` on the central,
and `[split.peripheral.battery]` below the corresponding `[[split.peripheral]]`.
All three accept `battery_adc_pin`, `adc_divider_measured`, `adc_divider_total`,
`charge_state`, `charge_led` and `battery_user_description`.

The divider defaults to 1:1; VDDH uses its fixed 1:5 divider. Charger and LED pins
use `{ pin, low_active }`. An LED requires an ADC or charger input on that board.
Without ADC input, a charger reports charging state but no percentage.
Battery inputs run independently of whether BLE is enabled.

A new battery table replaces that board's legacy battery settings as a whole.
Without one, legacy `[ble]` and flat split battery fields remain accepted.
Split boards do not inherit the top-level `[battery]` table. Defaults supplied by
a board preset still apply when a user omits a field; an empty table does not
erase preset values. Battery names and peripheral Battery Services use the same
resolved configuration as the devices.
