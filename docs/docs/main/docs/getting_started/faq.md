# FAQ

### My matrix is row2col, the matrix doesn't work

RMK uses col2row as the default matrix diode direction. To use the row2col matrix, you can add `row2col = true` under the `[matrix]` section or `[split.central.matrix]` section:

```toml
# keyboard.toml
[matrix]
row2col = true

# Or
[split.central.matrix]
row2col = true
```

If you're using Rust code, set the generics value `COL2ROW` to `false` to make the matrix row2col:

```rust
// col2row matrix
let mut matrix = Matrix::<_, _, _, ROW, COL, true>::new(row_pins, col_pins, debouncer);
// row2col matrix
let mut matrix = Matrix::<_, _, _, ROW, COL, false>::new(row_pins, col_pins, debouncer);
```

### Where is my built firmware?

By default, the built firmware is at `target/<TARGET>/<MODE>` folder, where `<TARGET>` is your microcontroller's [target](../user_guide/create_firmware/local_compilation#2-choose-your-hardware-and-install-the-target) and `<MODE>` is `debug` or `release`, depending on your build mode.

The firmware's name is your project name in `Cargo.toml`. It's actually an `elf` file, but without file extension.

### I want `hex`/`bin`/`uf2` file, how can I get it?

By default, Rust compiler generates `elf` file in target folder. There're a little extra steps for generating `hex`, `bin` or `uf2` file.

- `hex`/`bin`: To generate `hex`/`bin` file, you need [cargo-binutils](https://github.com/rust-embedded/cargo-binutils). You can use

  ```
  cargo install cargo-binutils
  rustup component add llvm-tools
  ```

  to install it. Then, you can use the following command to generate `hex` or `bin` firmware:

  ```
  # Generate .bin file
  cargo objcopy --release -- -O binary rmk.bin
  # Generate .hex file
  cargo objcopy --release -- -O ihex rmk.hex
  ```

- `uf2`: RMK provides [cargo-make](https://github.com/sagiegurari/cargo-make) config for all examples to generate `uf2` file automatically. Check `Makefile.toml` files in the example folders. The following command can be used to generate uf2 firmware:

  ```shell
  # Install cargo-make
  cargo install --force cargo-make

  # Generate uf2
  cargo make uf2 --release
  ```

### I changed the keymap in `keyboard.toml`, but the keyboard is not updated

RMK stores your keymap in flash and assumes you change it live with a host configurator — [Vial](https://vial.rocks) (enabled by default) or RMK's optional native [Rynk](../features/rynk) protocol. Because the stored keymap takes precedence, reflashing the firmware won't change an already-stored keymap by default. For testing scenarios, RMK provides a config `clear_storage` under the `[storage]` section. You can enable it to clear the storage when the keyboard boots.

```toml
[storage]
# Set `clear_storage` to true to clear all the stored info when the keyboard boots
clear_storage = true
```

Note that the storage will be cleared EVERY TIME you reboot the keyboard.

### rust-lld: error: section will not fit in region 'FLASH': overflowed by x bytes

This is because your MCU's flash is too small. Try building in release mode: `cargo build --release`. If the error persists, follow our [`binary size optimization`](/docs/features/binary_size_optimization.md) documentation to reduce your code size.

### Unable to find libclang

On Windows machines, you may get the following error when building the firmware:

```
error: failed to run custom build command for `nrf-mpsl-sys v0.1.1 (https://github.com/alexmoon/nrf-sdc.git?rev=7be9b853e15ca0404d65c623d1ec5795fd96c204#7be9b853)`

Caused by:
  process didn't exit successfully: `C:\Users\User\Documents\rmk\target\release\build\nrf-mpsl-sys-7601ddd28810dbeb\build-script-build` (exit code: 101)
  --- stderr

  thread 'main' panicked at C:\Users\User\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\bindgen-0.70.1\lib.rs:622:27:
  Unable to find libclang: "couldn't find any valid shared libraries matching: ['clang.dll', 'libclang.dll'], set the `LIBCLANG_PATH` environment variable to a path where one of these files can be found (invalid: [])"
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
warning: build failed, waiting for other jobs to finish...
error: failed to run custom build command for `nrf-sdc-sys v0.1.0 (https://github.com/alexmoon/nrf-sdc.git?rev=7be9b853e15ca0404d65c623d1ec5795fd96c204#7be9b853)`

Caused by:
  process didn't exit successfully: `C:\Users\User\Documents\rmk\target\release\build\nrf-sdc-sys-47ab10b68780c6ba\build-script-build` (exit code: 101)
  --- stderr

  thread 'main' panicked at C:\Users\User\.cargo\registry\src\index.crates.io-1949cf8c6b5b557f\bindgen-0.70.1\lib.rs:622:27:
  Unable to find libclang: "couldn't find any valid shared libraries matching: ['clang.dll', 'libclang.dll'], set the `LIBCLANG_PATH` environment variable to a path where one of these files can be found (invalid: [])"
  note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

That's because you don't have LLVM (Clang) installed, or the system doesn't know the path to the installed LLVM (Clang). You can try the solution here: <https://rust-lang.github.io/rust-bindgen/requirements.html#windows>

### Unable to generate bindings: NotExist

On Windows machines, you may get the following error when building the firmware:

```
error: failed to run custom build command for `nrf-sdc-sys v0.2.1 (D:\nrf-sdc\nrf-sdc-sys)`

Caused by:
  process didn't exit successfully: `D:\nrf-sdc\target\debug\build\nrf-sdc-sys-c5ad399d6ba4c579\build-script-build` (exit code: 101)
  --- stderr

  thread 'main' panicked at nrf-sdc-sys\build.rs:283:10:
  Unable to generate bindings: NotExist("./third_party/nordic/nrfxlib/softdevice_controller/include/sdc_soc.h")
  stack backtrace:
     0: std::panicking::begin_panic_handler
               at /rustc/29483883eed69d5fb4db01964cdf2af4d86e9cb2/library\std\src\panicking.rs:697
     ...
  note: Some details are omitted, run with `RUST_BACKTRACE=full` for a verbose backtrace.
```

To resolve this, you need to enable git symlink on Windows:

```bash
git config --global core.symlinks true
```

### I can see an `RMK Start` log, but nothing else

First, you need to check the RCC config of your board. Make sure that the USB's clock is enabled and set to 48 MHz. For example, if you're using STM32F1, you can set the RCC as follows:

```rust
// If you're using a keyboard.toml
#[rmk_keyboard]
mod keyboard {
    use embassy_stm32::{time::Hertz, Config};

    #[Override(chip_config)]
    fn config() -> Config {
        let mut config = Config::default();
        config.rcc.hse = Some(Hse {
            freq: Hertz(8_000_000),
            // Oscillator for bluepill, Bypass for nucleos.
            mode: HseMode::Oscillator,
        });
        config.rcc.pll = Some(Pll {
            src: PllSource::HSE,
            prediv: PllPreDiv::DIV1,
            mul: PllMul::MUL9,
        });
        config.rcc.sys = Sysclk::PLL1_P;
        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV2;
        config.rcc.apb2_pre = APBPrescaler::DIV1;
        config
    }
}
```

If the keyboard still doesn't work, enable full logging trace in `.cargo/config.toml`:

```toml
[env]
DEFMT_LOG = "trace"
```

run `cargo clean` and then `cargo run --release`. Open an [issue](https://github.com/rmk-rs/rmk/issues) with the detailed logs.

### I see "ERROR: Storage is full" error in the log

By default, RMK uses only 2 sectors of your microcontroller's internal flash. You may get the following error if 2 sectors are not big enough to store all your keymaps:

```
ERROR Storage is full
└─ rmk::storage::print_sequential_storage_err @ /Users/haobogu/Projects/keyboard/rmk/rmk/src/storage.rs:577
ERROR Got none when reading keymap from storage at (layer,col,row)=(1,5,8)
└─ rmk::storage::{impl#2}::read_keymap::{async_fn#0} @ /Users/haobogu/Projects/keyboard/rmk/rmk/src/storage.rs:460
ERROR Keymap reading aborted!
└─ rmk::keymap::{impl#0}::new_from_storage::{async_fn#0} @ /Users/haobogu/Projects/keyboard/rmk/rmk/src/keymap.rs:38
```

If you have more sectors available in your internal flash, you can increase `num_sectors` in `[storage]` section of your `keyboard.toml`, or change `storage_config` in your [`RmkConfig`](https://docs.rs/rmk/latest/rmk/config/struct.RmkConfig.html) if you're using Rust API. When using DFU (`dfu_rp` / `dfu_nrf`), the storage partition is placed after the DFU slot (via `rmk-memory.x`) and the default is 8 sectors (32 KB).

### OUTDATED: panicked at embassy-executor: task arena is full.

::: info
This error occurs only for RMK versions **before** v0.7.
:::

The current Embassy requires manual setting of the task arena size. By default, RMK sets it to 32768 in all examples:

```toml
# Cargo.toml
embassy-executor = { version = "0.10", features = [
    "defmt",
    "platform-cortex-m",
    "task-arena-size-32768",
    "executor-thread",
] }
```

If you get the `ERROR panicked at 'embassy-executor: task arena is full.'` error after flashing to your MCU, that means you should increase your Embassy's task arena. Embassy has a series of cargo features to do this; for example, changing the task arena size to 65536:

```diff
# Cargo.toml
embassy-executor = { version = "0.10", features = [
    "defmt",
    "platform-cortex-m",
-   "task-arena-size-32768",
+   "task-arena-size-65536",
    "executor-thread",
] }
```

In the latest git version of Embassy, the task arena size can be calculated automatically, but it requires the **nightly** version of Rust.

If you're comfortable with nightly Rust, you can enable the `nightly` feature of embassy-executor and remove the `task-arena-size-*` feature.

### `rustc` overflowed its stack while compiling `rmk-types`

Some `rustc` versions run out of stack on `rmk-types`' generated keycode tables. Give the compiler a bigger stack:

```bash
export RUST_MIN_STACK=67108864
```

To make it stick for a project, put it in the project's `.cargo/config.toml` instead of exporting it every time:

```toml title=".cargo/config.toml"
[env]
RUST_MIN_STACK = "67108864"
```

### What font is used for the RMK logo?

It's [Honk](https://fonts.google.com/specimen/Honk?categoryFilters=Technology:%2FTechnology%2FColor&preview.text=RMK).
