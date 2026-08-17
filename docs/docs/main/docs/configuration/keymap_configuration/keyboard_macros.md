# Keyboard Macros

RMK supports keyboard macros: Pressing a trigger to execute a sequence of keypresses.

This can be configured via Vial, the toml configuration file, or Rust.

## Macro operations

The following operations, coming from Vial, can be used to form a macro sequence. They are in `rmk::keyboard_macros`:

### Text(HidKeyCode, bool)

Execute a key press from any available HID keycode. The boolean flags if the key should be pressed with the shift modifier.

Note that other modifiers pressed outside of a sequence with `Text` are disabled.

### Tap(HidKeyCode)

Presses and releases a key. Modifiers pressed outside of a macro sequence are considered as well. If you don't need this prefer `Text(HidKeyCode, bool)` above, as the resulting macro is 3 times smaller in size.

### Press(HidKeyCode)

Press (and hold) a keycode. Useful for modifier keys.

### Release(HidKeyCode)

Release (a formerly pressed) keycode. Useful for modifier keys.

### Delay(u16)

Wait the given time in ms before executing the next macro operation.

### End

This marks the end of a macro sequence. Don't use it: The code removes all occurrences and adds one marker to the end of every sequence to be sure the sequences are terminated correctly.

With the `vial` feature enabled there are additionally `TapAction`, `PressAction`, and `ReleaseAction` operations, which tap/press/release an extended (non-HID) `Action` such as a Bluetooth-profile key.

## Configure a macro sequence

### Via the configuration file

See [macro](../behavior#macro) section under `behavior`

### Via Rust

A new field `keyboard_macros` has been added to the `BehaviorConfig` struct. Within it a field `macro_sequences` has to be set. This is in binary format (`[u8]`) and can only be as long as `MACRO_SPACE_SIZE`, which defaults to 256 and can be changed via `macro_space_size` in the `[rmk]` section of `keyboard.toml`.

The maximum number of Macros depends on the length of the sequences. Each `Text` operation takes 1 byte, `Tap`/`Press`/`Release` take 3 bytes, `Delay` and `TapAction`/`PressAction`/`ReleaseAction` take 4 bytes, and every macro ends with a 1-byte terminator.

If your sequences don't fit into `MACRO_SPACE_SIZE`, `define_macro_sequences` panics with "Too many Macro Operations!".

There are two helper functions to define macro sequences:

1. `define_macro_sequences(&[heapless::Vec<MacroOperation, MACRO_SPACE_SIZE>])` You can use it this way:

```rust
pub(crate) fn get_macro_sequences() -> [u8; MACRO_SPACE_SIZE] {
    define_macro_sequences(&[
        Vec::from_slice(&[
            MacroOperation::Text(HidKeyCode::H, true),
            MacroOperation::Text(HidKeyCode::E, false),
            MacroOperation::Text(HidKeyCode::L, false),
            MacroOperation::Text(HidKeyCode::L, false),
            MacroOperation::Text(HidKeyCode::O, false),
        ])
        .expect("too many elements"),
        Vec::from_slice(&[
            MacroOperation::Press(HidKeyCode::LShift),
            MacroOperation::Tap(HidKeyCode::W),
            MacroOperation::Release(HidKeyCode::LShift),
            MacroOperation::Tap(HidKeyCode::O),
            MacroOperation::Tap(HidKeyCode::R),
            MacroOperation::Tap(HidKeyCode::L),
            MacroOperation::Tap(HidKeyCode::D),
        ])
        .expect("too many elements"),
    ])
}
```

This code defines two macro sequences which produce "Hello" and "World". (As mentioned above prefer the first Macro for text only output. The first macro sequence is 6 bytes long, the second 22 bytes.)

For text output there is a convenience function: `to_macro_sequence(text: &str) -> heapless::Vec<MacroOperation, MACRO_SPACE_SIZE>`.

This function converts a `&str` into a sequence of `MacroOperation::Text`. The above example would be:

```rust
pub(crate) fn get_macro_sequences() -> [u8; MACRO_SPACE_SIZE] {
    define_macro_sequences(&[
        to_macro_sequence("Hello"),
        to_macro_sequence("World"),
    ])
}
```

(With the improvement that the `Text` macro operation is used in both cases.)

Note that you are still limited to the ascii characters defined as `HidKeyCode`s. For example, you can't enter a German Umlaut (`ü`) or unicode directly with a `HidKeyCode` binding. If you enter an illegal character it will be converted to `No` (nothing is typed).

Entering these special characters usually require a key combination which depends on your operating system and chosen keyboard layout (setting in the OS). For example, in MacOS with a en-US layout you can define the following sequence to enter an `ö`:

```rust
pub(crate) fn get_macro_sequences() -> [u8; MACRO_SPACE_SIZE] {
    define_macro_sequences(&[
        Vec::from_slice(&[
            MacroOperation::Press(HidKeyCode::LAlt),
            MacroOperation::Tap(HidKeyCode::U),
            MacroOperation::Release(HidKeyCode::LAlt),
            MacroOperation::Tap(HidKeyCode::O),
        ])
        .expect("too many elements"),
    ])
}
```

## Triggering a macro

### Binding

A macro can be triggered in two ways:

1. Using the macro shortcuts in keymap configuration (e.g., `Macro(0)` - `Macro(255)` in toml config, or `macros!(0)` - `macros!(255)` in Rust).
2. Using the `Action::TriggerMacro(index)`, where index can be any number between 0~255. If the total number of macro sequences is less than the index passed, nothing is executed (and an error "Macro not found" is logged). Remember that the index starts at `0`.
3. Defined macro sequences are automatically bound to a sequence: The first macro sequence defined is executed when triggering `Macro(0)` and `Action::TriggerMacro(0)`.

There is no difference using either: `macros!(n)` simply expands to `KeyAction::Single(Action::TriggerMacro(n))`.

### Combining

Both macro triggers can be used anywhere, where a `KeyCode` or an `Action` can be assigned.

As the only `Action` taking a `KeyCode` is `Action::Key`, combining with `Action`s is limited.

#### With `KeyAction`

You can combine the trigger with any `KeyAction`, like layer-taps, hold-taps, etc.

For example:

```rust
// Trigger macro(1) when tapping and switch to layer 1 when holding
// (the third field selects the morse profile; u8::MAX = default profile)
KeyAction::TapHold(Action::TriggerMacro(1), Action::LayerOn(1), u8::MAX)
```

Probably you most likely will need

```rust
macros!(0)
```

or

```rust
KeyAction::Single(Action::TriggerMacro(0))
```

#### With `Combo` (chording)

Combining with Combo allows for a quite powerful feature: Chording. Chording comes for the courtroom stenography and has its name from playing chords, like on a guitar. Chording is pressing a few letters to emit multiple letters.

Thus, one can press only the beginning of a word to write the whole word. For example, pressing `T` & `Y` could write `type`, pressing `T` & `Y`& `G` could write `typing`. If you want to implement this behavior we recommend using an extra layer, so rolling over `T` and `Y` will not accidentally execute the macro, but only when a layer toggle key is pressed as well.

This is the configuration for the above example, assuming `1` is the chording layer:

```rust
    use rmk::keyboard::combo::{Combo, ComboConfig};

    define_macro_sequences(&[
        to_macro_sequence("type"),
        to_macro_sequence("typing"),
    ])

    // `combos` is a `[Option<Combo>; COMBO_MAX_NUM]` array; unused slots stay `None`.
    // The default timeout is 50 ms.
    let mut combo_config = CombosConfig::default();
    combo_config.combos[0] = Some(Combo::new(ComboConfig::new([k!(T), k!(Y)], macros!(0), Some(1))));
    combo_config.combos[1] = Some(Combo::new(ComboConfig::new(
        [k!(T), k!(Y), k!(G)],
        KeyAction::Single(Action::TriggerMacro(1)),
        Some(1),
    )));
```

(`Action::TriggerMacro(1)` was used for demonstration only. Using `macros!(1)` is recommended to keep it brief.)

Note that instead of having a second macro for all verbs (normal and `ing` form) you can define a macro which converts a word to the `ing` form:

```rust
    define_macro_sequences(&[
        to_macro_sequence("type"),
        Vec::from_slice(&[
            MacroOperation::Press(HidKeyCode::Backspace),
            MacroOperation::Text(HidKeyCode::I, false),
            MacroOperation::Text(HidKeyCode::N, false),
            MacroOperation::Text(HidKeyCode::G, false),
        ])
        .expect("too many elements"),
    ])

    let mut combo_config = CombosConfig::default();
    combo_config.combos[0] = Some(Combo::new(ComboConfig::new([k!(T), k!(Y)], macros!(0), Some(1))));
    combo_config.combos[1] = Some(Combo::new(ComboConfig::new(
        [k!(G)],
        KeyAction::Single(Action::TriggerMacro(1)),
        Some(1),
    )));
```

With the configuration above pressing `T` & `Y` writes `type` and pressing `G` changes it to `typing`.

### With forks

You can use macro triggers in forks as well.

This is how you can trigger `hello` and `Hello` with pressing shift:

```rust
pub(crate) fn get_macro_sequences() -> [u8; MACRO_SPACE_SIZE] {
    define_macro_sequences(&[
        to_macro_sequence("hello"),
        to_macro_sequence("Hello"),
    ])
}
pub(crate) fn get_forks() -> ForksConfig {
    ForksConfig {
        forks: Vec::from_slice(&[
            Fork::new(
                macros!(0),
                macros!(0),
                macros!(1),
                StateBits::new_from(
                    ModifierCombination::LSHIFT,
                    LedIndicator::default(),
                    MouseButtons::default(),
                ),
                StateBits::default(),
                ModifierCombination::default(),
                false,
            ),
        ])
        .expect("Some fork is not valid"),
    }
}
```

## Tips

### Small and capital version of a word

If you want to spell a macro in small letters, but occationally with the first letter capitalized, you can do so in the following way:

For example, you might want to use a combo for the rare letter `q`. And as this letter mostly comes as `qu` you want to use a macro for that.

Thus, implement the macro:

```rust
pub(crate) fn get_macro_sequences() -> [u8; MACRO_SPACE_SIZE] {
    define_macro_sequences(&[
        Vec::from_slice(&[
            MacroOperation::Text(HidKeyCode::Q, false),
            MacroOperation::Text(HidKeyCode::U, false),
        ])
        .expect("too many elements"),
    ])
}
```

When you press `shift` and use `MacroOperation::Text`, like in the code above, no letter gets capitalized (outputs `qu`). Remember that `MacroOperation::Text` ignores all modifiers not being part of the sequence. `MacroOperation::Tap` doesn't, thus you can use `MacroOperation::Tap` for the first letter, and `MacroOperation::Text` for the following letters, to capitalize the first letter only.

```rust
pub(crate) fn get_macro_sequences() -> [u8; MACRO_SPACE_SIZE] {
    define_macro_sequences(&[
        Vec::from_slice(&[
            MacroOperation::Tap(HidKeyCode::Q),
            MacroOperation::Text(HidKeyCode::U, false),
        ])
        .expect("too many elements"),
    ])
}
```
