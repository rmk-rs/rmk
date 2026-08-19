//! [`DisplayDriver`] adapter for displays driven by the [`lcd-async`](https://crates.io/crates/lcd-async) crate.
//!
//! # Example
//!
//! ```rust,ignore
//! use lcd_async::{Builder, models::GC9107};
//! use rmk::display::{DisplayProcessor, drivers::lcd_async::LcdAsyncDisplay};
//! use static_cell::StaticCell;
//!
//! const W: usize = 128;
//! const H: usize = 128;
//!
//! static FB: StaticCell<[u8; W * H * 2]> = StaticCell::new();
//! let fb = FB.init([0; W * H * 2]);
//!
//! let display = Builder::new(GC9107, my_interface)
//!     .display_size(W as u16, H as u16)
//!     .init(&mut embassy_time::Delay).await.unwrap();
//!
//! let mut processor =
//!     DisplayProcessor::new(LcdAsyncDisplay::<_, _, _, _, W, H>::new(display, fb));
//! ```

use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::Rectangle;
use embedded_hal::digital::OutputPin;
use lcd_async::Display;
use lcd_async::interface::Interface;
use lcd_async::models::Model;
use lcd_async::raw_framebuf::RawFrameBuf;

use super::super::DisplayDriver;

/// Bridges an [`lcd_async::Display`] plus a software framebuffer into RMK's
/// [`DisplayDriver`] trait.
///
/// # Generics
///
/// - `DI` — bus interface (`Word = u8`).
/// - `MOD` — chip model from [`lcd_async::models`] using the `Rgb565` color format.
/// - `RST` — reset pin (use [`lcd_async::NoResetPin`] when reset is handled out-of-band).
/// - `BUF` — framebuffer storage of length `W * H * 2`. Typically `&'static mut [u8; W * H * 2]`.
/// - `W` / `H` — display resolution in pixels.
///
/// The wrapped [`Display`] must already be initialized by [`lcd_async::Builder::init`];
/// [`DisplayDriver::init`] is a no-op.
///
/// [`flush`](DisplayDriver::flush) sends only the rectangle drawn since the last
/// flush (see [`with_staging`](Self::with_staging)), so renderers should repaint
/// only what changed rather than clearing the screen each frame.
pub struct LcdAsyncDisplay<DI, MOD, RST, BUF, const W: usize, const H: usize>
where
    DI: Interface<Word = u8>,
    MOD: Model<ColorFormat = Rgb565>,
    RST: OutputPin,
    BUF: AsMut<[u8]> + AsRef<[u8]>,
{
    display: Display<DI, MOD, RST>,
    buffer: BUF,
    /// Union of the areas drawn since the last flush; `None` = nothing to send.
    dirty: Option<Rectangle>,
    /// Scratch for sending a sub-width rectangle as one contiguous transfer.
    staging: Option<&'static mut [u8]>,
}

impl<DI, MOD, RST, BUF, const W: usize, const H: usize> LcdAsyncDisplay<DI, MOD, RST, BUF, W, H>
where
    DI: Interface<Word = u8>,
    MOD: Model<ColorFormat = Rgb565>,
    RST: OutputPin,
    BUF: AsMut<[u8]> + AsRef<[u8]>,
{
    /// Wrap an already-initialized [`Display`] and its framebuffer storage.
    ///
    /// `buffer` length must be exactly `W * H * 2` bytes (Rgb565 = 2 bytes/pixel).
    pub fn new(display: Display<DI, MOD, RST>, buffer: BUF) -> Self {
        debug_assert_eq!(
            buffer.as_ref().len(),
            W * H * 2,
            "framebuffer length must equal W * H * 2 (Rgb565)",
        );
        Self {
            display,
            buffer,
            dirty: Some(Rectangle::new(Point::zero(), Size::new(W as u32, H as u32))),
            staging: None,
        }
    }

    /// Scratch for sending a sub-width dirty rectangle in one transfer instead of
    /// widening it to full rows. `width * height * 2` bytes covers the largest
    /// rectangle in one call; a smaller buffer takes more passes, and one too small
    /// for a single row falls back to full rows.
    pub fn with_staging(mut self, staging: &'static mut [u8]) -> Self {
        self.staging = Some(staging);
        self
    }

    /// Borrow the underlying [`Display`].
    pub fn display(&mut self) -> &mut Display<DI, MOD, RST> {
        &mut self.display
    }

    /// Merge `area` (clipped to the screen) into the dirty region.
    fn mark_dirty(&mut self, area: &Rectangle) {
        let screen = Rectangle::new(Point::zero(), Size::new(W as u32, H as u32));
        let area = area.intersection(&screen);
        let Some(area_br) = area.bottom_right() else {
            return;
        };
        self.dirty = Some(match self.dirty {
            None => area,
            // `dirty` is never zero-sized, so `unwrap_or` is only for panic-freedom.
            Some(d) => Rectangle::with_corners(
                d.top_left.component_min(area.top_left),
                d.bottom_right().unwrap_or(d.top_left).component_max(area_br),
            ),
        });
    }
}

impl<DI, MOD, RST, BUF, const W: usize, const H: usize> DisplayDriver for LcdAsyncDisplay<DI, MOD, RST, BUF, W, H>
where
    DI: Interface<Word = u8>,
    MOD: Model<ColorFormat = Rgb565>,
    RST: OutputPin,
    BUF: AsMut<[u8]> + AsRef<[u8]>,
{
    async fn init(&mut self) {}

    async fn flush(&mut self) {
        let Some(dirty) = self.dirty else {
            return;
        };
        let x0 = dirty.top_left.x as usize;
        let y0 = dirty.top_left.y as usize;
        let w = dirty.size.width as usize;
        let h = dirty.size.height as usize;

        // A sub-width rectangle isn't contiguous in the framebuffer; a transfer per
        // row costs more than the skipped columns save, so gather rows into `staging`.
        match self.staging.as_deref_mut() {
            Some(staging) if w < W && staging.len() >= w * 2 => {
                let rows_per_pass = staging.len() / (w * 2);
                let buffer = self.buffer.as_ref();
                let mut y = y0;
                while y < y0 + h {
                    let count = rows_per_pass.min(y0 + h - y);
                    for n in 0..count {
                        let from = ((y + n) * W + x0) * 2;
                        staging[n * w * 2..(n + 1) * w * 2].copy_from_slice(&buffer[from..from + w * 2]);
                    }
                    if self
                        .display
                        .show_raw_data(x0 as u16, y as u16, w as u16, count as u16, &staging[..count * w * 2])
                        .await
                        .is_err()
                    {
                        return;
                    }
                    y += count;
                }
            }
            // No staging: widen to full rows, which are contiguous.
            _ => {
                let rows = &self.buffer.as_ref()[y0 * W * 2..(y0 + h) * W * 2];
                if self
                    .display
                    .show_raw_data(0, y0 as u16, W as u16, h as u16, rows)
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
        self.dirty = None;
    }
}

impl<DI, MOD, RST, BUF, const W: usize, const H: usize> OriginDimensions for LcdAsyncDisplay<DI, MOD, RST, BUF, W, H>
where
    DI: Interface<Word = u8>,
    MOD: Model<ColorFormat = Rgb565>,
    RST: OutputPin,
    BUF: AsMut<[u8]> + AsRef<[u8]>,
{
    fn size(&self) -> Size {
        Size::new(W as u32, H as u32)
    }
}

impl<DI, MOD, RST, BUF, const W: usize, const H: usize> DrawTarget for LcdAsyncDisplay<DI, MOD, RST, BUF, W, H>
where
    DI: Interface<Word = u8>,
    MOD: Model<ColorFormat = Rgb565>,
    RST: OutputPin,
    BUF: AsMut<[u8]> + AsRef<[u8]>,
{
    type Color = Rgb565;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Rgb565>>,
    {
        let mut min = Point::new(i32::MAX, i32::MAX);
        let mut max = Point::new(i32::MIN, i32::MIN);
        {
            let mut fb = RawFrameBuf::<Rgb565, _>::new(self.buffer.as_mut(), W, H);
            fb.draw_iter(pixels.into_iter().inspect(|Pixel(pos, _)| {
                min = min.component_min(*pos);
                max = max.component_max(*pos);
            }))
            .ok();
        }
        if min.x <= max.x {
            self.mark_dirty(&Rectangle::with_corners(min, max));
        }
        Ok(())
    }

    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Rgb565>,
    {
        {
            let mut fb = RawFrameBuf::<Rgb565, _>::new(self.buffer.as_mut(), W, H);
            fb.fill_contiguous(area, colors).ok();
        }
        self.mark_dirty(area);
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Rgb565) -> Result<(), Self::Error> {
        {
            let mut fb = RawFrameBuf::<Rgb565, _>::new(self.buffer.as_mut(), W, H);
            fb.fill_solid(area, color).ok();
        }
        self.mark_dirty(area);
        Ok(())
    }

    fn clear(&mut self, color: Rgb565) -> Result<(), Self::Error> {
        {
            let mut fb = RawFrameBuf::<Rgb565, _>::new(self.buffer.as_mut(), W, H);
            fb.clear(color).ok();
        }
        self.mark_dirty(&Rectangle::new(Point::zero(), Size::new(W as u32, H as u32)));
        Ok(())
    }
}
