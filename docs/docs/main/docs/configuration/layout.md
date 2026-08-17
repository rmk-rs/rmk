# Layout

## Overview

RMK splits the keyboard description into three sections:

- `[matrix]` — the **electrical** wiring: which GPIO pins form the key matrix. See [matrix configuration](./keyboard_matrix#matrix-configuration).
- `[layout]` — the **physical** arrangement: the grid size (`rows`/`cols`), the `map` of key positions, and the shape of the rendered layout.
- `[keymap]` — the **logical** behavior: what each key does, across however many layers you define.

Here is a complete config for a 2×2 macropad: `[layout]` says where each key sits, `[keymap]` says what it does.

```toml
[layout]
rows = 2
cols = 2
map = """
(0,0) (0,1)
(1,0) (1,1)
"""

[[keymap.layer]]
keys = """
A B
C D
"""
```

Both strings are read in the same order, so the key at matrix position `(0,0)` is `A`, `(0,1)` is `B`, and so on. If your keyboard already has a [KLE](http://www.keyboard-layout-editor.com/) layout or a Vial `vial.json`, the `[layout]` section can be generated from it — see [converting from KLE or Vial](#converting-from-kle-or-vial).

## The layout map

`[layout].map` places your keys in the order you want to define them. **The order matters**: every `[[keymap.layer]]` reads its keys in this same order, so the _n_-th key in the map is the _n_-th key on each layer.

Each item in the map is one of:

| Item                       | Meaning                                                                                                                          |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `(row, col, hand, @shape)` | a key at matrix position `(row, col)`, with an optional [hand marker](#assigning-a-hand-to-each-key) and render [shape](#shapes) |
| `(e, id)`                  | a rotary [encoder](#encoders)                                                                                                    |
| `[n]`                      | a horizontal [gap](#gaps-and-row-steps) of `n` key-units                                                                         |
| `[y=n]`                    | an extra vertical [step](#gaps-and-row-steps) before the next row                                                                |
| newline                    | a row break                                                                                                                      |

`hand` and `@shape` are independent and optional — include either, both, or neither. When both appear, `hand` comes first, so `(row, col)`, `(row, col, hand)`, `(row, col, @shape)`, and `(row, col, hand, @shape)` are all valid. (The triple quotes `"""..."""` mark a multi-line string.)

The `(row, col)` coordinates use zero-based indexing and refer to a position in the "electronic matrix" of your keyboard. As shown in [matrix configuration](./keyboard_matrix#matrix-configuration), even direct-pin keyboards are represented as a matrix. For split keyboards, the coordinates refer to the "big unified matrix" that spans all split parts. This lets non-rectangular matrices be laid out intuitively.

The `map` and `[[keymap.layer]].keys` strings hold data only — they don't support inline comments. Put any annotations in normal TOML `#` comments outside the `"""…"""` block.

```toml
# simple numpad example:
# ┌───┬───┬───┬───┐
# │NUM│ / │ * │ - │ <-- row 0, col 0..4
# ├───┼───┼───┼───┤
# │ 7 │ 8 │ 9 │   │
# ├───┼───┼───┤ + │
# │ 4 │ 5 │ 6 │   │
# ├───┼───┼───┼───┤
# │ 1 │ 2 │ 3 │ E │
# ├───┴───┼───┤ N │
# │   0   │ . │ T │
# └───────┴───┴───┘
[layout]
rows = 5
cols = 4
map = """
(0,0) (0,1) (0,2) (0,3)
(1,0) (1,1) (1,2) (1,3,@2u_tall)
(2,0) (2,1) (2,2)
(3,0) (3,1) (3,2) (3,3,@2u_tall)
    (4,0,@2u)    (4,1)
"""

[keymap]
layers = 3
```

And a split ortholinear example, with L/R hand information filled in:

```toml
[layout]
rows = 4
cols = 10
map = """
(0, 0, L)  (0, 1, L)  (0, 2, L)  (0, 3, L)  (0, 4, L)    (0, 5, R)  (0, 6, R)  (0, 7, R)  (0, 8, R)  (0, 9, R)
(1, 0, L)  (1, 1, L)  (1, 2, L)  (1, 3, L)  (1, 4, L)    (1, 5, R)  (1, 6, R)  (1, 7, R)  (1, 8, R)  (1, 9, R)
(2, 0, L)  (2, 1, L)  (2, 2, L)  (2, 3, L)  (2, 4, L)    (2, 5, R)  (2, 6, R)  (2, 7, R)  (2, 8, R)  (2, 9, R)
                                 (3, 3, L)  (3, 4, L)    (3, 5, R)  (3, 6, R)
"""

[keymap]
layers = 3
```

Whitespace and line breaks between items are free to vary, but keeping an arrangement that mirrors the real keyboard makes the file easier to read.

## Assigning a hand to each key

The optional `hand` marker tags a key as belonging to the left or right hand. It is used only when `unilateral_tap = true` (see [behavior](./behavior#per-key-profiles-for-morse-tapdance-tap-hold-fine-tuning)); otherwise it is ignored.

- `L` (also `LH`, `Left`) — left hand
- `R` (also `RH`, `Right`) — right hand
- `*` (also `Bilateral`) — bilateral; treated as the opposite hand no matter which hand's modifier was held

Hand names are case-insensitive. The marker is the third element of the tuple, e.g. `(0, 0, L)`. See the split ortho example above for a full map with hands filled in.

## Rendered layout

Everything in this section changes only how your keyboard is _drawn_ in editors like Vial and Rynk — never what a key does. RMK compiles the rendered layout into a compact blob that the firmware streams to the host on request; the firmware itself never reads it. You can skip this section entirely: a bare `(row, col)` renders as a plain 1u key.

### Shapes

A `@shape` suffix sets a key's size, offset, and rotation. RMK ships a set of stock shapes:

- Wider keys, 1u tall: `@1.25u`, `@1.5u`, `@1.75u`, `@2u`, `@2.25u`, `@2.75u`, `@3u`, `@6.25u`, `@7u`
- `@2u_tall` — 1u wide, 2u tall (the numpad `+` and `Enter`)
- `@stepped_caps` — a 1.75u Caps Lock
- `@iso_enter` — the L-shaped ISO Enter
- `@bae` — a "big-ass Enter" (L-shaped)

The numpad example above uses `@2u_tall` for the 2u-tall `+` and `Enter` and `@2u` for the 2u-wide `0`, which is why it renders with those key sizes instead of a flat grid.

Define your own shapes under `[layout.shapes]`:

```toml
[layout.shapes]
lsft_iso = { w = 1.25 }
tilted   = { r = 15.0 }             # rotated 15° clockwise
isoenter = { w = 1.25, h = 2.0, y = -1.0, w2 = 1.5, h2 = 1.0, x2 = -0.125, y2 = -0.5 }
```

Every field is optional:

- `w`, `h` — width and height in key-units (default `1.0`)
- `x`, `y` — nudge the cap from its default position, in key-units (default `0.0`)
- `r` — rotation in degrees, clockwise (default `0.0`)
- `w2`, `h2`, `x2`, `y2` — an optional second rectangle, for L-shaped caps like the ISO Enter. The second rectangle is drawn only when `w2` is set (it's the trigger); the other three then default to `h2 = 1.0`, `x2 = 0.0`, `y2 = 0.0`.

A custom shape whose name matches a stock shape overrides it. A `@name` that isn't defined anywhere fails the build.

### Gaps and row steps

Two bracketed items fine-tune spacing inside the `map`:

- `[n]` — insert a horizontal gap of `n` key-units before the next key, e.g. the space between the halves of a split board.
- `[y=n]` — add `n` key-units to the vertical step before the next row (may be negative). Handy for staggered thumb clusters.

### Rotated clusters

A `[r=deg@(x,y)]` marker rotates every key and encoder after it by `deg` degrees clockwise about the point `(x, y)`, until the next `[r=...]` marker. Keys inside the region are laid out flat, exactly like unrotated keys — the build applies the rotation afterwards, so you never compute a rotated coordinate by hand.

```toml
map = """
(0,0) (0,1) (0,2) (0,3) (0,4) (0,5)
(1,0) (1,1) (1,2) (1,3) (1,4) (1,5)
(2,0) (2,1) (2,2) (2,3) (2,4) (2,5)
[y=0.25] [3.0] [r=15@(3,3.25)] (3,3) (3,4) (3,5)
"""
```

The three thumb keys sit on a flat grid starting at x = 3 and swing 15° clockwise about `(3, 3.25)` as one rigid piece. The pivot uses the same flat coordinates as everything else — here it is the top-left corner of the first thumb key (its row top is y = 3.25 because of the `[y=0.25]`).

A region spans line breaks, so a multi-row block rotates as one piece — keep writing rows and bring each one back under the cluster with a gap:

```toml
map = """
(0,0) (0,1) (0,2)
(1,0) (1,1) (1,2)
(2,0) (2,1) (2,2)
[3.5] [r=25@(3.5,3)] (3,0) (3,1)
[3.5] (4,0) (4,1)
"""
```

- `[r=0]` ends a region. A new `[r=deg@(x,y)]` also ends the previous one, so two thumb clusters chain without an `[r=0]` between them.
- A shape's own `r` still means "tilt in place about the key's center" and adds to the region's angle.

### Encoders

`(e, id)` places rotary encoder `id` in the rendered layout, e.g. `(e, 0)`. Encoder ids must be unique and cover `0..N` with no gaps. When you declare _any_ encoder tokens, their count must match the number of encoders your board declares — but providing no `(e, id)` tokens at all is allowed: the encoders still work, they just have nothing to render. Encoders are render-only — they are _not_ keymap positions, so they don't appear in `[[keymap.layer]].keys` (their actions go in `[[keymap.layer]].encoders` instead).

### Variants

One `map` can describe a _superset_ of positions that renders in several ways — for example a 60% board that ships as ANSI, ISO, and split-backspace. Each `[[layout.variant]]` is a complete render of the **same keymap**: it hides some keys and reshapes others, and the remaining keys reflow to close the gaps.

```toml
[layout]
rows = 5
cols = 16
default_variant = "ansi"
map = """..."""

[[layout.variant]]
name = "ansi"
hidden = ["(3,14)", "(0,14)"]                          # drop the ISO and split-bs keys

[[layout.variant]]
name = "iso"
shapes = { "(2,12)" = "@isoenter", "(3,0)" = "@lsft_iso" }
hidden = ["(0,14)"]
```

- `hidden` — `"(row,col)"` positions to drop from this variant; following keys reflow left to close the gap.
- `shapes` — `"(row,col)"` → `"@shape"` overrides that reshape a key in this variant only.
- `default_variant` — the name of the variant shown first (a `[layout]`-level field). If it is unset or names a variant that doesn't exist, the first variant is used.

## The keymap

Once the layout is defined, describe each layer under `[[keymap.layer]]`:

```toml
# layer 0 (default):
[[keymap.layer]]
name = "base_layer" #optional name for the layer
keys = """
NumLock KpSlash KpAsterisk KpMinus
Kp7     Kp8     Kp9        KpPlus
Kp4     Kp5     Kp6
Kp1     Kp2     Kp3        Enter
    Kp0         KpDot
"""

# layer 1:
[[keymap.layer]]
name = "mouse_navigation" #optional name for the layer
keys = """
TO(base_layer)   @my_cut    @my_copy         @my_paste
MouseBtn1        MouseUp    MouseBtn2        MouseWheelUp
MouseLeft        MouseBtn4  MouseRight
MouseWheelLeft   MouseDown  MouseWheelRight  MouseWheelDown
       MouseBtn1            MouseBtn2
"""
```

The number and order of keys on each layer must match the `(row, col)` keys in `layout.map` — encoders and render-only items don't count. Whitespace and line breaks are free to vary, but keeping a consistent arrangement with the real keyboard is worthwhile.

`[keymap].layers` sets the total number of layers. It's optional — it defaults to the number of `[[keymap.layer]]` blocks you define; set it larger only to reserve extra empty layers (handy for on-the-fly Vial or Rynk editing).

::: warning

If you define fewer layers than `keymap.layers`, RMK fills the rest with empty layers automatically (so you can configure them freely in Vial). Empty layers still consume flash and RAM, so avoid a large layer count if you're short on space.

:::

In each `layer.keys`, keys are bound to key actions. Because of the TOML format, this is done in a string: RMK parses it and fills in the actual keymap initializer, like the one in [`keymap.rs`](https://github.com/rmk-rs/rmk/tree/main/examples/use_rust/rp2040/src/keymap.rs).

The `layer.keys` string follows several rules:

1. For a simple keycode (i.e., keys in RMK's [`HidKeyCode`](https://github.com/rmk-rs/rmk/blob/main/rmk-types/src/keycode/hid.rs) enum), just fill in its name as listed in the [KeyCode table](./keymap_configuration/keycodes).

   For example, `Backspace` is turned into the corresponding HID keycode. The keycode string must be valid, or RMK won't compile. To make things easier, a number of alternative key names were added (see the alias column in the [KeyCode table](./keymap_configuration/keycodes)), and lookup is case-insensitive.

   For a simple keycode with modifiers held, use `WM(key, modifier)` to create a keypress-with-modifier action. Modifiers can be chained like `LShift | RGui` to hold several at once.

   To hold one or more modifiers without sending a non-modifier key, use `MOD(modifier)`. For example, `MOD(LCtrl | LAlt | LGui)` holds all three modifiers until the bound key is released.

   You may use aliases, prefixed with `@`, like `@my_copy` in the example above. Alias names are case sensitive. Defining aliases is covered below.

   You may use layer names instead of layer numbers, like `TO(base_layer)` above.
   ::: warning

   A layer name used this way may not contain whitespace and may not be a number. Layer names are case sensitive.

   :::

2. For a no-key (`KeyAction::No`), use `No`.

3. For a transparent key (`KeyAction::Transparent`), use `_` or `__` (any number of `_`), or `Trns`.

4. RMK supports many advanced layer operations:
   1. `DF(n)` — switch the default layer to layer `n`.
   2. `PDF(n)` — switch the default layer to layer `n` and persist it to storage, so it survives reboots.
   3. `MO(n)` — momentarily activate layer `n`.
   4. `LM(n, modifier)` — activate layer `n` with a modifier held. The modifier chains like `WM`.
   5. `LT(n, key, <profile_name>)` — activate layer `n` on hold, or tap `key` (tap/hold). `key` is an RMK [`KeyCode`](./keymap_configuration/keycodes); the optional `profile_name` sets the key's [profile](./behavior#per-key-profiles-for-morse-tapdance-tap-hold-fine-tuning).
   6. `OSL(n)` — one-shot layer `n`.
   7. `OSM(modifier)` — one-shot modifier. The modifier chains like `WM`.
   8. `TT(n)` — activate layer `n`, or tap-toggle it.
   9. `TG(n)` — toggle layer `n`.
   10. `TO(n)` — activate layer `n` and deactivate all other layers.

   These match QMK's definitions; see the [QMK layer docs](https://docs.qmk.fm/#/feature_layers). If you need another action, please [file an issue](https://github.com/rmk-rs/rmk/issues/new).

5. For modifier-tap-hold, use `MT(key, modifier, <profile_name>)`, where the modifier can be a chain as in rule 1. The optional `profile_name` sets the key's [profile](./behavior#per-key-profiles-for-morse-tapdance-tap-hold-fine-tuning).
<!-- If you're using home-row mod(HRM), you can also use `HRM(key, modifier)` to create a modifier-tap-hold whose configuration is optimized for home-row mod. -->

6. For a generic tap-hold, use `TH(key-tap, key-hold, <profile_name>)`. The optional `profile_name` sets the key's [profile](./behavior#per-key-profiles-for-morse-tapdance-tap-hold-fine-tuning).

   The tap/hold slots of `MT`, `TH`, and `LT` aren't limited to plain keycodes — they accept any single action, so you can nest other actions inside them. For example, `MT(WM(P, RAlt), LShift, HRM)` taps `RAlt+P` and holds `LShift` with the `HRM` profile, and `TH(WM(A, LShift), MO(2))` taps `Shift+A` and holds momentary-layer 2. Composite tap-hold/morse forms (`MT`/`TH`/`LT`/`TT`/`TD`) cannot be nested inside a slot.

7. For a shifted key, you can use `SHIFTED(key)` as an alternative to with-modifier (`WM(key, LShift)` - see above).

8. For Morse/Tap Dance, use `TD(n)` or `Morse(n)` — they are the same.

9. For keyboard macros, use `Macro(n)`.

10. For a [Plover HID steno](../features/steno) key, use `STN(key)`, where `key` is the steno key name (for example `STN(S1)`). Requires the `steno` Cargo feature.

## Aliases

The `[aliases]` section maps user-defined names to replacement strings, which you can then use in `layer.keys`:

```toml
# aliases for the example above:
[aliases]
my_cut = "WM(X, LCtrl)"
my_copy = "WM(C, LCtrl)"
my_paste = "WM(V, LCtrl)"
```

::: warning

Alias names may not contain whitespace or any of `(`, `)`, `,`, `@`, and they are case sensitive.

:::

## Converting from KLE or Vial

If your keyboard already has a [KLE](http://www.keyboard-layout-editor.com/) layout or a [Vial](https://get.vial.today/) definition, you don't have to write the `[layout]` section by hand: `rmkit layout convert` (part of [rmkit](https://github.com/rmk-rs/rmkit)) converts both. It accepts a raw KLE JSON export ("Download JSON" on keyboard-layout-editor.com) or a `vial.json` (which embeds the same KLE data in `layouts.keymap`), and emits the equivalent `[layout]`:

```bash
rmkit layout convert path/to/vial.json -o layout.toml   # vial.json → [layout]
rmkit layout convert path/to/kle_export.json            # a raw KLE export works too
rmkit layout convert --to-vial keyboard.toml            # reverse: [layout] → vial.json
```

Key positions, cap sizes, split gaps, rotation, ISO/L-shaped caps, encoders, and VIA layout options are all converted (into `map` items plus `[layout.shapes]` / `[[layout.variant]]` blocks as needed — a KLE rotation cluster becomes an `[r=deg@(x,y)]` region with its keys kept on the flat grid), and the result is round-tripped through RMK's own layout builder before it is printed, so it is guaranteed to build.

Two things to review in the output:

- **Matrix positions** are taken from the VIA-style `row,col` legends. A plain KLE export usually has key labels instead (`Esc`, `Q`, …) — the converter then assigns positions row-major and prints a warning; adjust them to match your `[matrix]` wiring.
- **No keycodes are converted** (KLE and `vial.json` carry none), so author the `[keymap]` yourself: each layer's `keys` follow the map's key order, plus one `["cw", "ccw"]` pair per encoder in `encoders`.

The reverse direction (`--to-vial`) turns a `keyboard.toml`'s `[layout]` into a starting `vial.json` for [Vial support](../features/vial_support). To preview the layout without flashing, render it in the terminal with `rmkit layout show` — it takes a `keyboard.toml`, or a `vial.json` / KLE export directly.
