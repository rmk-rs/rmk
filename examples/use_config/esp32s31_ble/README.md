# ESP-Mosaico keyboard (ESP32-S31)

A two-key USB/BLE keyboard for ESP-Mosaico with the
[Multi-function Interaction Module](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s31/esp-mosaico/user_guide_interact.html).
It supports [Rynk](../../../docs/docs/main/docs/features/rynk.md) keymap configuration
and wireless connections through an [RMK dongle](../../../docs/docs/main/docs/features/dongle.md).
This example uses the two mechanical switches; the display and other module
peripherals are not configured.

## Requirements

- ESP-Mosaico, the interaction module, and a USB Type-C data cable.
- Stable [Rust with rustup](https://rustup.rs/) and a Bash shell.
- `espflash 4.6.0` for image generation.
- [esptool 5.4.0](https://docs.espressif.com/projects/esptool/en/latest/esp32s31/installation.html)
  for flashing, available as `esptool` on your `PATH`.

After installing Rust and esptool, install the target and image converter:

```sh
rustup target add riscv32imafc-unknown-none-elf
cargo install espflash --version 4.6.0 --locked
```

Check the installed tools with `espflash --version` and `esptool version`.

## Build

From the RMK repository root:

```sh
cd examples/use_config/esp32s31_ble
cargo build --release --locked
```

The firmware ELF is `target/riscv32imafc-unknown-none-elf/release/rmk-esp32s31`.
Keep the ESP dependency patches in [Cargo.toml](Cargo.toml) and [Cargo.lock](Cargo.lock)
together when copying this example. They pin the S31 BLE and USB HS support used
by this firmware.

## Flash

1. With the board powered off, insert the interaction module into the **left H2 slot**.
2. Connect the Type-C data cable. Hold **BOOT**, press **POWER** to turn the board
   on, then release BOOT. The board enters ROM download mode and exposes a serial port.
3. From the example directory, flash the firmware:

   ```sh
   cargo run --release --locked -- --port PORT
   ```

   Replace `PORT` with the download port, such as `/dev/cu.usbmodem11101` on macOS,
   `/dev/ttyACM0` on Linux, or `COM3` on Windows.
4. After the board restarts, open a text editor and tap `KEY_L` and `KEY_R` to test
   the `A` and `B` keys.

The [runner](run.sh) converts the ELF with espflash, then uses esptool to write,
verify, and restart the board. It supplies the 16 MiB flash size and
[partition table](partitions.csv). Repeat the download-mode procedure for later flashes.

## Default controls

The keymap is defined in [keyboard.toml](keyboard.toml).

| Gesture | Action |
| --- | --- |
| Tap `KEY_L` | Send `A` |
| Tap `KEY_R` | Send `B` |
| Hold `KEY_L` until layer 1 activates, then tap `KEY_R` | Select the dongle (`User8`) |
| Hold `KEY_R` until layer 2 activates, then tap `KEY_L` | Select BLE profile 0 (`User0`) |
| Hold `KEY_L` alone when Rynk requests an unlock | Unlock configuration |

For direct Bluetooth, select BLE profile 0 and pair **RMK Mosaico** in the host's
Bluetooth settings. For dongle use, select the dongle and follow the
[dongle pairing instructions](../../../docs/docs/main/docs/features/dongle.md#pairing).
Use a receiver built with Rynk to configure this keyboard through the dongle.

To replace a saved BLE or dongle connection, keep the mode-selection key held for
five seconds while continuing to hold the layer key. This clears the selected bond.

USB takes priority while connected to a host. Disconnect the keyboard's USB cable
and use battery power to test either wireless mode.

## Pin reference

| Signal | Left H2 (default) | Right H1 |
| --- | --- | --- |
| `KEY_L` | GPIO13 | GPIO11 |
| `KEY_R` | GPIO12 | GPIO10 |

For the right H1 slot, rotate the module 180 degrees and set `direct_pins` to
`[["GPIO11", "GPIO10"]]` in `keyboard.toml`. The firmware enables internal pull-ups
and drives GPIO60 low to power the expansion module. No matrix wiring is needed.

RMK stores settings and bonds in the last 64 KiB of flash at `0xff0000`.
Keep the storage address in `keyboard.toml` aligned with `partitions.csv`.

## Troubleshooting

| Symptom | What to check |
| --- | --- |
| No serial port, or no response when flashing | Repeat the BOOT/POWER procedure and select the newly appearing port. Use a data-capable USB cable. |
| `MemData` timeout or `FlashEnd` error from espflash | Use the `cargo run` command above. Its runner uses esptool for the USB-OTG write operation. |
| Keys do not respond | Check that the module is in H2, or use the H1 pin mapping above. |
| Serial port disappears after flashing | Expected: the application exposes USB HID and Rynk. Runtime logs use UART0 through the TX/RX debug pads. |
