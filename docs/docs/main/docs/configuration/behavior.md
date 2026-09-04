# Behavior

The `[behavior]` section contains configuration for how different keyboard actions should behave:

```toml
[behavior]
tri_layer = { upper = 1, lower = 2, adjust = 3 }

[behavior.sticky_key]
timeout = "1s"
activate_on_keypress = false
```

::: note Rust API only
`BehaviorConfig` has three fields with no `keyboard.toml` counterpart: `default_layer` (the base layer at startup), `tap` (`TapConfig`) and `mouse_key` (`MouseKeyConfig`, mouse-key acceleration; only its repeat intervals come from `[rmk].mouse_key_interval` and `[rmk].mouse_wheel_interval`). With `keyboard.toml` they keep their defaults; set them when you build `BehaviorConfig` in Rust.
:::

## Tri Layer

Tri-layer enables a third layer (often called `adjust`) automatically when two other layers (`upper` and `lower`) are both active.

You can enable Tri-Layer by specifying the `upper`, `lower` and `adjust` layers in the `tri_layer` sub-table:

```toml
[behavior.tri_layer]
upper = 1
lower = 2
adjust = 3
```

In this example, when both layers 1 (`upper`) and 2 (`lower`) are active, layer 3 (`adjust`) will also be enabled.

## Sticky Keys

Sticky Keys retain modifiers and layers, and support repeated modified HID keyboard keys. The three supported forms are:

- `SK(LShift)` for one or more modifiers
- `SK(MO(1))` for a layer
- `SK(Tab, [LAlt])` for one HID keyboard key with a bracketed modifier list

The tap-key form does not accept consumer, system-control, mouse, or nested actions. `SK(MO(n))` is the only Sticky layer form.

```toml
[behavior.sticky_key]
timeout = "1s"
activate_on_keypress = true
release_after_hold = "500ms"
max_repeat = 0
release_mode = "other_key_release"

[behavior.sticky_key.profiles.quick]
release_mode = "other_key_press | double_tap"

[behavior.sticky_key.profiles.alt_tab]
timeout = "5s"
max_repeat = 8
release_mode = "other_key_press | layer_exit"
```

Each field may be omitted. Named profiles inherit omitted fields from `[behavior.sticky_key]` and are selected with `@name`, for example `SK(LShift, @quick)` or `SK(Tab, [LAlt], @alt_tab)`.

Profile names are case-sensitive. An undefined profile name fails the build.

### Profile fields

| Field | Behavior |
| --- | --- |
| `timeout` | Releases a latch that has not reached a configured release trigger. The default is `1s`. The timer starts when the physical Sticky key is released and the effect becomes latched. Holding the key does not consume this time. |
| `activate_on_keypress` | Sends a pure modifier report as soon as its Sticky key is pressed. If false, another key pressed while the Sticky key remains down still receives the modifier. This field has no useful effect on Sticky layers or tap keys. |
| `release_after_hold` | For a modifier or layer held at least this long, releases the effect on physical key-up instead of latching it. A foreign key pressed during the hold keeps the effect active until the Sticky key is released. A shorter tap receives the full `timeout` from key-up. Pure modifiers need `activate_on_keypress = true` to appear in a modifier-only report while held. The default is disabled, and Sticky tap keys ignore this field. |
| `max_repeat` | Limits how many times `SK(key, [modifiers])` emits the same tap key in one retained sequence, including its first press. For example, `2` emits twice and the third press cancels without emitting. `0` means unlimited. Modifiers and layers ignore this field. |
| `release_mode` | Selects one or more release triggers. Join triggers with `|`. A configured value must name at least one trigger and replaces the shape's default. |

The default release mode depends on the action:

| Shape | Default | Effect |
| --- | --- | --- |
| `SK(LShift)` | `other_key_release` | The target key receives Shift through its release. Keys rolled before that release also receive Shift. |
| `SK(MO(1))` | `other_key_release` | The selected layer stays active through the target key's release. |
| `SK(Tab, [LAlt])` | `other_key_press` | Another non-modifier action releases the retained tap key before that action runs. |

Release triggers work as follows:

- `other_key_press` releases on another action's press. A triggering key is resolved while a latched modifier or layer is still active, then RMK balances the modifier report or layer state. A Sticky tap key releases its retained key and modifiers before the foreign action. Plain modifier actions do not release a Sticky tap key.
- `other_key_release` keeps the effect through another key's press and releases it on that key's release.
- `layer_enter` and `layer_exit` release only when a layer changes state. Activating an active layer or deactivating an inactive layer does not count.
- `double_tap` cancels the latch when the same Sticky source is pressed again. Pressing a different Sticky key does not trigger it.

Combo and Morse decisions can delay resolution of the consuming key. RMK claims a press-triggered Sticky latch when the physical press arrives, so its timeout cannot expire while that decision is pending.

### Composition

Sticky modifiers combine. Different physical keys and combo outputs retain separate identities even when they produce overlapping modifier masks. A Sticky modifier and one Sticky layer may coexist.

Pressing the same Sticky layer again refreshes it. Pressing a different Sticky layer releases and replaces the old layer. Sticky layers use RMK's normal boolean layer state, so the latest activation or deactivation command controls a layer shared with another action.

A Sticky tap key releases an active Sticky modifier and layer. Pressing a Sticky modifier or layer releases an active Sticky tap key, and pressing a different Sticky tap key replaces the first.

Releasing a physically held Sticky modifier immediately removes only the bits owned by that producer. Other held Sticky modifiers, ordinary modifiers, Caps Word Shift, `WM` or `SHIFTED` modifiers, layers, and held keys remain active. If another physical key is still down when the final Sticky modifier producer is released, that modifier entry ends instead of becoming a new latch. For accumulated modifiers, `release_after_hold` starts at the first producer press, while the most recently accepted producer supplies the active profile.

RMK tracks up to eight simultaneously held Sticky modifier producers. A directly pressed Sticky modifier and a combo that outputs one each use a slot, including producers with the same modifier mask. A press above this limit is ignored, as is its later release, so it cannot release any accepted producer. Releasing an accepted producer frees its slot.

### Compatibility settings

`OSM(modifiers)` and `OSL(layer)` are syntax aliases for default-profile `SK(modifiers)` and `SK(MO(layer))`. They can select a named profile as their final argument, for example `OSM(LShift, @quick)` and `OSL(2, @navigation)`.

The legacy `[behavior.one_shot]` and `[behavior.one_shot_modifiers]` tables remain accepted. A field in `[behavior.sticky_key]` wins when both forms configure the same default. Otherwise, legacy `timeout` and `activate_on_keypress` fill omitted default fields. Legacy `quick_release = true` selects `other_key_press` only for a default-profile pure modifier when the canonical default does not set `release_mode`. It affects `OSM(...)` and equivalent `SK(...)` syntax. Named profiles inherit the resolved default timeout and activation fields, but legacy `quick_release` does not change their release mode.

### Host tools and storage

VIA and Vial can represent only default-profile Sticky modifiers and Sticky layers through their standard one-shot keycodes. VIA layers are limited to 0 through 15. Named-profile Sticky actions and `SK(key, [modifiers])` have no VIA encoding and convert to `No` with a firmware warning. Editing or round-tripping those cells through Vial loses the action. Vial's "One Shot Timeout" setting changes only the canonical default Sticky timeout.

Rynk can read and write all three Sticky action shapes and their existing numeric profile indices. Its behavior endpoint exposes only the default Sticky timeout as `oneshot_timeout_ms`; it does not edit Sticky profile definitions or the other profile fields.

Storage persists Sticky actions with the keymap. Its behavior record persists only the canonical default timeout under the compatibility name `one_shot_timeout`. Firmware configuration supplies every other default-profile field and all named profiles.

## Combo

In the `combo` sub-table, you can configure the keyboard's combo key functionality. Combo allows you to define a group of keys that, when pressed simultaneously, will trigger a specific output action.

Combo configuration includes the following parameters:

- `timeout`: Defines the maximum time window for pressing all combo keys. If the time exceeds this, the combo key will not be triggered. The format is a string, which can be milliseconds (e.g. "200ms") or seconds (e.g. "1s"). Defaults to 50ms.
- `prior_idle_time`: An optional cooldown window after any key press before a combo can start recording. This helps prevent accidental combo triggers during fast typing. The format is a string (e.g. `"130ms"`). If not set, there is no idle check (equivalent to ZMK's `require-prior-idle-ms`).
- `combos`: An array containing all defined combos. Each combo configuration is an object containing the following attributes:
  - `actions`: An array of strings defining the keys that need to be pressed simultaneously to trigger the combo action.
  - `output`: A string defining the output action to be triggered when all keys in `actions` are pressed simultaneously.
  - `layer`: An optional parameter, a number, specifying which layer the combo is valid on. If not specified, the combo is valid on all layers.

Here is an example of combo configuration:

```toml
[behavior.combo]
timeout = "150ms"
prior_idle_time = "130ms"  # optional, prevents accidental triggers during fast typing
combos = [
  # Press J and K keys simultaneously to output Escape key
  { actions = ["J", "K"], output = "Escape" },
  # Press F and D keys simultaneously to output Tab key, but only valid on layer 0
  { actions = ["F", "D"], output = "Tab", layer = 0 },
  # Three-key combo, press A, S, and D keys to switch to layer 2
  { actions = ["A", "S", "D"], output = "TO(2)" }
]
```

## Macro

In the `macro` sub-table, you can configure the keyboard's macro functionality. Macros are explained in more detail in the [keyboard macros](./keymap_configuration/keyboard_macros.md) page.

Macro operations are defined with an `operation` and a `keycode`, `duration` or `text` field depending on the operation. Available operations are:

```toml
[[behavior.macro.macros]]
operations = [
  { operation = "down", keycode = "_" }, # [!code focus:5]
  { operation = "up", keycode = "_" },
  { operation = "tap", keycode = "_" },
  { operation = "delay", duration = "0ms" },
  { operation = "text", text = "foo" }
]
```

- `keycode` accepts a plain [keycode](./keymap_configuration/keycodes.md) or a supported action expression such as `WM(A, LCtrl)` or `PDF(1)`. Action expressions use the Vial extended encoding and require the `vial` feature; without it, the build fails. Firmware macros cannot emit Sticky actions (`SK`, `OSM`, or `OSL`).
- `duration` is at most 65024ms; longer delays fail the build.
- A macro cannot trigger another macro. `Macro(n)` inside a macro is ignored with a warning.

```toml
# Outputs "Hello"
[[behavior.macro.macros]]
operations = [
    { operation = "text", text = "Hello" }
]

# Outputs "Hello" with a 1 second delay after the first letter
[[behavior.macro.macros]]
operations = [
    { operation = "down", keycode = "LShift" },
    { operation = "tap", keycode = "H" },
    { operation = "up", keycode = "LShift" },
    { operation = "delay", duration = "1s" },
    { operation = "tap", keycode = "E" },
    { operation = "tap", keycode = "L" },
    { operation = "tap", keycode = "L" },
    { operation = "tap", keycode = "O" },
]
```

## Morse (and TapDance)

In the `morse` sub-table, you can configure the keyboard's morse functionality. Morse is a superset of the well-known [tap dance](https://docs.qmk.fm/features/tap_dance), enabling you to assign different actions to various combinations of taps and holds performed within a specific time window.

Morse keys are defined as a list under the `[behavior.morse]` section:

```toml
[behavior.morse]
morses = [
  # ... morse entries ...
]
```

RMK provides three methods for defining a Morse key.

### Define a morse key

#### 1. Vial-style Tap Dance

This method is fully compatible with Vial's Tap Dance, it defines four specific actions:

1. `tap`: The action to be triggered on the first tap. This is the default action when the key is tapped once.
2. `hold`: The action to be triggered when the key is held down (not tapped) beyond the tapping term.
3. `hold_after_tap`: The action to be triggered when the key is held down after being tapped once.
4. `double_tap`: The action to be triggered when the key is tapped twice within the tapping term.

Example:

```toml
[behavior.morse]
morses = [
  # A Vial-style tap dance key
  { tap = "F1", hold = "MO(1)", hold_after_tap = "MO(2)", double_tap = "F2" }
]
```

#### 2. Tap and Hold Arrays

This is an extended version of tap dance. It allows you to define sequences of actions for multiple taps and for holds that occur after a specific number of taps.

- `tap_actions`: An array of actions triggered by sequential taps. Each tap within the tapping term increments the tap count and triggers the corresponding action from the `tap_actions` array. For example, `tap_actions = ["F1", "F2", "F3"]` means a single tap triggers "F1", double tap triggers "F2", triple tap triggers "F3". Once the longest configured pattern is reached (the third tap here), the action fires immediately and the sequence ends; the next tap starts a new sequence.
- `hold_actions`: An array of actions triggered when the key is held, indexed by the number of taps _before_ the hold: the first entry is a plain hold, the second is a hold after one tap, and so on. For example, `hold_actions = ["MO(1)", "MO(2)", "MO(3)"]` means holding triggers "MO(1)", holding after one tap triggers "MO(2)", holding after two taps triggers "MO(3)".

Example:

```toml
[behavior.morse]
morses = [
  # A morse key defined with tap and hold action arrays
  { tap_actions = ["F1", "F2", "F3", "F4", "F5"], hold_actions = ["MO(1)", "MO(2)", "MO(3)", "MO(4)", "MO(5)"] }
]
```

#### 3. Full Morse Patterns

This is the most powerful method, allowing you to define actions based on [Morse code](https://en.wikipedia.org/wiki/Morse_code)-like patterns of taps and holds. This lets you assign a large number of actions to a single key

- `morse_actions`: A list of pattern-to-action mappings. The pattern is a tap/hold sequence, a tap is represented by a `.` or `0`, a hold is represented by a `_`, `-` or `1`. For example, the morse pattern of `C` can be described like this: `"-.-."` or `"_._."` or `"1010"`. The maximum length of the pattern is 15.

Example:

```toml
[behavior.morse]
morses = [
  # A morse key defined using full Morse patterns
  { morse_actions = [
        { pattern = ".-", action = "A" },
        { pattern = "-...", action = "B" },
        { pattern = "-.-.", action = "C" },
        { pattern = "-..", action = "D" },
    ], profile = "MRZ" },
]
```

::: warning
The three definition methods are mutually exclusive. For any single Morse key definition, you must choose only one of the following approaches:

- Full Morse: `morse_actions`
- Tap and Hold Arrays: `tap_actions` and/or `hold_actions`
- Vial-style: `tap`, `hold`, `hold_after_tap`, `double_tap`.

Mixing fields from different methods in the same definition is not allowed.
:::

### Profile

The `profile` of a morse key contains all tunable configurations of this morse key, such as behavior mode, timing configurations, etc.

::: tip

- `enable_flow_tap`: Enables HRM (Home Row Mod) mode. When enabled, the global `prior_idle_time` setting becomes functional. Defaults to `false`. Profiles may set this to override the global `[behavior.morse]` value; omitting it inherits the global value.
- `prior_idle_time`: _(global only)_ If the previous non-modifier key was pressed within this period before pressing the current tap-hold key, the tap action for the tap-hold behavior will be triggered. This parameter lives in `[behavior.morse]` (not in a per-key profile) and is effective only when `enable_flow_tap` is enabled for the key. Defaults to 120ms.
  :::

A profile contains the following fields:

- `unilateral_tap`: (Experimental) Enables unilateral tap mode. When enabled, tap action will be triggered when a key from "same" hand is pressed. In current experimental version, the "same" hand is calculated using the `<hand>`, which can be given in `layout.map`. This option is recommended to set to true when `enable_flow_tap` is set to true. In `normal_mode` the tap resolves when the same-hand key is pressed; in `permissive_hold` mode, when it is released. `hold_on_other_press` mode ignores this option, because the hold fires on the other key's press first.

- The morse mode, which can be set by enabling one of these:
  - `permissive_hold`: Enables permissive hold mode. When enabled, hold action will be triggered when a key is pressed and released during tap-hold decision. This option is recommended to set to true when `enable_flow_tap` is set to true.
  - `hold_on_other_press`: Enables hold-on-other-key-press mode. When enabled, hold action will be triggered immediately when any other key (including another tap-hold key) is pressed while a tap-hold key is being held. This provides faster modifier activation without waiting for the timeout. Defaults to `false`.
  - `normal_mode` : this is the default mode, when nor the `permissive_hold` nor the `hold_on_other_press` is set.

- `hold_timeout`: Defines the duration a tap-hold key must be pressed to determine hold behavior. If tap-hold key is released within this time, the key is recognized as a "tap". Holding it beyond this duration triggers the "hold" action when that hold pattern is final; if a longer configured morse pattern can still continue from the hold, RMK keeps the morse key unresolved until the sequence is completed. Defaults to 250ms. Maximum 8191ms (13-bit field).
- `gap_timeout`: Defines the duration a tap-hold key must be released to terminate a morse sequence. Buffered non-morse keys remain behind unresolved morse keys until the sequence resolves, preserving typing order during rollovers. Defaults to 250ms. Maximum 8191ms (13-bit field). Note that only morse and tap-dance needs this setting, simple tap-hold does not.
- `quick_tap_timeout`: If the same morse/tap-hold key is pressed again within this window after its last release, the tap action fires immediately on press and stays held while the key is held. This lets the OS auto-repeat the tap action instead of triggering the hold action. Disabled by default. Maximum 8191ms (13-bit field).
  - Setting `quick_tap_timeout = "0ms"` explicitly disables quick-tap for that profile, even if a non-zero global default is configured. This lets you opt out on a per-profile basis. Omitting the field entirely causes the profile to inherit the global default.
  - A re-press within the window resolves as a tap even if a `double_tap` action is configured, so double-tapping faster than `quick_tap_timeout` produces two taps instead of the `double_tap` action.

#### Default profile for Morse/TapDance/TapHold

In the `[behavior.morse]` sub-table you can configure the default profile. If there's no explicit profile applied to a morse key, default profile will be used.

The following are some examples for default profile setting:

```toml
# This default setting enables HRM with all tap-hold features
[behavior.morse]
enable_flow_tap = true
prior_idle_time = "120ms"
hold_on_other_press = true
hold_timeout = "250ms"
gap_timeout = "250ms"
```

```toml
# This default setting enables fast modifiers without HRM
[behavior.morse]
enable_flow_tap = false
hold_on_other_press = true
hold_timeout = "200ms"
gap_timeout = "200ms"
```

```toml
# This default setting is the most basic configuration
[behavior.morse]
enable_flow_tap = false
hold_timeout = "250ms"
gap_timeout = "250ms"
```

#### Per-key profiles for Morse, TapDance, Tap Hold fine tuning

In the `morse.profiles` sub-table you can define individual key profiles. Each profile has an associated name, which can be referred

- from the tap hold keys in the key map if the third optional parameter is filled:
  - `TH(key-tap, key-hold, <profile_name>)`,
  - `MT(key, modifier, <profile_name>)`,
  - `LT(n, key, <profile_name>)`
- the Morse keys may also have their per key profile overrides by setting the `profile` field.

The following examples are the typical default configurations:

```toml

[behavior.morse.profiles]
# This profile is recommended on the home row, when enable_flow_tap = true, and the hold action activates a layer or acts as a modifier (aka home row mod)
HRM = { unilateral_tap = true, permissive_hold = true, hold_timeout = "250ms", gap_timeout = "250ms" }

# This profile is recommended when the hold action activates a layer or acts as a modifier (without HRM) (for example thumb keys)
FH = { enable_flow_tap = false, hold_on_other_press = true, unilateral_tap = false, hold_timeout = "200ms", gap_timeout = "200ms" }

# This profile is recommended for "real" morse keys
MRZ = { normal_mode = true, unilateral_tap = false, hold_timeout = "200ms", gap_timeout = "200ms" }
```

Then you can reference the profile in layer config:

```toml
[[keymap.layer]]
keys = """
MT(A, LShift, HRM)
LT(1, A, FH)
TH(A, B, MRZ)
"""
```

### Global Configuration Limits

The following parameters in the `[rmk]` section control the resource allocation for the Morse feature:

- `morse_max_num`: The maximum number of Morse key you can create. (Default: 8, Range: 0-255)
- `morse_profile_max_num`: The capacity of the named profile table in `[behavior.morse.profiles]`. (Default: 16, Range: 0-255)
- `max_patterns_per_key`: The maximum number of individual patterns (like ".-") or actions that a single Morse key can contain. (Default: 8, Range: 4-32)

```toml
[rmk]
morse_max_num = 10  # To support up to 10 morse keys
max_patterns_per_key = 32  # To support up to 32 morse patterns per morse key
```

Note that the Vial-style method (using `tap`, `hold`, `hold_after_tap`, `double_tap`) needs at least 4 patterns. RMK raises `morse_max_num` and `max_patterns_per_key` automatically to fit the morse keys defined in `keyboard.toml`, so set them only to reserve room for keys added later (for example through Vial). A single key cannot hold more than 32 patterns; the build fails above that limit.

::: warning Vial Compatibility
Please note that while the firmware can handle all Morse configurations, Vial can only recognize and edit the four basic Vial-style actions. These correspond to the patterns for single tap (.), hold (-), double tap (..), and hold-after-tap (.-). More complex patterns defined using morse_actions or extended tap_actions will not be visible or editable in Vial.
:::

### Comprehensive Example

Here is a comprehensive example of morse configuration:

```toml
[rmk]
# Maximum number of morses keyboard can store (max 255)
morse_max_num = 9
# Maximum number of patterns a morse key can handle (max 32)
max_patterns_per_key = 32

[behavior.morse]
# default profile for morse, tap dance and tap-hold keys:
enable_flow_tap = true
prior_idle_time = "120ms"  # flow_tap needs this
hold_on_other_press = true
hold_timeout = "250ms"
gap_timeout = "250ms"

# list of morse (tap dance) keys:
morses = [
  # td(0): Function key that outputs F1 on tap, F2 on double tap, layer 1 on hold
  { tap = "F1", double_tap = "F2", hold = "MO(1)" },

  # td(1): Modifier key that outputs Shift on hold, Alt on hold after tap,
  { tap = "LCtrl", hold = "LShift", hold_after_tap = "LAlt" },

  # td(2): Navigation key that outputs Tab on tap, Escape on double tap, layer 2 on hold
  { tap = "Tab", hold = "MO(2)", double_tap = "Escape" },

  # td(3): Extended morse for function keys
  { tap_actions = ["F1", "F2", "F3", "F4", "F5"], hold_actions = ["MO(1)", "MO(2)", "MO(3)", "MO(4)", "MO(5)"] },

  # td(4): the morse ABC
  { morse_actions = [
      { pattern = ".-", action = "A" },
      { pattern = "-...", action = "B" },
      { pattern = "-.-.", action = "C" },
      { pattern = "-..", action = "D" },
      { pattern = ".", action = "E" },
      { pattern = "..-.", action = "F" },
      { pattern = "--.", action = "G" },
      { pattern = "....", action = "H" },
      { pattern = "..", action = "I" },
      { pattern = ".---", action = "J" },
      { pattern = "-.-", action = "K" },
      { pattern = ".-..", action = "L" },
      { pattern = "--", action = "M" },
      { pattern = "-.", action = "N" },
      { pattern = "---", action = "O"},
      { pattern = ".--.", action = "P" },
      { pattern = "--.-", action = "Q" },
      { pattern = ".-.", action = "R" },
      { pattern = "...", action = "S" },
      { pattern = "-", action = "T" },
      { pattern = "..-", action = "U" },
      { pattern = "...-", action = "V" },
      { pattern = ".--", action = "W" },
      { pattern = "-..-", action = "X" },
      { pattern = "-.--", action = "Y" },
      { pattern = "--..", action = "Z" }
    ], profile = "MRZ" }
]

# these can be used to override the default morse profile given in [behavior.morse]
[behavior.morse.profiles]
# for home row mod
HRM = { unilateral_tap = true, permissive_hold = true, hold_timeout = "250ms", gap_timeout = "250ms" }
# for "real" morse
MRZ = { normal_mode = true, unilateral_tap = false, hold_timeout = "200ms", gap_timeout = "200ms" }
# for "fast" modifiers (for example on thumb keys)
PN = { hold_on_other_press = true, unilateral_tap = false, hold_timeout = "250ms", gap_timeout = "250ms" }
```

### Using Morse(Tap Dance) in Keymaps

You can use both `Morse` and `TD` to represent a morse key in your keymap, you can reference it by its index (starting from 0):

```toml
[layout]
rows = 4
cols = 3
map = """
(0,0) (0,1) (0,2)
(1,0) (1,1) (1,2)
(2,0) (2,1) (2,2)
(3,0) (3,1) (3,2)
"""

[keymap]
# Layers 0 and 1 are defined below; layer 2 (referenced by LT(2, ...) and TG(2)) stays empty
layers = 3

[[keymap.layer]]
keys = """
A      B              C
TD(0)  TD(1)          TD(2)
LCtrl  MO(1)          LShift
OSL(1) LT(2, Kc9, PN) LM(1, LShift | LGui)
"""

[[keymap.layer]]
keys = """
_ TT(1) TG(2)
_ _     _
_ _     _
_ _     _
"""
```

Here `TD(0)`, `TD(1)`, and `TD(2)` reference morse dances by index, and the trailing `PN` in `LT(2, Kc9, PN)` names a morse profile (defined above). `keys` and `map` blocks hold data only.

## Fork

In the `fork` sub-table, you can configure the keyboard's state-based key fork functionality. Forks allow you to define a trigger key and condition-dependent possible replacement keys. When the trigger key is pressed, the condition is checked by the following rule: If any of the `match_any` states are active AND none of the `match_none` states are active, the trigger key will be replaced with positive_output; otherwise, it will be replaced with the negative_output. By default, the modifiers listed in `match_any` will be suppressed, including Sticky modifiers and their `OSM` aliases, for the time the replacement key action is executed. However, with `kept_modifiers` some of them can be kept instead of automatic suppression.

Fork configuration includes the following parameters:

- `forks`: An array containing all defined forks. Each fork configuration is an object containing the following attributes:
  - `trigger`: Defines the triggering key.
  - `negative_output`: A string defining the output action to be triggered when the conditions are not met
  - `positive_output`: A string defining the output action to be triggered when the conditions are met
  - `match_any`: A string defining a combination of modifier keys, lock LEDs, mouse buttons (optional)
  - `match_none`: A string defining a combination of modifier keys, lock LEDs, mouse buttons (optional)
  - `kept_modifiers`: A string defining a combination of modifier keys, which should not be 'suppressed' from the keyboard state for the time the replacement action is executed (optional)
  - `bindable`: Enables the evaluation of not yet triggered forks on the output of this fork to further manipulate the output. Advanced use cases can be solved using this option (optional)

Each fork must set at least one of `match_any` and `match_none`; the build fails otherwise.

For `match_any`, `match_none` the legal values are listed below (many values may be combined with "|"):

- `LShift`, `LCtrl`, `LAlt`, `LGui`, `RShift`, `RCtrl`, `RAlt`, `RGui` (these include explicitly held and Sticky modifiers)
- `CapsLock`, `ScrollLock`, `NumLock`, `Compose`, `Kana`
- `MouseBtn1` .. `MouseBtn8`

Here is a sample of fork configuration with random examples:

```toml
[behavior.fork]
forks = [
  # Shift + '.' output ':' key
  { trigger = "Dot", negative_output = "Dot", positive_output = "WM(Semicolon, LShift)", match_any = "LShift|RShift" },

  # Shift + ',' output ';' key but only if no Alt is pressed
  { trigger = "Comma", negative_output = "Comma", positive_output = "Semicolon", match_any = "LShift|RShift", match_none = "LAlt|RAlt" },

  # left bracket outputs by default '{', with shifts pressed outputs '['
  { trigger = "LeftBracket", negative_output = "WM(LeftBracket, LShift)", positive_output = "LeftBracket", match_any = "LShift|RShift" },

  # Flip the effect of shift on 'x'/'X'
  { trigger = "X", negative_output = "WM(X, LShift)", positive_output = "X", match_any = "LShift|RShift" },

  # F24 usually outputs 'a', except when Left Shift or Ctrl pressed, in that case triggers a macro
  { trigger = "F24", negative_output = "A", positive_output = "Macro1", match_any = "LShift|LCtrl" },

  # Swap Z and Y keys if MouseBtn1 is pressed (on the keyboard) (Note that these must not be bindable to avoid infinite fork loops!)
  { trigger = "Y", negative_output = "Y", positive_output = "Z", match_any = "MouseBtn1", bindable = false },
  { trigger = "Z", negative_output = "Z", positive_output = "Y", match_any = "MouseBtn1", bindable = false },

  # Shift + Backspace output Delete key (inside a layer tap/hold)
  { trigger = "LT(2, Backspace)", negative_output = "LT(2, Backspace)", positive_output = "LT(2, Delete)", match_any = "LShift|RShift" },

  # Ctrl + play/pause will send next track. MediaPlayPause -> MediaNextTrack
  # Ctrl + Shift + play/pause will send previous track. MediaPlayPause -> MediaPrevTrack
  # Alt + play/pause will send volume up. MediaPlayPause -> AudioVolUp
  # Alt + Shift + play/pause will send volume down. MediaPlayPause -> AudioVolDown
  # Ctrl + Alt + play/pause will send brightness up. MediaPlayPause -> BrightnessUp
  # Ctrl + Alt + Shift + play/pause will send brightness down. MediaPlayPause -> BrightnessDown
  # ( Note that the trigger and immediate trigger keys of the fork chain could be 'virtual keys',
  #   which will never output, like F23, but here multiple overrides demonstrated.)
    { trigger = "MediaPlayPause", negative_output = "MediaPlayPause", positive_output = "MediaNextTrack", match_any = "LCtrl|RCtrl", bindable = true },
  { trigger = "MediaNextTrack", negative_output = "MediaNextTrack", positive_output = "BrightnessUp", match_any = "LAlt|RAlt", bindable = true },
  { trigger = "BrightnessUp", negative_output = "BrightnessUp", positive_output = "BrightnessDown", match_any = "LShift|RShift", bindable = false },
  { trigger = "MediaNextTrack", negative_output = "MediaNextTrack", positive_output = "MediaPrevTrack", match_any = "LShift|RShift", match_none = "LAlt|RAlt", bindable = false},
  { trigger = "MediaPlayPause", negative_output = "MediaPlayPause", positive_output = "AudioVolUp", match_any = "LAlt|RAlt", match_none = "LCtrl|RCtrl", bindable = true },
  { trigger = "AudioVolUp", negative_output = "AudioVolUp", positive_output = "AudioVolDown", match_any = "LShift|RShift", match_none = "LCtrl|RCtrl", bindable = false }
]
```

Please note that the processing of forks happens after combos and before others, so the trigger key must be the one listed in your keymap (or combo output). For example if `LT(2, Backspace)` is in your keymap, then `trigger = "Backspace"` will NOT work, you should "replace" the full key and use `trigger = "LT(2, Backspace)"` instead, like in the example above. You may want to include `F24` or similar dummy keys in your keymap, and use them as trigger for your pre-configured forks, such as Shift/CapsLock dependent macros to enter unicode characters of your language.

Vial does not support fork configuration yet.

## Auto Mouse Layer

`[[behavior.auto_mouse_layer]]` is an array of entries. Each entry automatically activates a layer when X/Y cursor motion from a pointing device (e.g., PMW3610, trackball) is detected, and deactivates it after a `timeout` of inactivity. Scroll-only events do not trigger the layer.

When multiple pointing devices are present, set `device_id` on each entry to target the device. An entry without `device_id` acts as a fallback for any device not covered by a more specific entry — leave `device_id` unset on a single entry to use one shared configuration for every device.

Example configuration:

```toml
# Default for every pointing device.
[[behavior.auto_mouse_layer]]
target_layer = 3
timeout = "600ms"
threshold = 2

# Override for the second pointing device (device_id = 1).
[[behavior.auto_mouse_layer]]
device_id = 1
target_layer = 4
timeout = "500ms"
threshold = 5

# Immediate deactivation on non-mouse key press, but keep the layer while
# modifiers (Ctrl/Shift/Alt/Gui) are held so users can Ctrl-click, etc.
[[behavior.auto_mouse_layer]]
target_layer = 5
deactivate_on_key = true
extra_mouse_keys = ["LCtrl", "LShift", "LAlt", "LGui"]

# Extend the timeout on any non-deactivating key press so users can continue clicking without cursor movement.
[[behavior.auto_mouse_layer]]
target_layer = 6
deactivate_on_key = true
extra_mouse_keys = ["LCtrl", "LShift", "LAlt", "LGui"]
reset_timeout_on_key = true

# Required when using deactivate_on_key / reset_timeout_on_key (defaults to 0).
[event.action]
subs = 1
```

| Field                  | Type             | Default   | Description                                                                                                                                                                                                                                                              |
| ---------------------- | ---------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `device_id`            | integer          | —         | Pointing device id this entry applies to. Omit for a fallback that matches any device not covered by another entry. At most one fallback (and at most one entry per `device_id`) is allowed.                                                                             |
| `target_layer`         | integer          | —         | Layer index to activate (must be `< [keymap].layers`).                                                                                                                                                                                                                   |
| `timeout`              | string           | `"500ms"` | Inactivity duration before deactivation (e.g., `"600ms"`, `"2s"`).                                                                                                                                                                                                       |
| `threshold`            | integer          | `1`       | Minimum absolute X/Y delta to trigger motion (`>= 1`). Increase to filter sensor noise.                                                                                                                                                                                  |
| `deactivate_on_key`    | bool             | `false`   | When `true`, pressing any non-mouse key immediately deactivates `target_layer` (ignoring `timeout`). Mouse HID keys and keys listed in `extra_mouse_keys` do NOT trigger deactivation. Keys are classified by their **resolved** keycode; see the limitation note below. |
| `extra_mouse_keys`     | array of strings | `[]`      | Extra keycodes (e.g. `"LCtrl"`, `"Space"`) treated like mouse keys for the purpose of `deactivate_on_key`.                                                                                                                                                               |
| `reset_timeout_on_key` | bool             | `false`   | When `true`, key presses that do NOT deactivate `target_layer` push the `timeout` deadline forward (reset it to _now + `timeout`_). When `deactivate_on_key` is `false`, every key press extends the timeout.                                                            |

::: warning
Prefer a dedicated layer that is not bound to any manual keys (like `MO` or `TG`). The auto-mouse task releases its ownership when keyboard-driven changes deactivate the layer, so transient overlap is handled cleanly. Layer state is still a single boolean, however, so pressing `TG(target_layer)` while auto-mouse is active toggles the layer off instead of pinning it on.

Entries that share the same `target_layer` cooperate: the layer stays active until the last device stops moving, so per-device `timeout`/`threshold` differences on a shared layer are safe.
:::

::: warning Limitation: keys that cannot be classified

Some keys cannot be classified; they never trigger immediate deactivation (only `timeout` clears the layer) and extend the deadline when `reset_timeout_on_key` is set:

- **Keys that emit no classifiable keycode**: layer keys (`MO`, `TG`, `TO`, `DF`, `TT`, `LM`, ...), Sticky actions (`SK`, including the `OSM` and `OSL` aliases), user keys, and keyboard control keys (bootloader, reboot, ...).
- **Macros**: keycodes emitted while a macro runs bypass action resolution; the trigger key itself is also unclassifiable.
- **`Again` / `Repeat`**: the repeated keycode is unknown at classification time.
- **`GraveEscape`**: resolves to Escape or Grave after classification.

:::

::: note Event Configuration

- **Subscriber Slots**: Increment `[event.pointing].subs` and `[event.layer_change].subs` by `1` each in your `keyboard.toml` to reserve slots for this task. If any entry uses `deactivate_on_key` or `reset_timeout_on_key`, also set `[event.action].subs` to `1` (it defaults to `0`), otherwise the build fails with a validation error. See [Event Configuration](./event.md).
- **Buffer Size**: If pointing events are dropped under high-frequency input, increase `[event.pointing].channel_size` (default `8`). `[event.layer_change].channel_size` defaults to `1` and only needs raising if you burst many layer changes faster than subscribers consume them.
  :::

::: note Rust API
Configure the layer via `BehaviorConfig` and run the helper future alongside your other keyboard tasks. Subscriber slots are resolved from `keyboard.toml`'s `[event]` section at build time, so point `KEYBOARD_TOML_PATH` (set in `.cargo/config.toml`) to a `keyboard.toml` and increment `[event.pointing].subs` and `[event.layer_change].subs` by `1` there as well. When using `deactivate_on_key` / `reset_timeout_on_key`, also set `[event.action].subs` to `1` (it defaults to `0`) in that file. Otherwise the firmware panics at startup.

The number of entries defaults to `2` without a `keyboard.toml`; with one it is auto-derived from `[[behavior.auto_mouse_layer]]` (`0` when absent), so set `[rmk].auto_mouse_layer_max_num` explicitly to override. `extra_mouse_keys` is a `&'static [KeyCode]`, so its length has no cap.

```rust
use embassy_time::Duration;
use rmk::config::{AutoMouseLayerConfig, BehaviorConfig};
use rmk::heapless::Vec;

// Configure the auto mouse layer. Each entry targets either a specific
// `device_id` or, when `device_id` is `None`, acts as a fallback for any
// device not covered by another entry.
let auto_mouse_layer = Vec::from_iter([
    AutoMouseLayerConfig::new(
        None,                       // fallback for every device
        3,                          // target_layer index
        Duration::from_millis(600), // timeout duration
        2,                          // threshold
    ),
    AutoMouseLayerConfig::new(
        Some(1),                    // device_id == 1
        4,
        Duration::from_millis(500),
        5,
    ),
    // Immediate deactivation on non-mouse keypress, with modifiers exempted:
    AutoMouseLayerConfig::new(None, 5, Duration::from_millis(600), 1)
        .with_deactivate_on_key(&[
            rmk::types::keycode::KeyCode::Hid(rmk::types::keycode::HidKeyCode::LCtrl),
            rmk::types::keycode::KeyCode::Hid(rmk::types::keycode::HidKeyCode::LShift),
        ]),
    // Extend the timeout on any non-deactivating key press so the layer stays
    // alive while the user is still interacting even without cursor motion.
    AutoMouseLayerConfig::new(None, 6, Duration::from_millis(600), 1)
        .with_deactivate_on_key(&[
            rmk::types::keycode::KeyCode::Hid(rmk::types::keycode::HidKeyCode::LCtrl),
        ])
        .with_reset_timeout_on_key(),
]);
let behavior_config = BehaviorConfig {
    auto_mouse_layer,
    ..Default::default()
};

// include it in `run_all!` alongside the rest:
let mut auto_mouse_layer = rmk::AutoMouseLayerRunner::new(&keymap);
run_all!(
    matrix,
    keyboard,
    // ...other runnables
    auto_mouse_layer,
).await;
```

:::
