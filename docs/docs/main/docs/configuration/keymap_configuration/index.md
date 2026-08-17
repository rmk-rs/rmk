# Keymap configuration

RMK supports configuring the default keymap at the compile time. Keymap in RMK is a 3-D matrix of [`KeyAction`](https://docs.rs/rmk-types/latest/rmk_types/action/enum.KeyAction.html), which represent the keyboard's action after you trigger a physical key. The 3 dimensions are the number of columns, rows and layers.

RMK provides both Rust code or config ways to set your default keymap.

## Define default keymap in `keyboard.toml`

Please check [layout section](../layout) in keyboard configuration doc.

## Define default keymap in Rust source file

The default keymap could also be defined at a Rust source file, There are `keymap.rs`s in example folder, such as [this](https://github.com/rmk-rs/rmk/blob/main/examples/use_rust/nrf52840_ble/src/keymap.rs), which could be a good example of defining keymaps using Rust in RMK. A simplified two-layer version:

```rust
use rmk::types::action::KeyAction;
use rmk::{a, k, layer, mo};
pub(crate) const COL: usize = 14;
pub(crate) const ROW: usize = 5;
pub(crate) const NUM_LAYER: usize = 2;

#[rustfmt::skip]
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        layer!([
            [k!(Grave), k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(Kc5), k!(Kc6), k!(Kc7), k!(Kc8), k!(Kc9), k!(Kc0), k!(Minus), k!(Equal), k!(Backspace)],
            [k!(Tab), k!(Q), k!(W), k!(E), k!(R), k!(T), k!(Y), k!(U), k!(I), k!(O), k!(P), k!(LeftBracket), k!(RightBracket), k!(Backslash)],
            [k!(Escape), k!(A), k!(S), k!(D), k!(F), k!(G), k!(H), k!(J), k!(K), k!(L), k!(Semicolon), k!(Quote), a!(No), k!(Enter)],
            [k!(LShift), k!(Z), k!(X), k!(C), k!(V), k!(B), k!(N), k!(M), k!(Comma), k!(Dot), k!(Slash), a!(No), a!(No), k!(RShift)],
            [k!(LCtrl), k!(LGui), k!(LAlt), a!(No), a!(No), k!(Space), a!(No), a!(No), a!(No), mo!(1), k!(RAlt), a!(No), k!(RGui), k!(RCtrl)]
        ]),
        layer!([
            [k!(Grave), k!(F1), k!(F2), k!(F3), k!(F4), k!(F5), k!(F6), k!(F7), k!(F8), k!(F9), k!(F10), k!(F11), k!(F12), k!(Delete)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [k!(CapsLock), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), k!(Up)],
            [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), k!(Left), a!(No), k!(Down), k!(Right)]
        ]),
    ]
}

```

First of all, the keyboard matrix's basic info(number of rows, cols and layers) is defined as consts:

```rust
pub(crate) const COL: usize = 14;
pub(crate) const ROW: usize = 5;
pub(crate) const NUM_LAYER: usize = 2;
```

Then, the keymap is defined as a static 3-D matrix of `KeyAction`:

```rust
// You should define a function that returns the default keymap by yourself
pub const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    ...
}
```

A keymap in RMK is a 3-level hierarchy: layer - row - column. Each keymap is a slice of layers whose length is `NUM_LAYER`. Each layer is a slice of rows whose length is `ROW`, and each row is a slice of `KeyAction`s whose length is `COL`.

RMK provides a bunch of macros which simplify the keymap definition a lot. You can check all available macros in [RMK doc](https://docs.rs/rmk/latest/rmk/index.html#macros). For example, `layer!` macro is used to define a layer. `k!` macro is used to define a normal key in the keymap. If there is no actual key at a position, you can use `a!(No)` to represent `KeyAction::No`.
