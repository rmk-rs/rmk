# ESP-Mosaico keyboard (ESP32-S31)

A two-key USB / BLE keyboard using ESP-Mosaico and its
[Multi-function Interaction Module](https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32s31/esp-mosaico/user_guide_interact.html).
It also connects to an [RMK dongle](../../../docs/docs/main/docs/features/dongle.md).
Rynk configures the keymap over USB, BLE, or the dongle.

## Hardware

Insert the interaction module into the **left H2 slot** with power off.
H2 is the USB Serial/JTAG side; H1 is the UART/codec side.

| Key | Left H2 (default) | Right H1 |
| --- | --- | --- |
| KEY_L | GPIO13 | GPIO11 |
| KEY_R | GPIO12 | GPIO10 |

For the right slot, change `direct_pins` in `keyboard.toml` to
`[["GPIO11", "GPIO10"]]` and rotate the module as shown in its guide.
The keys are active-low and use internal pull-ups. The configuration drives
GPIO60 low to enable expansion power. No matrix wiring or diodes are needed.

## Build and flash

Use stable Rust, **espflash 4.6.0 or later** for image generation, and
**esptool 5.4.0 or later** for flashing over the Mosaico Type-C port:

```sh
rustup target add riscv32imafc-unknown-none-elf
cargo install espflash --version 4.6.0 --locked
esptool version
cargo build --release --locked
```

Install [esptool](https://github.com/espressif/esptool) if it is not available.
The runner uses esptool because espflash 4.6.0 fails on the S31 USB-OTG path:
stub upload times out, and ROM-only flashing returns a `FlashEnd` error.

ESP32-S31 BLE requires the upstream
[S31 BLE implementation](https://github.com/esp-rs/esp-hal/commit/86508e8ad9563883e3a7fec14730a6c89569cb54),
which is not in the published `esp-radio 1.0.0-beta.1`.
The manifest pins a later revision containing that implementation and the
[S31 USB RX FIFO fix](https://github.com/HaoboGu/esp-hal/commit/163e6b62b5b3f1d1dc2cae10b6ccc68ed9e92fde).
Keep its patches and lockfile together when copying this example. It uses the
newer `esp_rtos::start(timer)` API.

Connect the Type-C port to your computer. With the board off, hold **BOOT**
and press **POWER** to enter ROM download mode, then release BOOT:

```sh
cargo run --release --locked
```

The runner supplies the 16 MiB flash size and `partitions.csv`. RMK storage
occupies the last 64 KiB at `0xff0000`; keep the TOML storage address and
partition table in sync. Power-cycle after flashing if the board remains in
download mode. Repeat the BOOT procedure for later flashes.

The Type-C port becomes USB HID + Rynk when the firmware runs. Logs go to
UART0 through the board's TX/RX debug pads; the runner does not attach a
serial monitor to the HID port.

## Keys and wireless use

| Gesture | Action |
| --- | --- |
| Tap KEY_L / KEY_R | A / B |
| Hold KEY_L until layer 1 activates, then tap KEY_R | Select / pair the dongle (`User8`) |
| Hold KEY_R until layer 2 activates, then tap KEY_L | Select BLE profile 0 (`User0`) |
| Keep the selected `User8` or `User0` key held for 5 seconds | Clear that bond and pair again |
| Hold KEY_L alone when Rynk requests unlock | Unlock configuration |

For direct BLE, select profile 0 and pair **RMK Mosaico** in the host's Bluetooth
settings. For dongle use, choose a receiver built with Rynk and follow the
[dongle pairing steps](../../../docs/docs/main/docs/features/dongle.md#pairing).
USB takes priority while the keyboard is connected to a USB host; unplug its
USB cable and use the onboard battery to test wireless typing.

This example uses the two mechanical switches. Display, touch, LEDs, audio,
sensors and battery-gauge reporting are not configured. Automatic battery ADC
calibration is unavailable for S31 in esp-hal. Builds and image generation can
be checked locally; USB enumeration, typing and BLE pairing still need testing
on physical boards.
