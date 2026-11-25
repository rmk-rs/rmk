# Wireless/Bluetooth

## Enable Bluetooth

In `keyboard.toml`, set `enabled = true` under `[ble]`. Your firmware also needs the
[Bluetooth Cargo feature for your chip](../features/wireless#supported-microcontrollers).

```toml
[ble]
enabled = true
```

See the [configuration example](./appendix#keyboardtoml) for optional radio and battery settings.

### Passkey entry

To allow typing a code during pairing, add `passkey_entry` to the `rmk` feature list in `Cargo.toml`
and set `passkey_entry = true` in `[ble]`.

## Battery configuration

Battery reporting is optional. If your board preset includes battery settings, use those. To set up
measurement on a single-board keyboard with a voltage divider, add `[battery]` and use the pin and
resistor values from your board's schematic.

```toml
[battery]
battery_adc_pin = "P0_05"
adc_divider_measured = 2000
adc_divider_total = 2806
```

`battery_adc_pin` is the pin between the two resistors. `adc_divider_measured` is the resistance
from that pin to ground, and `adc_divider_total` is the sum of both resistors. Use the same unit
for both values.

If you're upgrading from RMK 0.9, see [how to move your battery settings](../migration/v09_v10#battery-configuration-tables).

### Split keyboards {#split-battery-adc-configuration}

On a split keyboard, put the battery fields in the table for each board you want to measure.

- Use `[split.central.battery]` for the central, the board that connects to your computer or phone.
- Put `[split.peripheral.battery]` below the existing `[[split.peripheral]]` entry for that board,
  before the next peripheral entry.

Omit a board's table to use its preset. If you add a table, include all battery settings that board
needs. See the [nRF52840 split example](https://github.com/rmk-rs/rmk/blob/main/examples/use_config/nrf52840_ble_split/keyboard.toml).

### Show left and right battery levels {#peripheral-battery-reporting-over-ble-gatt}

On a two-board split that measures both batteries, set `central_side = "left"` or
`central_side = "right"` in `[split]` to match the side of the central board. Compatible computers
and phones can then show both battery levels.
