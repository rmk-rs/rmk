# Use Rust

By default, the generated project uses `keyboard.toml` to configure the RMK keyboard firmware. If you want to customize your firmware using Rust, there are several steps you need to follow to make the generated firmware project compile.

### Update memory.x

`memory.x` is the linker script of a Rust embedded project; it's used to define the memory layout of the microcontroller. The generated project enables the `memory-x` feature for `embassy-stm32`, so if you're using STM32, you can just ignore this step.

For other ARM Cortex-M microcontrollers, you only need to update the `LENGTH` of FLASH and RAM for your microcontroller.

If you're using **nRF52840**, ensure that you have [Adafruit_nRF52_Bootloader](https://github.com/adafruit/Adafruit_nRF52_Bootloader) flashed to your board. Most nice!nano compatible boards have it already. As long as you can open a USB drive for your board and update the uf2 firmware by dragging and dropping, you're all set.

You can check either your microcontroller's datasheet or an existing Rust project for your microcontroller for the correct values.

::: note
If you're using **rmk-boot** with DFU firmware updates (`dfu_rp` / `dfu_nrf`), you don't need to write a `memory.x` manually. Download `rmk-memory.x` alongside the bootloader binary, place it next to your `Cargo.toml`, and configure your `build.rs` as described in the [embassy-boot flashing guide](../user_guide/flash_firmware/use_embassy_boot.mdx).
:::

### Update `main.rs`

The generated `main.rs` needs to be updated as well to use Rust code. You can copy the code from RMK's Rust example, such as <https://github.com/rmk-rs/rmk/blob/main/examples/use_rust/rp2040/src/main.rs> to `src/main.rs` to get started.

Next, check `src/main.rs` and make sure the bound USB interrupt is correct. Different microcontrollers have different USB peripherals, which means the interrupt bindings will also differ. Your best resource is to check [Embassy's examples](https://github.com/embassy-rs/embassy/tree/main/examples) for your chip family to see how to bind the USB interrupt correctly.

For example, if you're using STM32F4, there is a [USB serial example](https://github.com/embassy-rs/embassy/blob/main/examples/stm32f4/src/bin/usb_serial.rs) there. The code for binding the USB interrupt is at [lines 15-17](https://github.com/embassy-rs/embassy/blob/main/examples/stm32f4/src/bin/usb_serial.rs#L15-L17):

```rust
bind_interrupts!(struct Irqs {
    OTG_FS => usb::InterruptHandler<peripherals::USB_OTG_FS>;
});
```

### Add your own layout

This step only applies when you build with the [Vial](https://get.vial.today/) protocol (the `vial` feature), like the Rust examples do. RMK supports the [Vial app](https://get.vial.today/), an open-source cross-platform (Windows/macOS/Linux/web) keyboard configurator. The Vial-like keymap definition needs to be imported into the firmware project.

Fortunately, RMK does most of the heavy lifting for you. All you need to do is create your own keymap definition and convert it to `vial.json` following **[Vial's documentation here](https://get.vial.today/docs/porting-to-via.html)**, and place it at the root of the firmware project, replacing the default one. RMK will handle all the rest for you.

If you opt into [Rynk](./rynk) instead of the default Vial protocol, you don't need a `vial.json` — skip this step.

### Add your default keymap

After adding the layout of your keyboard, the default keymap should also be updated. The default keymap is defined in `src/keymap.rs`. Update the keyboard matrix constants and add a `get_default_keymap()` function that returns the default keymap of your keyboard.

RMK provides a bunch of useful [macros](https://docs.rs/rmk/latest/rmk/#macros) to help you define your keymap. Check out the [keymap configuration](../configuration/keymap_configuration) chapter for more details. You can also check the `src/keymap.rs` files in the <https://github.com/rmk-rs/rmk/blob/main/examples/use_rust> examples for reference.

Some `KeyAction`s are not supported by the macros; plain `KeyAction`s also work, for example: `KeyAction::TapHold(Action::Key(KeyCode::Hid(HidKeyCode::Kc1)), Action::Key(KeyCode::Hid(HidKeyCode::Kc2)), u8::MAX)` (the last argument indexes the morse profile table; use `u8::MAX` for the default tap-hold timing)

### Define your matrix

Next, you're going to change the I/O pins of the keyboard matrix to make RMK run on your own PCB. Generally, I/O pins are defined in `src/main.rs`. RMK will generate a helper macro to help you define the matrix. For example, if you're using RP2040, you can define your pins using `config_matrix_pins_rp!`:

```rust
let (row_pins, col_pins) = config_matrix_pins_rp!(
    peripherals: p,
    input: [PIN_6, PIN_7, PIN_8, PIN_9],
    output: [PIN_19, PIN_20, PIN_21]
);
```

`input` and `output` are lists of used pins; change them accordingly.

If your keys are directly connected to the microcontroller pins, you can define your pins like this:

```rust
    let direct_pins = config_matrix_pins_rp! {
        peripherals: p,
        direct_pins: [
            [PIN_0, PIN_1,  PIN_2],
            [PIN_3, _,  PIN_5],
        ]
    };
```

So far so good, you've done all necessary modifications of your firmware project. You can also check TODOs listed in the generated `README.md` file. Happy coding!
