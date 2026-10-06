# Special Characters and Unicode

RMK ultimately emits HID codes to the operating system. How these codes are interpreted to print a letter depends on the operating system and the keyboard layout setting. For example, pressing the key for `;` on a keyboard with an en-US layout will print `;`. But if you change that to de-DE, it will print `ö` instead.

This documentation and the `KeyCodes` assume an en-US layout.

## Issuing Specific Characters

Entering special characters usually requires a key combination, which depends on your operating system and chosen keyboard layout (setting in the OS). For example, in macOS with an en-US layout, you can define the following sequence to enter an `ä`:

```toml
[[behavior.macro.macros]]
operations = [
    { operation = "down", keycode = "LAlt" },
    { operation = "tap", keycode = "U" },
    { operation = "up", keycode = "LAlt" },
    { operation = "tap", keycode = "A" },
]
```

## Printing unicode

Each unicode symbol has an `code point` (aka alt-sequence) identifying it, usually depicted as `U+` and a hex number, like `U+2764` for ❤. This [wikipedia article](https://en.wikipedia.org/wiki/List_of_Unicode_characters) lists all unicode symbols.

Depending on your Operating System and Keyboard Layout you can enter a specific character by pressing a key combination, usually using the alt modifier.

If you are using Windows, follow [this description](https://altcodeunicode.com/how-to-use-alt-codes/) to enter unicode characters.

MacOS has a key layout called `Unicode Hex Input`, which is similar to en-US, but allows entering unicode alt sequences by holding alt pressed and entering the unicode number.

In rmk you can define the input sequence for printing a unicode symbol using [keyboard macros](./keyboard_macros.md).
