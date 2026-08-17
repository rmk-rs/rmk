# Supported Hardwares

Leveraging the great Rust embedded ecosystem, RMK is able to support a wide range of hardware across different architectures, including Cortex-M, Xtensa, and RISC-V.

Below is a (non-exhaustive) list of the currently supported hardware. The `keyboard.toml` column shows whether the chip can be selected with `[keyboard].chip` (and so used with cloud compilation and `rmkit`); chips marked `-` are supported through the Rust API only.

| Hardware    | Architectures     | Connectivity | Tested on Hardware | Examples              | `keyboard.toml` | Note                                          |
| ----------- | ----------------- | ------------ | ------------------ | --------------------- | --------------- | --------------------------------------------- |
| STM32       | Cortex-M0/3/4/7   | USB          | ✅                 | Partial (F1/F4/G4/H7) | ✅              | Supported on all models with USB peripheral   |
| ESP32C3     | RISC-V            | BLE          | ✅                 | ✅                    | ✅              | ESP32-C3 lacks full USB functionality         |
| ESP32C6     | RISC-V            | BLE          | ✅                 | ✅                    | ✅              | ESP32-C6 lacks full USB functionality         |
| ESP32H2     | RISC-V            | BLE          | ✅                 | ✅                    | ✅              | ESP32-H2 lacks full USB functionality         |
| ESP32S3     | Xtensa            | USB + BLE    | ✅                 | ✅                    | ✅              |                                               |
| RP2040      | Cortex-M0+        | USB + BLE    | ✅                 | ✅                    | ✅              | BLE is available on the Raspberry Pi Pico W   |
| RP2350      | Cortex-M33/RISC-V | USB + BLE    | ✅                 | ✅                    | -               | BLE is available on the Raspberry Pi Pico 2 W |
| nRF54LM20   | Cortex-M33        | USB + BLE    | ✅                 | ✅                    | -               | -                                             |
| nRF54L15    | Cortex-M33        | BLE          | ✅                 | ✅                    | -               | No USB peripheral                             |
| nRF52840/33 | Cortex-M4F        | USB + BLE    | ✅                 | ✅                    | ✅              | -                                             |
| nRF52832    | Cortex-M4F        | BLE          | ✅                 | ✅                    | ✅              | -                                             |
| nRF52820    | Cortex-M4         | USB + BLE    | -                  | -                     | -               | No SAADC, so no battery ADC; not tested       |
| nRF52810/11 | Cortex-M4         | BLE          | -                  | Partial (52810)       | ✅              | not tested                                    |
| PY32F07X    | Cortex-M0+        | USB          | ✅                 | ✅                    | -               | Storage support is currently unavailable      |
| SF32LB52    | Cortex-M33        | USB + BLE    | ✅                 | ✅                    | -               | -                                             |

## Adding Support for New Hardware

RMK can run on any hardware platform with [Embassy](https://github.com/embassy-rs/embassy) support. To enable specific communication:

- USB Support: Requires implementation of [embassy-usb-driver](https://github.com/embassy-rs/embassy/tree/main/embassy-usb-driver) traits.
- BLE Support: Requires implementation of [bt-hci](https://github.com/embassy-rs/bt-hci) traits.

Once your hardware has the corresponding trait implementations, RMK support will be available out of the box.
