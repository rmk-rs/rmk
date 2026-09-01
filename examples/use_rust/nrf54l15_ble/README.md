# nRF54L15 BLE example

This example targets the nRF54L15 DK.

It uses the DK's four onboard buttons as a 2x2 direct-pin keyboard matrix:

- Button 1 (`P1.13`) -> row 0 / col 0
- Button 2 (`P1.09`) -> row 0 / col 1
- Button 3 (`P1.08`) -> row 1 / col 0
- Button 4 (`P0.04`) -> row 1 / col 1

Storage is backed by the internal flash via `nrf-mpsl`, so keymap/profile changes persist.

## Running

1. Enter the example directory:

   ```shell
   cd examples/use_rust/nrf54l15_ble
   ```

2. Build, flash, and run it:

   ```shell
   cargo run --release
   ```

This example is BLE-only. The current RMK nRF54L15 feature enables `_no_usb`, so there is no USB
fallback path on this board target.
