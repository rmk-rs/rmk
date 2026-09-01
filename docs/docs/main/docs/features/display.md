# Display

RMK has built-in support for OLED and other small displays through the `DisplayProcessor`. It subscribes to keyboard state events and redraws the screen automatically whenever something changes.

## Supported Drivers

| Driver     | Chip(s)                                                                                       | Feature flag |
| ---------- | --------------------------------------------------------------------------------------------- | ------------ |
| SSD1306    | SSD1306                                                                                       | `ssd1306`    |
| oled-async | SH1106, SH1107, SH1108, SSD1309                                                               | `oled_async` |
| lcd-async  | GC9107, GC9A01, ILI9225, ILI9341, ILI9342C, ILI9486, ILI9488, RM67162, ST7735, ST7789, ST7796 | `lcd_async`  |

There is also a per-chip feature flag for each supported chip (for example `sh1106` or `st7789`) which simply enables the corresponding driver feature.

### Supported Sizes

| Driver  | Supported resolutions               |
| ------- | ----------------------------------- |
| SSD1306 | 128x64, 128x32, 96x16, 72x40, 64x48 |
| SH1106  | 128x64                              |
| SH1107  | 64x128, 128x128                     |
| SH1108  | 64x160, 96x160, 128x160, 160x160    |
| SSD1309 | 128x64                              |

All OLED drivers support 0, 90, 180 and 270 degree rotation. LCD resolutions are set via the `W`/`H` parameters of `LcdAsyncDisplay`.

## Built-in Renderers

RMK ships two renderers out of the box (both monochrome, for `BinaryColor` displays):

- **`LogoRenderer`** — displays the RMK logo. Used by default when you don't specify a renderer.
- **`OledRenderer`** — full keyboard status screen: layer, WPM, modifier indicators, Caps/Num Lock, battery level, BLE status, and split keyboard connection state. Layout adapts automatically between landscape and portrait orientations.

Color LCDs (`lcd_async`) use the `Rgb565` color type, so they need a [custom renderer](#custom-renderers).

## Configuration

For `keyboard.toml` users, see the [Display Configuration](../configuration/display) reference for all available options.

## Rust API

For `use_rust` keyboards, initialize the display manually and pass it to `DisplayProcessor`:

```rust
// above main function

use embassy_rp::i2c::{self, I2c}; // use embassy HAL crate of your chip (for example RP2040)

bind_interrupts!(struct Irqs {
    // ... other interrupts ...
    I2C1_IRQ => i2c::InterruptHandler<I2C1>; // example: I2C1
});

use rmk::display::DisplayProcessor;
use rmk::core_traits::Runnable;

// SSD1306 via ssd1306 crate
use ssd1306::{I2CDisplayInterface, Ssd1306Async, prelude::*};

// in main function
// use embassy HAL crate of your chip (for example RP2040)
let config = embassy_rp::i2c::Config::default();
// for example using I2C1 on PIN_3 (scl) and PIN_2 (sda)
let i2c = I2c::new_async(p.I2C1, p.PIN_3, p.PIN_2, Irqs, config);

let interface = I2CDisplayInterface::new(i2c);
let display = Ssd1306Async::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
    .into_buffered_graphics_mode();

// Default: LogoRenderer
let mut oled = DisplayProcessor::new(display);

// Or use the built-in OledRenderer
use rmk::display::OledRenderer;
let mut oled = DisplayProcessor::with_renderer(display, OledRenderer::default());

run_all!(matrix, oled).await;
```

### SH1106 / oled-async

```rust
use display_interface_i2c::I2CInterface;
use oled_async::Builder;
use oled_async::displayrotation::DisplayRotation;
use oled_async::displays::sh1106::Sh1106_128_64;
use oled_async::mode::graphics::GraphicsMode;
use rmk::display::DisplayProcessor;

let interface = I2CInterface::new(i2c, 0x3C, 0x40);
let display: GraphicsMode<_, _> = Builder::new(Sh1106_128_64 {})
    .with_rotation(DisplayRotation::Rotate0)
    .connect(interface)
    .into();

let mut oled = DisplayProcessor::new(display);
```

A complete SH1106 keyboard is in the [`rp2040_oled`](https://github.com/rmk-rs/rmk/blob/main/examples/use_rust/rp2040_oled/src/main.rs) example.

### Color LCDs / lcd-async

LCDs driven by the [`lcd-async`](https://crates.io/crates/lcd-async) crate are wrapped in `LcdAsyncDisplay`, which pairs the initialized display with a `Rgb565` framebuffer (`W * H * 2` bytes). The built-in renderers are monochrome, so pass a custom `Rgb565` renderer:

```rust
use lcd_async::{Builder, models::GC9107};
use rmk::display::DisplayProcessor;
use rmk::display::drivers::lcd_async::LcdAsyncDisplay;
use static_cell::StaticCell;

const W: usize = 128;
const H: usize = 128;

static FB: StaticCell<[u8; W * H * 2]> = StaticCell::new();
let fb = FB.init([0; W * H * 2]);

let display = Builder::new(GC9107, my_interface)
    .display_size(W as u16, H as u16)
    .init(&mut embassy_time::Delay)
    .await
    .unwrap();

let mut lcd = DisplayProcessor::with_renderer(LcdAsyncDisplay::<_, _, _, _, W, H>::new(display, fb), MyRgb565Renderer);
```

### Render Intervals

```rust
use embassy_time::Duration;

// Enable animation polling (redraw every 33 ms even without events)
let mut oled = DisplayProcessor::new(display)
    .with_render_interval(Duration::from_millis(33));

// Override the minimum time between event-driven renders (default: 33 ms)
let mut oled = DisplayProcessor::new(display)
    .with_min_render_interval(Duration::from_millis(10));
```

## Custom Renderers

Implement `DisplayRenderer<C>` for your color type (`BinaryColor` for monochrome OLEDs):

```rust
use core::fmt::Write as _;
use embedded_graphics::{
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::*,
    text::Text,
};
use rmk::display::{DisplayRenderer, RenderContext};

#[derive(Default)]
pub struct MyRenderer;

impl DisplayRenderer<BinaryColor> for MyRenderer {
    fn render<D: DrawTarget<Color = BinaryColor>>(&mut self, ctx: &RenderContext, display: &mut D) {
        display.clear(BinaryColor::Off).ok();

        let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        let mut line: heapless::String<32> = heapless::String::new();

        write!(&mut line, "Layer {}  WPM {}", ctx.layer, ctx.wpm).ok();
        Text::new(&line, Point::new(0, 10), style).draw(display).ok();
    }
}
```

Then pass it to `DisplayProcessor::with_renderer`:

```rust
let mut oled = DisplayProcessor::with_renderer(display, MyRenderer);
```

Or reference it in `keyboard.toml`. The crate must be a dependency of your keyboard crate, and the renderer must implement `Default`, because RMK constructs it with `MyRenderer::default()`:

```toml
[display]
renderer = "my_crate::MyRenderer"
```

### `RenderContext` Fields

The `ctx` argument passed to `render` carries a snapshot of the current keyboard state:

| Field             | Type                  | Description                                                                |
| ----------------- | --------------------- | -------------------------------------------------------------------------- |
| `layer`           | `u8`                  | Current active layer index                                                 |
| `wpm`             | `u16`                 | Words-per-minute estimate                                                  |
| `caps_lock`       | `bool`                | Caps Lock indicator state                                                  |
| `num_lock`        | `bool`                | Num Lock indicator state                                                   |
| `modifiers`       | `ModifierCombination` | Active modifier keys (Shift, Ctrl, Alt, GUI)                               |
| `key_pressed`     | `bool`                | Whether a key is currently held down                                       |
| `key_press_latch` | `bool`                | True if a key was pressed since the last render; cleared after each render |
| `sleeping`        | `bool`                | Whether the keyboard is in sleep mode                                      |
| `battery`         | `BatteryStatusEvent`  | Battery charge level and state                                             |

Feature-gated fields (require the corresponding RMK feature to be enabled):

| Field                   | Feature          | Description                                        |
| ----------------------- | ---------------- | -------------------------------------------------- |
| `ble_status`            | `_ble`           | BLE connection profile and state                   |
| `central_connected`     | `split`          | Whether the central is connected (peripheral side) |
| `peripherals_connected` | `split`          | Per-peripheral connection state array              |
| `peripheral_batteries`  | `split` + `_ble` | Per-peripheral battery state array                 |

::: tip `key_press_latch` vs `key_pressed`
Use `key_press_latch` when you want to react to a new key press — it stays `true` even if the key was released before the render ran. Use `key_pressed` to reflect the real-time held state (e.g. to display a held-key animation).
:::

## Custom Display Drivers

If your display chip is not natively supported, implement `DisplayDriver` for your display type:

```rust
use embedded_graphics::prelude::*;
use embedded_graphics::pixelcolor::BinaryColor;
use rmk::display::DisplayDriver;

struct MyDisplay { /* ... */ }

impl DrawTarget for MyDisplay {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        // Write pixels to framebuffer
        Ok(())
    }
}

impl OriginDimensions for MyDisplay {
    fn size(&self) -> Size { Size::new(128, 32) }
}

impl DisplayDriver for MyDisplay {
    async fn init(&mut self) {
        // Initialize display hardware
    }

    async fn flush(&mut self) {
        // Flush framebuffer to display
    }
}
```

## Related Documentation

- [Display Configuration](../configuration/display) — `keyboard.toml` reference for display settings
- [Processor](./processor) — how processors work and the `#[processor]` macro
- [Event](./event) — built-in events that `DisplayProcessor` subscribes to
- [Split Keyboard](./split_keyboard) — split keyboard setup
