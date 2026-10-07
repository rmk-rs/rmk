use core::fmt::Write as _;

use embedded_graphics::image::{Image, ImageRaw};
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_5X8;
#[cfg(feature = "_ble")]
use embedded_graphics::mono_font::iso_8859_1::FONT_6X9;
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
#[cfg(feature = "_ble")]
use embedded_graphics::primitives::Rectangle;
use embedded_graphics::primitives::{Circle, Line, PrimitiveStyle};
use embedded_graphics::text::Text;
#[cfg(feature = "_ble")]
use rmk_types::battery::{BatteryStatus, ChargeState};

use super::icons;
use crate::display::{DisplayRenderer, RenderContext};
#[cfg(feature = "_ble")]
use crate::event::BatteryStatusEvent;

const FONT_STYLE: MonoTextStyle<'_, BinaryColor> = MonoTextStyle::new(&FONT_5X8, BinaryColor::On);
const STROKE: PrimitiveStyle<BinaryColor> = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
const FILL: PrimitiveStyle<BinaryColor> = PrimitiveStyle::with_fill(BinaryColor::On);

const ICON_SZ: i32 = icons::ICON_SIZE as i32;
const MOD_GAP: i32 = 3;
const LOCK_DOT_DIAMETER: u32 = 4;
const LOCK_DOT_SPACING: i32 = LOCK_DOT_DIAMETER as i32 + 2;

/// Display orientation derived from dimensions.
enum Orientation {
    /// w > h (e.g. 128x64, 128x32)
    Landscape,
    /// h > w (e.g. 32x128)
    Portrait,
}

/// Pre-computed layout: vertical zone boundaries for each section.
struct Layout {
    orientation: Orientation,
    w: i32,
    h: i32,
    /// Y positions of horizontal separators between zones.
    /// Landscape: 3 zones  (info | modifiers | status)
    /// Portrait:  4 zones  (info | mods_top | mods_bot | status)
    zones: [i32; 5],
}

impl Layout {
    fn from_display<D: DrawTarget>(display: &D) -> Self {
        let bbox = display.bounding_box();
        let w = bbox.size.width as i32;
        let h = bbox.size.height as i32;

        let (orientation, zones) = if w >= h {
            // Landscape: 3 equal zones
            let z = h / 3;
            (Orientation::Landscape, [0, z, 2 * z, h, h])
        } else {
            // Portrait: 4 equal zones
            let z = h / 4;
            (Orientation::Portrait, [0, z, 2 * z, 3 * z, h])
        };

        Self {
            orientation,
            w,
            h,
            zones,
        }
    }

    /// Vertical center baseline for text in zone `z`.
    fn zone_center_y(&self, z: usize) -> i32 {
        let top = self.zones[z];
        let bot = self.zones[z + 1];
        top + (bot - top) / 2
    }

    /// Top of zone `z`.
    fn zone_top(&self, z: usize) -> i32 {
        self.zones[z]
    }

    /// Height of zone `z`.
    fn zone_height(&self, z: usize) -> i32 {
        self.zones[z + 1] - self.zones[z]
    }

    /// Horizontal center for `n` characters (FONT_5X8, char width = 5).
    fn center_x(&self, char_count: usize) -> i32 {
        ((self.w - char_count as i32 * 5) / 2).max(0)
    }
}

/// Default OLED renderer with automatic landscape/portrait adaptation.
#[derive(Default)]
pub struct OledRenderer;

impl DisplayRenderer<BinaryColor> for OledRenderer {
    fn render<D: DrawTarget<Color = BinaryColor>>(&mut self, ctx: &RenderContext, display: &mut D) {
        display.clear(BinaryColor::Off).ok();
        if ctx.sleeping {
            return;
        }
        let layout = Layout::from_display(display);

        draw_info_zone(ctx, display, &layout);
        draw_modifier_zones(ctx, display, &layout);
        draw_status_zone(ctx, display, &layout);
    }
}

fn draw_info_zone<D: DrawTarget<Color = BinaryColor>>(ctx: &RenderContext, display: &mut D, layout: &Layout) {
    let mut lyr: heapless::String<8> = heapless::String::new();
    let mut wpm: heapless::String<8> = heapless::String::new();

    match layout.orientation {
        Orientation::Landscape => {
            write!(lyr, "Lyr:{}", ctx.layer).ok();
            write!(wpm, "WPM:{:03}", ctx.wpm).ok();

            Text::new(&wpm, Point::new(2, 8), FONT_STYLE).draw(display).ok();
            Text::new(&lyr, Point::new(2, 16), FONT_STYLE).draw(display).ok();
        }
        Orientation::Portrait => {
            // Zone 0: only layer + BLE (pinned to top)
            write!(lyr, "L:{}", ctx.layer).ok();
            Text::new(&lyr, Point::new(0, 8), FONT_STYLE).draw(display).ok();

            // Zone 1: WPM centered
            write!(wpm, "{:03}", ctx.wpm).ok();
            Text::new(
                &wpm,
                Point::new(layout.center_x(wpm.len()), layout.zone_center_y(1)),
                FONT_STYLE,
            )
            .draw(display)
            .ok();
        }
    }

    draw_connection_indicator(ctx, display, layout);
}

fn draw_modifier_zones<D: DrawTarget<Color = BinaryColor>>(ctx: &RenderContext, display: &mut D, layout: &Layout) {
    let m = ctx.modifiers;
    let mods: [(&[u8; 8], bool); 4] = [
        (&icons::SHIFT, m.left_shift() || m.right_shift()),
        (&icons::CTRL, m.left_ctrl() || m.right_ctrl()),
        (&icons::ALT, m.left_alt() || m.right_alt()),
        (&icons::GUI, m.left_gui() || m.right_gui()),
    ];

    match layout.orientation {
        Orientation::Landscape => {
            // All 4 icons in a single centered row
            let y = layout.zone_center_y(1) - ICON_SZ / 2 + 2;
            draw_mod_row(display, &mods, layout.w, y);
        }
        Orientation::Portrait => {
            // 2 rows of 2 icons in zone 2
            let zone_cy = layout.zone_center_y(2);
            let row_spacing = ICON_SZ + 4; // icon height + gap between rows
            let y1 = zone_cy - row_spacing / 2 - ICON_SZ / 2 - 1;
            let y2 = y1 + row_spacing + 1;
            draw_mod_row(display, &mods[..2], layout.w, y1);
            draw_mod_row(display, &mods[2..], layout.w, y2);
        }
    }
}

/// Draw a horizontal row of modifier icons, centered in the display width.
fn draw_mod_row<D: DrawTarget<Color = BinaryColor>>(display: &mut D, mods: &[(&[u8; 8], bool)], w: i32, y: i32) {
    let count = mods.len() as i32;
    let total_w = count * ICON_SZ + (count - 1) * MOD_GAP;
    let start_x = (w - total_w) / 2;

    for (i, (icon_data, active)) in mods.iter().enumerate() {
        let x = start_x + i as i32 * (ICON_SZ + MOD_GAP);
        let raw: ImageRaw<BinaryColor> = ImageRaw::new(*icon_data, icons::ICON_SIZE);
        Image::new(&raw, Point::new(x, y)).draw(display).ok();

        if *active {
            let underline_y = y + ICON_SZ + 1;
            Line::new(Point::new(x, underline_y), Point::new(x + ICON_SZ - 1, underline_y))
                .into_styled(STROKE)
                .draw(display)
                .ok();
        }
    }
}

fn draw_status_zone<D: DrawTarget<Color = BinaryColor>>(ctx: &RenderContext, display: &mut D, layout: &Layout) {
    let status_zone = match layout.orientation {
        Orientation::Landscape => 2,
        Orientation::Portrait => 3,
    };

    // Lock indicator dots (bottom-left of zone)
    let lock_h = LOCK_DOT_DIAMETER as i32 + LOCK_DOT_SPACING;
    let lock_y = layout.zone_top(status_zone) + layout.zone_height(status_zone) - lock_h;
    draw_lock_dots(ctx, display, 2, lock_y);

    // Battery (right side, only for BLE)
    #[cfg(feature = "_ble")]
    draw_battery_icon(ctx.battery, display, layout);
}

fn draw_lock_dots<D: DrawTarget<Color = BinaryColor>>(ctx: &RenderContext, display: &mut D, x: i32, y: i32) {
    if ctx.caps_lock {
        Circle::new(Point::new(x, y), LOCK_DOT_DIAMETER)
            .into_styled(FILL)
            .draw(display)
            .ok();
    }
    if ctx.num_lock {
        Circle::new(Point::new(x, y + LOCK_DOT_SPACING), LOCK_DOT_DIAMETER)
            .into_styled(FILL)
            .draw(display)
            .ok();
    }
}

#[cfg(feature = "_ble")]
fn draw_battery_icon<D: DrawTarget<Color = BinaryColor>>(
    battery: BatteryStatusEvent,
    display: &mut D,
    layout: &Layout,
) {
    const NUM_BARS: i32 = 6;
    const BODY_W: i32 = 5;
    const BODY_H: i32 = NUM_BARS + 2;
    const NUB_W: i32 = 3;
    const NUB_H: i32 = 1;
    const BAR_H: i32 = 1;

    const TOTAL_H: i32 = NUB_H + BODY_H;

    let status_zone = match layout.orientation {
        Orientation::Landscape => 2,
        Orientation::Portrait => 3,
    };
    // Pin battery to the bottom of the status zone
    let top_y = layout.zone_top(status_zone) + layout.zone_height(status_zone) - TOTAL_H;

    let body_x = layout.w - BODY_W;
    let nub_x = body_x + (BODY_W - NUB_W) / 2;
    let body_y = top_y + NUB_H;

    // Nub
    Rectangle::new(Point::new(nub_x, top_y), Size::new(NUB_W as u32, NUB_H as u32))
        .into_styled(STROKE)
        .draw(display)
        .ok();

    // Body outline
    Rectangle::new(Point::new(body_x, body_y), Size::new(BODY_W as u32, BODY_H as u32))
        .into_styled(STROKE)
        .draw(display)
        .ok();

    // Fill bars (bottom-up)
    let bars: i32 = match *battery {
        BatteryStatus::Available { level: Some(pct), .. } => ((pct as i32 * NUM_BARS) + 99) / 100,
        _ => 0,
    };

    for i in 0..bars {
        let bar_y = body_y + BODY_H - 1 - (i + 1) * BAR_H;
        Rectangle::new(
            Point::new(body_x + 1, bar_y),
            Size::new((BODY_W - 2) as u32, BAR_H as u32),
        )
        .into_styled(FILL)
        .draw(display)
        .ok();
    }

    // Text label
    if layout.w > 32 {
        const LABEL_CW: i32 = 6; // FONT_6X9 char width
        const LABEL_CH: i32 = 9;
        let label_style = MonoTextStyle::new(&FONT_6X9, BinaryColor::On);

        let mut label: heapless::String<8> = heapless::String::new();
        match *battery {
            BatteryStatus::Available {
                charge_state: ChargeState::Charging,
                level: Some(pct),
            } => write!(label, "{}%+", pct).ok(),
            BatteryStatus::Available {
                charge_state: ChargeState::Charging,
                level: None,
            } => write!(label, "CHG").ok(),
            BatteryStatus::Available { level: Some(pct), .. } => write!(label, "{}%", pct).ok(),
            BatteryStatus::Available { level: None, .. } => write!(label, "?").ok(),
            BatteryStatus::Unavailable => write!(label, "N/A").ok(),
        };

        let label_w = label.len() as i32 * LABEL_CW;
        let label_x = body_x - 3 - label_w;
        let label_y = top_y + LABEL_CH - 2;
        Text::new(&label, Point::new(label_x, label_y), label_style)
            .draw(display)
            .ok();
    }
}

fn draw_connection_indicator<D: DrawTarget<Color = BinaryColor>>(
    ctx: &RenderContext,
    _display: &mut D,
    _layout: &Layout,
) {
    let _connected = is_connected(ctx);

    #[cfg(feature = "_ble")]
    {
        draw_ble_indicator(_connected, _display, _layout);
    }

    #[cfg(all(not(feature = "_ble"), feature = "split"))]
    {
        draw_status_mark(_connected, _display, _layout.w - 7, 4);
    }
}

#[cfg(feature = "_ble")]
fn draw_ble_indicator<D: DrawTarget<Color = BinaryColor>>(connected: bool, display: &mut D, layout: &Layout) {
    const BT_W: i32 = icons::BT_ICON_W as i32;
    const BT_H: i32 = icons::BT_ICON_H as i32;
    const STATUS_SZ: i32 = 5;
    const GAP: i32 = 2;

    let bt_x = layout.w - BT_W - 2;
    let bt_y = layout.zone_top(0) + 2;

    let bt_raw: ImageRaw<BinaryColor> = ImageRaw::new(&icons::BT_ICON, icons::BT_ICON_W);
    Image::new(&bt_raw, Point::new(bt_x, bt_y)).draw(display).ok();

    // Status mark: below BT icon when there's enough width, to the left otherwise
    let (status_x, status_y) = if layout.h > 32 {
        (bt_x + (BT_W - STATUS_SZ) / 2, bt_y + BT_H + GAP)
    } else {
        (bt_x - STATUS_SZ - GAP, bt_y + (BT_H - STATUS_SZ) / 2)
    };

    draw_status_mark(connected, display, status_x, status_y);
}

/// Draw a small 5x5 checkmark (connected) or cross (disconnected).
fn draw_status_mark<D: DrawTarget<Color = BinaryColor>>(connected: bool, display: &mut D, x: i32, y: i32) {
    if connected {
        Line::new(Point::new(x, y + 2), Point::new(x + 2, y + 4))
            .into_styled(STROKE)
            .draw(display)
            .ok();
        Line::new(Point::new(x + 2, y + 4), Point::new(x + 4, y))
            .into_styled(STROKE)
            .draw(display)
            .ok();
    } else {
        Line::new(Point::new(x, y), Point::new(x + 4, y + 4))
            .into_styled(STROKE)
            .draw(display)
            .ok();
        Line::new(Point::new(x + 4, y), Point::new(x, y + 4))
            .into_styled(STROKE)
            .draw(display)
            .ok();
    }
}

/// Returns true when the keyboard considers itself connected.
fn is_connected(_ctx: &RenderContext) -> bool {
    // Split + BLE:
    // - peripheral: connected when paired to the central
    // - central: connected when host BLE is connected
    #[cfg(all(feature = "split", feature = "_ble"))]
    return _ctx.central_connected || _ctx.ble_status.state == crate::types::ble::BleState::Connected;

    // Split without BLE:
    // - peripheral: connected when paired to the central
    // - central: connected when at least one peripheral is paired
    #[cfg(all(feature = "split", not(feature = "_ble")))]
    return _ctx.central_connected || _ctx.peripherals_connected.iter().any(|&connected| connected);

    // BLE: connected when BLE state reports connected
    #[cfg(all(not(feature = "split"), feature = "_ble"))]
    return _ctx.ble_status.state == crate::types::ble::BleState::Connected;

    // Wired: always connected
    #[cfg(not(any(feature = "split", feature = "_ble")))]
    true
}

#[cfg(all(test, feature = "_ble"))]
mod tests {
    use embedded_graphics::mock_display::MockDisplay;

    use super::*;

    fn assert_battery_render(battery: BatteryStatusEvent, label: &str, filled_pixels: usize) {
        let mut actual = MockDisplay::<BinaryColor>::new();
        actual.set_allow_overdraw(true);
        OledRenderer.render(
            &RenderContext {
                battery,
                ..Default::default()
            },
            &mut actual,
        );

        // Battery interior on the 64x64 mock display: six rows of three pixels.
        let filled = (57..63)
            .flat_map(|y| (60..63).map(move |x| Point::new(x, y)))
            .filter(|p| actual.get_pixel(*p) == Some(BinaryColor::On))
            .count();
        assert_eq!(filled, filled_pixels, "{battery:?}");

        let mut expected = MockDisplay::<BinaryColor>::new();
        expected.set_allow_overdraw(true);
        expected.clear(BinaryColor::Off).unwrap();
        Text::new(
            label,
            Point::new(56 - label.len() as i32 * 6, 62),
            MonoTextStyle::new(&FONT_6X9, BinaryColor::On),
        )
        .draw(&mut expected)
        .unwrap();
        for pixel in (52..64).flat_map(|y| (32..59).map(move |x| Point::new(x, y))) {
            assert_eq!(
                actual.get_pixel(pixel),
                expected.get_pixel(pixel),
                "label {label}, pixel {pixel:?}"
            );
        }
    }

    #[test]
    fn unknown_battery_levels_have_no_full_bars() {
        for (level, charging, label, pixels) in [
            (None, false, "?", 0),
            (None, true, "CHG", 0),
            (Some(100), false, "100%", 18),
        ] {
            assert_battery_render(
                BatteryStatusEvent(BatteryStatus::Available {
                    charge_state: charging.into(),
                    level,
                }),
                label,
                pixels,
            );
        }
    }
}
