# USB Logging

RMK uses [defmt](https://defmt.ferrous-systems.com) as the default logger, which works great if you have a debug probe. If you don’t have a debug probe, you can still view logs over USB by configuring USB as a serial port.

## Usage

To enable USB logging, disable the default features and then enable the `usb_log` feature in `Cargo.toml`:

```toml
rmk = { version = "0.9", default-features = false, features = [
    "storage",
    "usb_log", # Enable USB logging
    "..",
] }
```

The log level is fixed at debug: the `usb_log` feature enables `log/max_level_debug`, and the `log` crate rejects a second `max_level_*` feature at compile time.

::: tip
Don't forget to re-enable the other default features you need (such as `storage`, `vial`, `host_lock`, and `watchdog`) — but not `defmt`: `usb_log` is based on the `log` crate, which cannot be enabled together with the `defmt` feature.
:::

To view the logs, you'll need to install a serial port monitor. Open your serial monitor, select the port corresponding to your keyboard, and connect. The logs will be displayed in the monitor window. Note that logs from the boot stage cannot be captured by the USB logger. You will only be able to see logs after the serial port connection is established.

Some microcontrollers (like ESP32S3) don't have enough USB endpoints, so USB logging cannot be enabled for those microcontrollers. To enable USB logging, make sure that your microcontroller has at least 5 In + 4 OUT endpoints available (except the control endpoint, EP0).

`usb_log` fails to compile on two groups of chips:

- Chips without USB, such as `nrf52832_ble`, `nrf52810_ble`, `nrf52811_ble`, `nrf54l15_ble`, `esp32c3_ble`, `esp32c6_ble` and `esp32h2_ble`.
- High-speed USB chips, currently `nrf54lm20_ble`: `embassy-usb-logger` only handles 64-byte packets, which high-speed bulk endpoints can't use. Use `defmt` on these chips.
