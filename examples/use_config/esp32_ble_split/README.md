# ESP32C6/ESP32C3 BLE Split Keyboard Example

This example demonstrates a split keyboard configuration where:
- **Central** uses ESP32C6 
- **Peripheral** uses ESP32C3

## Prerequisites

To run this example, you should have the latest Rust installed. The full instruction of installing esp Rust toolchain can be found [here](https://docs.esp-rs.org/book/installation/index.html).

[`espflash`](https://github.com/esp-rs/espflash) should also be installed:

```
cargo install cargo-espflash espflash
```

## Building and Flashing

We've provided convenient aliases in `.cargo/config.toml`:

```bash
cd examples/use_config/esp32_ble_split

# Build and flash central (ESP32C6)
cargo run-central

# Build and flash peripheral (ESP32C3)
cargo run-peripheral

# Just build (without flashing)
cargo build-central
cargo build-peripheral
```

## Expected Output

If everything is good, you'll see the log as the following:

```shell
cargo run --release --bin central
    Compiling ...
    ...
    ...
    Finished `release` profile [optimized + debuginfo] target(s) in 11.70s
     Running `espflash flash --target esp32c6 --monitor target/riscv32imac-unknown-none-elf/release/central`
[2025-04-10T10:01:23Z INFO ] Serial port: '/dev/cu.usbmodem211401'
[2025-04-10T10:01:23Z INFO ] Connecting...
[2025-04-10T10:01:23Z INFO ] Using flash stub
Chip type:         esp32c6 (revision v0.1)
Crystal frequency: 40 MHz
Flash size:        4MB
Features:          WiFi 6, BT 5
MAC address:       40:4c:ca:5b:c7:dc
App/part. size:    768,944/4,128,768 bytes, 18.62%
[2025-04-10T10:01:23Z INFO ] Segment at address '0x0' has not changed, skipping write
[2025-04-10T10:01:23Z INFO ] Segment at address '0x8000' has not changed, skipping write
[00:00:06] [========================================]     411/411     0x10000                                                                                             [2025-04-10T10:01:31Z INFO ] Flashing has completed!
```

## Troubleshooting

If espflash reports the following error:

```
Error: espflash::connection_failed

  × Error while connecting to device
  ╰─▶ Serial port not found
```

You should identify which serial port is connected to your esp board, and use `--port` to specify the used serial port:

```
# For central (ESP32C6)
cargo run-central -- --port /dev/cu.usbmodem211401

# For peripheral (ESP32C3)
cargo run-peripheral -- --port /dev/cu.usbmodem211402
```

## Binary Analysis

If you want to get some insight of segments of your binary, [`espsegs`](https://github.com/bjoernQ/espsegs) would help:

```
# Install it first
cargo install --git https://github.com/bjoernQ/espsegs

# Check central binary (ESP32C6)
espsegs target/riscv32imac-unknown-none-elf/release/central --chip esp32c6

# Check peripheral binary (ESP32C3)
espsegs target/riscv32imc-unknown-none-elf/release/peripheral --chip esp32c3
```