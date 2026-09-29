# nrf52840 BLE split example

RMK supports [nice!nano](https://nicekeyboards.com/nice-nano) as well as any custom nrf52840 board you have. 

## Build firmware

You can build firmware for central and peripheral separately:

```shell
# Build central firmware
cargo build --release --bin central

# Build peripheral firmware
cargo build --release --bin peripheral
```

## Nice!nano support

nice!nano has the [Adafruit_nRF52_Bootloader](https://github.com/adafruit/Adafruit_nRF52_Bootloader) built-in, which supports .uf2 firmware format. That means you don't need any debugging probe to flash your firmware. RMK uses `cargo-make` tool to generate .uf2 firmware, then generation processing is defined in `Makefile.toml`

The following are steps of how to get .uf2 firmware work in RMK:

1. Get `cargo-make` tool:
   ```shell
   cargo install --force cargo-make
   ```
2. Compile RMK and get .uf2:
   ```shell
   cargo make uf2 --release
   ```
3. Flash

   Set your nice!nano to bootloader mode, a USB drive will show. Just drag the .uf2 firmware to USB drive. RMK will be automatically flashed. Check nice!nano's document: https://nicekeyboards.com/docs/nice-nano/getting-started#flashing-firmware-and-bootloaders. 

Note that RMK will switch to USB mode if an USB cable is connected. Remember to remove USB cable after flashing!

You can also check the instruction [here](https://nicekeyboards.com/docs/nice-nano/) for more info about nice!nano.

## With debugging probe

With a debugging probe, you can have the full control of you hardware. To use RMK you should check whether the bootloader is flashed to your board first. To use RMK with existing bootloader such as [Adafruit_nRF52_Bootloader](https://github.com/adafruit/Adafruit_nRF52_Bootloader), check `memory.x` in the project root, ensure that the flash starts from 0x00001000

```
MEMORY
{
  /* NOTE 1 K = 1 KiB = 1024 bytes */
  /* These values correspond to the nRF52840 WITH Adafruit nRF52 bootloader */
  FLASH : ORIGIN = 0x00001000, LENGTH = 1020K
  RAM : ORIGIN = 0x20000008, LENGTH = 255K
}
```

Or you can use RMK without bootloader:

```
MEMORY
{
  /* NOTE 1 K = 1 KiB = 1024 bytes */
  /* These values correspond to the nRF52840 */
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K
  RAM : ORIGIN = 0x20000000, LENGTH = 256K
}
```

After you have `memory.x` set, use `cargo run --release` to flash the RMK firmware to your board:

1. Enter example folder:
   ```shell
   cd examples/use_config/nrf52840_ble_split
   ```
2. Compile, flash and run the example
   ```shell
   # Run central firmware
   cargo run --release --bin central

   # Run peripheral firmware
   cargo run --release --bin peripheral
   ```
