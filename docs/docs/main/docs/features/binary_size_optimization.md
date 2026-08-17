# Binary size

RMK has included many optimizations by default to reduce binary size. But there are still some tricks to reduce the binary size further. If you get a linker error like:

```
= note: rust-lld: error:
        ERROR(cortex-m-rt): The .text section must be placed inside the FLASH memory.
        Set _stext to an address smaller than 'ORIGIN(FLASH) + LENGTH(FLASH)'
```

or some errors occur when writing configs to flash, that means your microcontroller's internal flash is not big enough.

::: tip
For the minimal example, please check out the `examples/use_rust/stm32f1` and `examples/use_config/stm32f1` examples.
:::

There are several approaches to solve the problem:

## Common approaches

### Change `DEFMT_LOG` level

Logging is quite useful when debugging the firmware, but it requires a lot of flash. You can change the default logging level to `error` at `.cargo/config.toml`, to print only error messages and save flash:

```diff
# .cargo/config.toml

[env]
- DEFMT_LOG = "debug"
+ DEFMT_LOG = "error"
```

### Enable unstable feature

According to [embassy's doc](https://embassy.dev/book/#_my_binary_is_still_big_filled_with_stdfmt_stuff), you can set the following in your `.cargo/config.toml`

```toml
[unstable]
build-std = ["core"]
build-std-features = ["panic_immediate_abort"]
```

And then compile your project with **nightly** Rust:

```
cargo +nightly build --release
# Or
cargo +nightly size --release
```

## For `keyboard.toml` users

RMK provides several options that you can use to reduce the binary size:

1. If you don't need storage, you can disable the `storage` feature to save some flash. To disable `storage` feature you need to disable default features of `rmk` crate, and then enable other features you need. This only works for USB-only builds: every BLE chip feature and the `dongle` feature enable `storage` themselves.

2. You can also fully remove `defmt` by removing `defmt` feature from `rmk` crate and similar feature gates from all other dependencies. Setting `defmt_log = false` in `keyboard.toml` (see below) only replaces `defmt-rtt` with a logger that discards everything; `defmt` and `panic-probe` stay in your `Cargo.toml`.

3. If you don't need on-the-fly configuration, you can disable the host configurator feature by disabling default features of the `rmk` crate.

```toml
# The default features `defmt`, `storage`, `vial`, `host_lock`, and `watchdog` are all disabled
rmk = { version = "0.9", default-features = false }
```

If you're using `keyboard.toml`, you'll also need to disable storage, defmt logging, and the host protocol in the toml config:

```toml
# Disable storage, defmt logging and the host protocol in keyboard.toml
[storage]
enabled = false

[dependency]
defmt_log = false

[host]
# With no host feature enabled, both must be false (they must match the Cargo features)
vial_enabled = false
rynk_enabled = false
```

## For Rust code users

### Use `panic-halt`

By default, RMK uses `panic-probe` to print error messages if panic occurs. But `panic-probe` actually takes lots of flash because the panic call can not be optimized. The solution is to use `panic-halt` instead of `panic-probe`:

```diff
# In your binary's Cargo.toml

- panic-probe = { version = "1", features = ["print-defmt"] }
+ panic-halt = "1.0"
```

Then in `main.rs`, use `panic-halt` instead:

```diff
// src/main.rs

- use panic_probe as _;
+ use panic_halt as _;

```

### Remove `defmt-rtt`

You can also remove the entire defmt-rtt logger to save flash.

```diff
# In your binary's Cargo.toml
- defmt-rtt = "1"
```

In this case, you have to implement an empty defmt logger.

```diff
# src/main.rs
- use defmt_rtt as _;

+ #[defmt::global_logger]
+ struct Logger;
+
+ unsafe impl defmt::Logger for Logger {
+     fn acquire() {}
+     unsafe fn flush() {}
+     unsafe fn release() {}
+     unsafe fn write(_bytes: &[u8]) {}
+ }

```

### Totally remove storage and host configurator support

You can disable the `storage` and host protocol (`vial`, or `rynk` if you enabled it) features in `Cargo.toml`:

```toml
# The default features `defmt`, `storage`, `vial`, `host_lock`, and `watchdog` are all disabled
rmk = { version = "0.9", default-features = false }
```

And then remove anything no longer needed in `main.rs`.
