# Special Keys

RMK maps all [keys](https://docs.rs/rmk-types/latest/rmk_types/keycode/index.html) QMK does. However, at the time of writing, not all features are supported.

The following keys are supported (some further keys might work, but are not documented).

## Repeat/Again key

[Similar to QMK](https://docs.qmk.fm/features/repeat_key), pressing this key repeats the last key pressed. RMK binds this function both to the standard `Again` keycode and to a dedicated `Repeat` special key. Binding `Again` ensures better compatibility with Vial, which features the `Again` key as a dedicated key (unlike the `RepeatKey`, which doesn't exist in Vial). Although some old keyboards might have a key for `Again`, it is not used in modern operating systems anymore.

In QMK an `AlternativeRepeatKey` is supported. This functionality is not implemented in RMK.

## Caps Word

RMK includes `CapsWordToggle` (case-insensitive, no aliases). Caps word capitalizes letters (and turns `-` into `_`) until you press a key other than a letter, digit, `-`, Backspace or Delete, or after 5 seconds without one of those keys.
