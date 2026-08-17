# Display

This page covers the `keyboard.toml` configuration for displays. For supported drivers, custom renderers, and the Rust API, see the [Display feature documentation](../features/display).

## `[display]`

Configures the display for non-split keyboards. For split keyboards, use `[split.central.display]` and/or `[split.peripheral.display]` instead — see [Split Keyboard Display](#split-keyboard-display).

### Display Fields

| Field                 | Required | Default          | Description                                                                                                                                              |
| --------------------- | -------- | ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `driver`              | Yes      | —                | Display driver: `ssd1306`, `sh1106`, `sh1107`, `sh1108`, or `ssd1309`                                                                                    |
| `size`                | Yes      | —                | Display resolution. Each driver accepts a fixed set of sizes, listed below                                                                               |
| `rotation`            | No       | `0`              | Display rotation in degrees: `0`, `90`, `180`, or `270`                                                                                                  |
| `renderer`            | No       | `"LogoRenderer"` | Renderer to use. Built-in: `"OledRenderer"`, `"LogoRenderer"`. Custom: full Rust path (e.g. `"my_crate::MyRenderer"`); the type must implement `Default` |
| `render_interval`     | No       | —                | Poll interval in ms for periodic redraws (animations). Omit for event-driven only                                                                        |
| `min_render_interval` | No       | `33`             | Minimum time in ms between event-driven renders. Coalesces rapid events to avoid flickering                                                              |

Supported `size` values per driver:

- `ssd1306`: `128x64`, `128x32`, `96x16`, `72x40`, `64x48`
- `sh1106`: `128x64`
- `sh1107`: `64x128`, `128x128`
- `sh1108`: `64x160`, `96x160`, `128x160`, `160x160`
- `ssd1309`: `128x64`

### `[display.protocol.i2c]`

| Field      | Required | Default | Description                                                    |
| ---------- | -------- | ------- | -------------------------------------------------------------- |
| `instance` | Yes      | —       | I2C peripheral instance (e.g. `"I2C0"`, `"I2C1"`, `"TWISPI0"`) |
| `sda`      | Yes      | —       | SDA pin                                                        |
| `scl`      | Yes      | —       | SCL pin                                                        |
| `address`  | No       | `0x3C`  | 7-bit I2C address                                              |

### Example

```toml
[display]
driver = "ssd1306"
size = "128x32"
rotation = 0
renderer = "OledRenderer"
min_render_interval = 10

[display.protocol.i2c]
instance = "I2C1"
scl = "PIN_3"
sda = "PIN_2"
```

## Split Keyboard Display

In a split keyboard, each half can have its own display. Use `[split.central.display]` and `[split.peripheral.display]` — the fields are the same as `[display]`.

```toml
[split.central.display]
driver = "ssd1306"
size = "128x64"
rotation = 0
render_interval = 33
min_render_interval = 10
renderer = "my_crate::DongleRenderer"

[split.central.display.protocol.i2c]
instance = "TWISPI0"
scl = "P0_17"
sda = "P0_20"

[split.peripheral.display]
driver = "ssd1306"
size = "128x32"
rotation = 90
render_interval = 33
min_render_interval = 10
renderer = "OledRenderer"

[split.peripheral.display.protocol.i2c]
instance = "TWISPI0"
scl = "P0_20"
sda = "P0_17"
```
