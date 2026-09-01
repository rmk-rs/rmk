# Contributing

ANY contributions are welcome! Here is a simple step-by-step guide for developers:

1. Before you start, you may want to read the [Under the Hood](#under-the-hood) section to understand how RMK works. [GitHub Issues](https://github.com/rmk-rs/rmk/issues) is also a good place for questions.

2. Check out the active PRs to make sure that what you want to add isn't already being implemented by others.

3. Write your code!

4. Open a PR to merge your code into the main repo. Make sure all CIs pass.

## Under the Hood

If you're not familiar with RMK, the following is a simple introduction to the source code of RMK.

### Project Architecture

The firmware side of the RMK repository has four crates: `rmk`, `rmk-config`, `rmk-types` and `rmk-macro`. Each is its own cargo workspace; there is no `Cargo.toml` at the repository root.

- `rmk`: The main crate that contains the core firmware logic, including matrix scanning, key processing, USB/BLE communication, and all the runtime services.
- `rmk-macro`: A proc-macro helper for RMK that reads the `keyboard.toml` config file, converts the TOML config to RMK config, and generates the boilerplate code.
- `rmk-config`: Contains the configuration data structures and parsing logic shared between `rmk-macro` and `rmk`, defining how keyboard configurations are represented in memory.
- `rmk-types`: Provides common type definitions used across all RMK crates, such as keyboard actions, key events, and other shared data structures.

The host side lives in the `rynk/` workspace: `rynk` (the Rynk host client), `rynk-usb`, `rynk-ble`, `rynk-wasm` and `rynk-kle`. See `rynk/README.md`.

So, if you want to contribute new features to RMK, look into the `rmk` core crate. If you want to add support for a new chip, update `rmk` (feature gates and drivers), `rmk-config` (the chip family in `src/chip.rs` and a chip default in `src/default_config/`) and `rmk-macro` (the generated initialization code) so that users can use `keyboard.toml` to configure keyboards with your new chip. If you want to add new configurations, look into both `rmk-config` and `rmk/src/config`.

### Dev loop

CI runs `.github/ci/format.sh`, `.github/ci/check.sh`, `.github/ci/test.sh` and `.github/ci/host.sh`; the feature rows they iterate are defined in `.github/ci/_lib.sh` (`RMK_FEATURESETS` for check/clippy, `RMK_TEST_FEATURESETS` for tests). Locally:

- Format everything (nightly rustfmt): `bash scripts/format_all.sh`. Pass `--touched` to format only files changed in the working tree.
- Run one test feature row from `rmk/` (requires [cargo-nextest](https://nexte.st/)): `cargo nextest run --no-default-features --features=split,vial,storage,async_matrix,_ble`
- Run the whole test suite, including the `rynk/` workspace: `bash scripts/test_all.sh`

Keyboard behavior is tested by the TOML scenarios in `rmk/tests/scenarios/`, which `run_tests!` expands into ordinary tests. Read `rmk/tests/scenarios/README.md` before adding a case. `rmk-macro` tests are expansion snapshots and need `cargo install cargo-expand`; run them from `rmk-macro/` with `cargo nextest run --features _simulator`.

### RMK Core

The `rmk` crate is the main crate. It provides several entry APIs to start the keyboard firmware. All the entry APIs are similar; they:

- Initialize the storage, keymap, and matrix first
- Create services: main keyboard service, matrix service, USB service, BLE service, host service (Vial or Rynk), light service, etc.
- Run all tasks in an infinite loop; if a task fails, wait some time and rerun it

Generally, there are 4-5 running tasks at the same time, depending on the user's config. Communication between tasks is done via channels. There are several built-in channels:

- `FLASH_CHANNEL`: a multi-sender, single-receiver channel. Many tasks send `FlashOperationMessage`, such as the BLE task (which saves bond info) and the vial task (which saves keys), etc.
- **Event channels**: RMK uses a type-safe event system where each event type (e.g., `KeyboardEvent`, `LayerChangeEvent`, `BatteryStatusEvent`) has its own `PubSubChannel`. Input devices publish events to their typed channels, and processors subscribe to the event types they care about.
- **Report channels**: each transport has its own report channel (`USB_REPORT_CHANNEL`, `BLE_REPORT_CHANNEL`). After a key event is processed, the keyboard task routes the resulting HID report to the active transport's channel, and the USB/BLE writer task drains it and sends the report to the host.

### Matrix Scanning & Key Processing

An important part of keyboard firmware is how it performs [matrix scanning](https://en.wikipedia.org/wiki/Keyboard_matrix_circuit) and how it processes the scanning result to generate keys.

In RMK, this work is done by `Matrix` and `Keyboard` respectively. The `Matrix` scans the key matrix and sends a `KeyboardEvent` if there's a key change in the matrix. Then the `Keyboard` receives the `KeyboardEvent` and processes it into an actual keyboard report. Finally, the keyboard report is sent to the USB/BLE tasks and forwarded to the host via USB/BLE.
