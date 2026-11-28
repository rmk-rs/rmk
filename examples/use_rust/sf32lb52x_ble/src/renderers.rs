//! Per-slot renderer for the SuperKey 3-screen module.
//!
//! Each display shows the label of one fixed (row, col) keymap position with a
//! per-slot background colour. The renderer reads the active layer from the
//! keymap on every redraw, so Vial remaps and layer changes propagate without
//! restart.

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyleBuilder, Rectangle};
use rmk::display::{DisplayRenderer, RenderContext};
use rmk::keymap::KeyMap;
use rmk::types::action::{Action, KeyAction};
use rmk::types::keycode::{HidKeyCode, KeyCode};
use u8g2_fonts::FontRenderer;
use u8g2_fonts::types::{FontColor, HorizontalAlignment, VerticalPosition};

use crate::display::{LCD_H, LCD_W};

/// 46-pixel-tall bold Inconsolata. Roughly fills the 128×128 panel without
/// running into the inset frame.
static BIG_FONT: FontRenderer = FontRenderer::new::<u8g2_fonts::fonts::u8g2_font_inb46_mr>();

/// Renders the label for a single keymap position.
///
/// Construction parameters:
/// - `keymap` — borrowed for the lifetime of the renderer; queried on every
///   redraw for the action at `(row, col)` of the active layer.
/// - `row`, `col` — fixed keymap position this display shows.
/// - `bg` — solid background colour for this slot.
pub struct KeyLabelRenderer<'a> {
    keymap: &'a KeyMap<'a>,
    row: u8,
    col: u8,
    bg: Rgb565,
}

impl<'a> KeyLabelRenderer<'a> {
    pub fn new(keymap: &'a KeyMap<'a>, row: u8, col: u8, bg: Rgb565) -> Self {
        Self { keymap, row, col, bg }
    }
}

impl<'a> DisplayRenderer<Rgb565> for KeyLabelRenderer<'a> {
    fn render<D: DrawTarget<Color = Rgb565>>(&mut self, _ctx: &RenderContext, display: &mut D) {
        let layer = self.keymap.active_layer();
        let action = self.keymap.action_at_pos(layer as usize, self.row, self.col);
        let label = action_label(action);

        let _ = display.clear(self.bg);

        let frame = PrimitiveStyleBuilder::new()
            .stroke_color(Rgb565::WHITE)
            .stroke_width(2)
            .build();
        let _ = Rectangle::new(Point::new(6, 6), Size::new(LCD_W as u32 - 12, LCD_H as u32 - 12))
            .into_styled(frame)
            .draw(display);

        let _ = BIG_FONT.render_aligned(
            label,
            Point::new((LCD_W as i32) / 2, (LCD_H as i32) / 2),
            VerticalPosition::Center,
            HorizontalAlignment::Center,
            FontColor::Transparent(Rgb565::WHITE),
            display,
        );
    }
}

fn action_label(action: KeyAction) -> &'static str {
    match action.to_action() {
        Action::Key(KeyCode::Hid(hid)) => hid_keycode_label(hid),
        _ => "?",
    }
}

/// Returns a short label for the most common HID keycodes. Anything not
/// covered renders as "?" — extend as your keymap grows.
fn hid_keycode_label(hid: HidKeyCode) -> &'static str {
    match hid {
        HidKeyCode::A => "A",
        HidKeyCode::B => "B",
        HidKeyCode::C => "C",
        HidKeyCode::D => "D",
        HidKeyCode::E => "E",
        HidKeyCode::F => "F",
        HidKeyCode::G => "G",
        HidKeyCode::H => "H",
        HidKeyCode::I => "I",
        HidKeyCode::J => "J",
        HidKeyCode::K => "K",
        HidKeyCode::L => "L",
        HidKeyCode::M => "M",
        HidKeyCode::N => "N",
        HidKeyCode::O => "O",
        HidKeyCode::P => "P",
        HidKeyCode::Q => "Q",
        HidKeyCode::R => "R",
        HidKeyCode::S => "S",
        HidKeyCode::T => "T",
        HidKeyCode::U => "U",
        HidKeyCode::V => "V",
        HidKeyCode::W => "W",
        HidKeyCode::X => "X",
        HidKeyCode::Y => "Y",
        HidKeyCode::Z => "Z",
        HidKeyCode::Kc1 => "1",
        HidKeyCode::Kc2 => "2",
        HidKeyCode::Kc3 => "3",
        HidKeyCode::Kc4 => "4",
        HidKeyCode::Kc5 => "5",
        HidKeyCode::Kc6 => "6",
        HidKeyCode::Kc7 => "7",
        HidKeyCode::Kc8 => "8",
        HidKeyCode::Kc9 => "9",
        HidKeyCode::Kc0 => "0",
        HidKeyCode::Enter => "ENT",
        HidKeyCode::Escape => "ESC",
        HidKeyCode::Backspace => "BS",
        HidKeyCode::Tab => "TAB",
        HidKeyCode::Space => "SPC",
        HidKeyCode::Left => "<",
        HidKeyCode::Right => ">",
        HidKeyCode::Up => "^",
        HidKeyCode::Down => "v",
        _ => "?",
    }
}
