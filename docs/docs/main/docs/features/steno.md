# Stenography

RMK supports [Plover](https://opensteno.org/plover/) stenography over
USB HID. When the `steno` feature is enabled, the keyboard registers as a
stenography machine that Plover (v5.1+) connects to via the
[plover-machine-hid](https://github.com/dnaq/plover-machine-hid) plugin,
without serial emulation.

Steno keys are mapped in your layout like any other key. RMK sends the live
state of all held steno keys to the host on every key press and release,
matching the [Plover HID protocol](https://github.com/dnaq/plover-machine-hid).
Chord detection happens entirely in Plover: by default chords fire when all
keys are released, but Plover's "first-up chord send" and "auto-repeat"
options also work because the firmware reports every state change.

## Setup

### 1. Enable the feature

Add the `steno` feature to your `Cargo.toml`:

```toml
rmk = { version = "0.9", features = ["steno"] }
```

### 2. Map steno keys in your layout

#### keyboard.toml

Use `STN(key)` in your layer keys:

```toml
[[keymap.layer]]
keys = [
    "STN(NUM1)",  "STN(NUM1)",  "STN(NUM1)",  "STN(NUM1)",  "STN(NUM1)",   "STN(NUM1)",  "STN(NUM1)",  "STN(NUM1)",  "STN(NUM1)",  "STN(NUM1)",
    "STN(S1)",    "STN(T)",     "STN(P)",     "STN(H)",     "STN(STAR1)",  "STN(STAR1)", "STN(RF)",    "STN(RP)",    "STN(RL)",    "STN(RT)",
    "STN(S1)",    "STN(K)",     "STN(W)",     "STN(R)",     "STN(STAR1)",  "STN(STAR1)", "STN(RR)",    "STN(RB)",    "STN(RG)",    "STN(RS)",
    "STN(A)",     "STN(O)",     "_",          "_",          "_",           "_",          "_",          "_",          "STN(RE)",    "STN(RU)",
]
```

#### Rust API

Use the `steno!` macro:

```rust
use rmk::{a, steno};

let keymap = [
    steno!(NUM1), steno!(NUM1), steno!(NUM1), steno!(NUM1), steno!(NUM1),  steno!(NUM1), steno!(NUM1), steno!(NUM1), steno!(NUM1), steno!(NUM1),
    steno!(S1),   steno!(T),    steno!(P),    steno!(H),    steno!(STAR1), steno!(STAR1),steno!(RF),   steno!(RP),   steno!(RL),   steno!(RT),
    steno!(S1),   steno!(K),    steno!(W),    steno!(R),    steno!(STAR1), steno!(STAR1),steno!(RR),   steno!(RB),   steno!(RG),   steno!(RS),
    steno!(A),    steno!(O),    a!(No),       a!(No),       a!(No),        a!(No),       a!(No),       a!(No),       steno!(RE),   steno!(RU),
];
```

#### Key names

`STN(key)` and `steno!(key)` accept any `StenoKey` name (case-insensitive in `keyboard.toml`), each mapped to its position in the Plover HID key chart:

| Group                                              | Keys                                                                      |
| -------------------------------------------------- | ------------------------------------------------------------------------- |
| Standard (left hand, vowels, asterisk, number bar) | `S1`, `T`, `K`, `P`, `W`, `H`, `R`, `A`, `O`, `STAR1`, `NUM1`             |
| Standard (right hand, prefixed `R`)                | `RE`, `RU`, `RF`, `RR`, `RP`, `RB`, `RL`, `RG`, `RT`, `RS`, `RD`, `RZ`    |
| Extended                                           | `S2`, `STAR2`, `STAR3`, `STAR4`, `NUM2` to `NUM9`, `NUMA`, `NUMB`, `NUMC` |
| Extra (vendor-defined)                             | `X1` to `X26`                                                             |

`STAR` is an alias for `STAR1`.

## Connecting Plover

1. Flash your keyboard with the `steno` feature enabled.
2. Open Plover and go to **Configure > Machine**.
3. Select **Plover HID** as the machine type.
4. Click **Connect**. Plover finds your keyboard by its HID usage page
   (`0xFF50`).
5. Test a chord with Plover paper tape.

## Limitations

- USB only. BLE does not support steno because the standard HID-over-GATT
  service has no stenography characteristic.
- No dictionary on the keyboard. RMK sends raw steno chords to the host;
  translation to text happens in Plover.
- No serial emulation: TX Bolt and GeminiPR are not supported.
