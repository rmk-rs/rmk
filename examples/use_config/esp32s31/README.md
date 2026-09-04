# esp32s31 USB example

ESP32-S31 is a RISC-V chip, so this example builds with the standard stable Rust toolchain: add the target with `rustup target add riscv32imafc-unknown-none-elf`. The keyboard talks to the host over the chip's high-speed USB OTG port, which uses dedicated pins. BLE is not available yet because [`esp-radio`](https://github.com/esp-rs/esp-hal/tree/main/esp-radio) has no ESP32-S31 support.

[`espflash`](https://github.com/esp-rs/espflash) 4.5.0 or later is required, since earlier releases don't know the ESP32-S31:

```
cargo install cargo-espflash espflash
```

After having everything installed, use the following command to run the example:

```
cd examples/use_config/esp32s31
cargo run --release
```

If espflash reports the following error:

```
Error: espflash::connection_failed

  × Error while connecting to device
  ╰─▶ Serial port not found
```

You should to identify which serial port are connected to your esp board, and use `--port` to specify the used serial port:

```
# Suppose that the esp board are connected to /dev/cu.usbmodem211401
cargo run --release -- --port /dev/cu.usbmodem211401
```

If you want to get some insight of segments of your binary, [`espsegs`](https://github.com/bjoernQ/espsegs) would help:

```
# Install it first
cargo install --git https://github.com/bjoernQ/espsegs

# Check all segments
espsegs target/riscv32imafc-unknown-none-elf/release/rmk-esp32s31 --chip esp32s31
```
