//! Display support.
//!
//! Provides [`DisplayProcessor`] — a processor that subscribes to keyboard
//! state events and renders them on any display implementing [`DisplayDriver`].
//!
//! # Customization
//!
//! The processor is generic over a [`DisplayDriver`] and a [`DisplayRenderer`].
//! The built-in [`LogoRenderer`] displays the RMK logo on startup.  For a
//! full-featured keyboard status display, use [`OledRenderer`] instead.  To draw your own content implement [`DisplayRenderer<C>`]
//! for your color type and pass it via [`DisplayProcessor::with_renderer`].
//!
//! # Feature flags
//!
//! - `display` — base traits and processor (requires `embedded-graphics`)
//! - `ssd1306` — SSD1306 OLED driver support (implies `display`)
//!
//! # Example — SSD1306 with default renderer
//!
//! ```rust,ignore
//! use ssd1306::{I2CDisplayInterface, Ssd1306Async, prelude::*};
//! use rmk::display::DisplayProcessor;
//!
//! let interface = I2CDisplayInterface::new(i2c);
//! let display = Ssd1306Async::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
//!     .into_buffered_graphics_mode();
//!
//! let mut oled = DisplayProcessor::new(display);
//! run_all!(matrix, oled);
//! ```
//!
//! # Example — custom renderer
//!
//! ```rust,ignore
//! use rmk::display::{DisplayRenderer, DisplayProcessor, RenderContext};
//! use embedded_graphics::{prelude::*, pixelcolor::BinaryColor, text::Text,
//!     mono_font::{ascii::FONT_6X10, MonoTextStyle}};
//!
//! struct BigLayer;
//!
//! impl DisplayRenderer<BinaryColor> for BigLayer {
//!     fn render<D: DrawTarget<Color = BinaryColor>>(
//!         &mut self, ctx: &RenderContext, display: &mut D,
//!     ) {
//!         let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
//!         let mut buf: heapless::String<8> = heapless::String::new();
//!         core::fmt::write(&mut buf, format_args!("L{}", ctx.layer)).ok();
//!         Text::new(&buf, Point::new(0, 12), style).draw(display).ok();
//!     }
//! }
//!
//! let mut oled = DisplayProcessor::with_renderer(display, BigLayer);
//! run_all!(matrix, oled);
//! ```

pub mod drivers;
mod renderers;

#[cfg(feature = "oled_async")]
pub use display_interface_i2c;
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_time::{Duration, Instant, Ticker, Timer};
use embedded_graphics::prelude::*;
#[cfg(feature = "lcd_async")]
pub use lcd_async;
#[cfg(feature = "oled_async")]
pub use oled_async;
pub use renderers::{LogoRenderer, OledRenderer};
use rmk_macro::processor;
#[cfg(feature = "_ble")]
use rmk_types::ble::BleStatus;
use rmk_types::modifier::ModifierCombination;
#[cfg(feature = "ssd1306")]
pub use ssd1306;

use crate::core_traits::Runnable;
#[cfg(feature = "_ble")]
use crate::event::ConnectionStatusChangeEvent;
#[cfg(all(feature = "split", feature = "_ble"))]
use crate::event::PeripheralBatteryEvent;
use crate::event::{
    BatteryStatusEvent, KeyboardEvent, LayerChangeEvent, LedIndicatorEvent, ModifierEvent, SleepStateEvent,
    WpmUpdateEvent,
};
#[cfg(feature = "split")]
use crate::event::{CentralConnectedEvent, PeripheralConnectedEvent};
use crate::processor::Processor;

/// Snapshot of keyboard state passed to renderers on every redraw.
///
/// # Feature-gated fields
///
/// Some fields are only available when specific RMK features are enabled:
///
/// - `ble_status` — requires the `_ble` feature
/// - `central_connected`, `peripherals_connected` — require the `split` feature
/// - `peripheral_batteries` — requires both `split` and `_ble` features
///
/// Third-party renderers that access these fields must enable the
/// corresponding features in their `Cargo.toml` dependency on `rmk`,
/// and guard access with matching `#[cfg]` attributes.
pub struct RenderContext {
    /// Board-global key geometry generated from the same physical-layout
    /// source used by lighting and host layout readback.
    pub physical_layout: crate::physical_layout::PhysicalLayout<'static>,
    /// Current active layer index.
    pub layer: u8,
    /// Current words-per-minute estimate.
    pub wpm: u16,
    /// Whether Caps Lock is active.
    pub caps_lock: bool,
    /// Whether Num Lock is active.
    pub num_lock: bool,
    /// Current battery status.
    pub battery: BatteryStatusEvent,
    /// Whether the keyboard is sleeping.
    pub sleeping: bool,
    /// Current BLE connection status (profile + state).
    #[cfg(feature = "_ble")]
    pub ble_status: BleStatus,
    /// Whether the central is connected (only meaningful on peripherals).
    #[cfg(feature = "split")]
    pub central_connected: bool,
    /// Per-peripheral connection state, indexed by peripheral id.
    #[cfg(feature = "split")]
    pub peripherals_connected: [bool; crate::SPLIT_PERIPHERALS_NUM],
    /// Per-peripheral battery status, indexed by peripheral id.
    #[cfg(all(feature = "split", feature = "_ble"))]
    pub peripheral_batteries: [BatteryStatusEvent; crate::SPLIT_PERIPHERALS_NUM],
    /// Currently active modifier keys (Shift, Ctrl, Alt, GUI).
    pub modifiers: ModifierCombination,
    /// Whether a key is currently held down.
    pub key_pressed: bool,
    /// Latched true when a key press event arrives, cleared after each render.
    ///
    /// Use this instead of `key_pressed` for detecting new presses in renderers —
    /// it persists even if the key was released before the next render ran.
    pub key_press_latch: bool,
}

impl Default for RenderContext {
    fn default() -> Self {
        Self {
            physical_layout: crate::physical_layout::PhysicalLayout::default(),
            layer: 0,
            wpm: 0,
            caps_lock: false,
            num_lock: false,
            battery: BatteryStatusEvent(rmk_types::battery::BatteryStatus::Unavailable),
            sleeping: false,
            #[cfg(feature = "_ble")]
            ble_status: BleStatus::default(),
            #[cfg(feature = "split")]
            central_connected: false,
            #[cfg(feature = "split")]
            peripherals_connected: [false; crate::SPLIT_PERIPHERALS_NUM],
            #[cfg(all(feature = "split", feature = "_ble"))]
            peripheral_batteries: [BatteryStatusEvent(rmk_types::battery::BatteryStatus::Unavailable);
                crate::SPLIT_PERIPHERALS_NUM],
            modifiers: ModifierCombination::new(),
            key_pressed: false,
            key_press_latch: false,
        }
    }
}

/// Async display driver trait.
///
/// Extends [`DrawTarget`] with the async I/O operations (`init` and `flush`)
/// that are driver-specific and not covered by `embedded-graphics`.
///
/// RMK provides built-in implementations behind feature flags (e.g. `ssd1306`).
pub trait DisplayDriver: DrawTarget {
    /// Initialize the display hardware.
    fn init(&mut self) -> impl core::future::Future<Output = ()>;
    /// Flush the framebuffer to the display.
    fn flush(&mut self) -> impl core::future::Future<Output = ()>;
}

/// Trait for custom display renderers.
///
/// Generic over the pixel color type `C`, so it works with both monochrome
/// OLEDs (`BinaryColor`) and color LCDs (`Rgb565`, etc.).
///
/// # Example
///
/// ```rust,ignore
/// use rmk::display::{DisplayRenderer, RenderContext};
/// use embedded_graphics::{
///     prelude::*, pixelcolor::BinaryColor, text::Text,
///     mono_font::{ascii::FONT_6X10, MonoTextStyle},
/// };
///
/// struct MyRenderer;
///
/// impl DisplayRenderer<BinaryColor> for MyRenderer {
///     fn render<D: DrawTarget<Color = BinaryColor>>(
///         &mut self,
///         ctx: &RenderContext,
///         display: &mut D,
///     ) {
///         let style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
///         let mut buf: heapless::String<16> = heapless::String::new();
///         core::fmt::write(&mut buf, format_args!("L{} W{}", ctx.layer, ctx.wpm)).ok();
///         Text::new(&buf, Point::new(0, 12), style).draw(display).ok();
///     }
/// }
/// ```
pub trait DisplayRenderer<C: PixelColor> {
    /// Draw the current keyboard state on the display.
    ///
    /// The renderer is responsible for clearing the display if needed.
    /// After this method returns, the caller flushes the display buffer.
    fn render<D: DrawTarget<Color = C>>(&mut self, ctx: &RenderContext, display: &mut D);
}

/// Processor that renders keyboard state on a display.
///
/// Subscribes to [`LayerChangeEvent`], [`WpmUpdateEvent`], [`LedIndicatorEvent`],
/// and [`BatteryStatusEvent`], redrawing the screen whenever any of these change.
///
/// The rendering is delegated to a [`DisplayRenderer`].  Use [`new`](Self::new)
/// for the built-in [`LogoRenderer`], or [`with_renderer`](Self::with_renderer)
/// for a custom one.
///
/// # Generics
///
/// - `D` — display driver, must implement [`DisplayDriver`].
/// - `R` — the renderer, defaults to [`LogoRenderer`].
#[processor(subscribe = [KeyboardEvent, LayerChangeEvent, WpmUpdateEvent, LedIndicatorEvent, ModifierEvent, BatteryStatusEvent, SleepStateEvent])]
#[cfg_attr(feature = "_ble", processor(subscribe = [ConnectionStatusChangeEvent]))]
#[cfg_attr(feature = "split", processor(subscribe = [PeripheralConnectedEvent, CentralConnectedEvent]))]
#[cfg_attr(all(feature = "split", feature = "_ble"), processor(subscribe = [PeripheralBatteryEvent]))]
#[::rmk::macros::runnable_generated]
pub struct DisplayProcessor<D, R = LogoRenderer>
where
    D: DisplayDriver,
    R: DisplayRenderer<D::Color>,
{
    display: D,
    renderer: R,
    ctx: RenderContext,
    initialized: bool,
    last_render: Instant,
    pending_render: bool,
    /// Minimum time between renders (rate-limiter for event-driven renders).
    min_render_interval: Duration,
    /// Poll interval for animations. `None` disables polling (event-driven only).
    render_interval: Option<Duration>,
}

impl<D> DisplayProcessor<D, LogoRenderer>
where
    D: DisplayDriver,
    LogoRenderer: DisplayRenderer<D::Color>,
{
    /// Create a new display processor with the built-in [`LogoRenderer`].
    ///
    /// Polling is disabled by default — the display only redraws on events.
    /// Use [`with_render_interval`](Self::with_render_interval) to enable
    /// periodic redraws for animations.
    pub fn new(display: D) -> Self {
        Self::with_renderer(display, LogoRenderer)
    }
}

impl<D, R> DisplayProcessor<D, R>
where
    D: DisplayDriver,
    R: DisplayRenderer<D::Color>,
{
    /// Create a new display processor with a custom [`DisplayRenderer`].
    ///
    /// Polling is disabled by default. Call
    /// [`with_render_interval`](Self::with_render_interval) to enable it.
    pub fn with_renderer(display: D, renderer: R) -> Self {
        Self {
            display,
            renderer,
            ctx: RenderContext::default(),
            initialized: false,
            last_render: Instant::from_ticks(0),
            pending_render: false,
            min_render_interval: Duration::from_millis(33),
            render_interval: None,
        }
    }

    /// Set the poll interval for periodic redraws (animations).
    ///
    /// When set, the display redraws at this interval even without events.
    /// Without this, the display only redraws when keyboard state changes.
    pub fn with_render_interval(mut self, interval: Duration) -> Self {
        self.render_interval = Some(interval);
        self
    }

    /// Provide the keyboard's generated board-global key geometry to custom
    /// renderers. This does not force built-in renderers to draw a keyboard.
    pub fn with_physical_layout(mut self, physical_layout: crate::physical_layout::PhysicalLayout<'static>) -> Self {
        self.ctx.physical_layout = physical_layout;
        self
    }

    /// Set the minimum time between event-driven renders.
    ///
    /// When events arrive faster than this interval, redraws are coalesced
    /// and the latest state is drawn once the interval elapses. Default: 10 ms.
    pub fn with_min_render_interval(mut self, interval: Duration) -> Self {
        self.min_render_interval = interval;
        self
    }

    /// Periodic poll — drives animations even when no events arrive.
    async fn poll(&mut self) {
        self.pending_render = true;
        self.render().await;
    }

    fn next_render_wait(&self) -> Option<Duration> {
        if self.pending_render {
            // A redraw was deferred by the rate limiter, so wait only for the
            // remaining time before flushing the latest state.
            Some(
                self.min_render_interval
                    .checked_sub(self.last_render.elapsed())
                    .unwrap_or(Duration::MIN),
            )
        } else {
            None
        }
    }

    /// Redraw the display if enough time has passed since the last render.
    async fn render(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_render) < self.min_render_interval {
            // Keep the newest state dirty so the run loop can flush it later.
            self.pending_render = true;
            return;
        }

        if !self.initialized {
            self.display.init().await;
            self.initialized = true;
        }

        self.renderer.render(&self.ctx, &mut self.display);
        self.ctx.key_press_latch = false;
        self.display.flush().await;

        self.pending_render = false;
        self.last_render = Instant::now();
    }

    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        self.ctx.layer = event.0;
        self.render().await;
    }

    async fn on_wpm_update_event(&mut self, event: WpmUpdateEvent) {
        self.ctx.wpm = event.0;
    }

    async fn on_led_indicator_event(&mut self, event: LedIndicatorEvent) {
        self.ctx.caps_lock = event.0.caps_lock();
        self.ctx.num_lock = event.0.num_lock();
        self.render().await;
    }

    async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
        self.ctx.battery = event;
        self.render().await;
    }

    async fn on_keyboard_event(&mut self, event: KeyboardEvent) {
        self.ctx.key_pressed = event.pressed;
        if event.pressed {
            self.ctx.key_press_latch = true;
        }
        self.render().await;
    }

    async fn on_modifier_event(&mut self, event: ModifierEvent) {
        self.ctx.modifiers = event.modifier;
        self.render().await;
    }

    async fn on_sleep_state_event(&mut self, event: SleepStateEvent) {
        self.ctx.sleeping = event.0;
        self.render().await;
    }

    #[cfg(feature = "_ble")]
    async fn on_connection_status_change_event(&mut self, event: ConnectionStatusChangeEvent) {
        self.ctx.ble_status = event.0.ble;
        self.render().await;
    }

    #[cfg(feature = "split")]
    async fn on_peripheral_connected_event(&mut self, event: PeripheralConnectedEvent) {
        if let Some(slot) = self.ctx.peripherals_connected.get_mut(event.id) {
            *slot = event.connected;
        }
        self.render().await;
    }

    #[cfg(feature = "split")]
    async fn on_central_connected_event(&mut self, event: CentralConnectedEvent) {
        self.ctx.central_connected = event.connected;
        self.render().await;
    }

    #[cfg(all(feature = "split", feature = "_ble"))]
    async fn on_peripheral_battery_event(&mut self, event: PeripheralBatteryEvent) {
        if let Some(slot) = self.ctx.peripheral_batteries.get_mut(event.id) {
            *slot = event.state;
        }
        self.render().await;
    }
}

impl<D, R> Runnable for DisplayProcessor<D, R>
where
    D: DisplayDriver,
    R: DisplayRenderer<D::Color>,
{
    async fn run(&mut self) -> ! {
        use crate::event::EventSubscriber;
        let mut sub = <Self as Processor>::subscriber();

        // Prime from current state after subscribing, so the first render
        // reflects state that was set before the processor started.
        self.ctx.sleeping = crate::state::current_sleeping();
        let indicators = crate::keyboard::current_led_indicator();
        self.ctx.caps_lock = indicators.caps_lock();
        self.ctx.num_lock = indicators.num_lock();
        #[cfg(feature = "_ble")]
        {
            self.ctx.ble_status = crate::state::current_ble_status();
        }

        self.pending_render = true;
        self.render().await;
        let mut ticker = self.render_interval.map(Ticker::every);

        loop {
            if !self.ctx.sleeping {
                match (ticker.as_mut(), self.next_render_wait()) {
                    // Polling enabled and a redraw is pending: wait for whichever
                    // happens first — the next animation tick, the deferred redraw,
                    // or a new event.
                    (Some(ticker), Some(wait)) => {
                        match select3(ticker.next(), Timer::after(wait), sub.next_event()).await {
                            Either3::First(_) => self.poll().await,
                            Either3::Second(_) => self.render().await,
                            Either3::Third(event) => self.process(event).await,
                        }
                    }
                    // Polling enabled and nothing pending: only animation ticks or
                    // new events can wake the loop.
                    (Some(ticker), None) => match select(ticker.next(), sub.next_event()).await {
                        Either::First(_) => self.poll().await,
                        Either::Second(event) => self.process(event).await,
                    },
                    // Event-driven mode with a deferred redraw: wait until the
                    // rate-limit window closes, unless a new event arrives first.
                    (None, Some(wait)) => match select(Timer::after(wait), sub.next_event()).await {
                        Either::First(_) => self.render().await,
                        Either::Second(event) => self.process(event).await,
                    },
                    // Event-driven mode with nothing pending: just block on events.
                    (None, None) => {
                        let event = sub.next_event().await;
                        self.process(event).await;
                    }
                }
            } else {
                // While sleeping, ignore timers and wait only for state changes.
                let was_sleeping = self.ctx.sleeping;
                let event = sub.next_event().await;
                self.process(event).await;

                if was_sleeping
                    && !self.ctx.sleeping
                    && let Some(ticker) = ticker.as_mut()
                {
                    // Restart the animation cadence after waking up.
                    ticker.reset();
                }
            }
        }
    }
}
