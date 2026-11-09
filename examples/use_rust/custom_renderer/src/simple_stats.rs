use core::fmt::Write as _;

use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_6X10;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::text::Text;
use rmk::display::{DisplayRenderer, RenderContext};

/// Minimal text renderer for a 128x32 OLED.
///
/// This example is intentionally simple and is meant to be a starting point
/// for custom renderers built on top of [`RenderContext`].
#[derive(Default)]
pub struct SimpleStatsRenderer;

impl DisplayRenderer<BinaryColor> for SimpleStatsRenderer {
    async fn render<D: DrawTarget<Color = BinaryColor>>(&mut self, ctx: &RenderContext, display: &mut D) {
        display.clear(BinaryColor::Off).ok();

        let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);

        let mut line1: heapless::String<32> = heapless::String::new();
        write!(&mut line1, "Layer {}  WPM {}", ctx.layer, ctx.wpm).ok();
        Text::new(&line1, Point::new(0, 10), style).draw(display).ok();

        let mut line2: heapless::String<32> = heapless::String::new();
        write!(
            &mut line2,
            "Caps {}  Num {}",
            if ctx.caps_lock { "on" } else { "off" },
            if ctx.num_lock { "on" } else { "off" }
        )
        .ok();
        Text::new(&line2, Point::new(0, 21), style).draw(display).ok();

        let line3 = if ctx.sleeping {
            "Sleeping"
        } else if ctx.key_pressed {
            "Key pressed"
        } else {
            "Ready"
        };
        Text::new(line3, Point::new(0, 32), style).draw(display).ok();
    }
}
