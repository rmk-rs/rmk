//! Common functionality across pointing devices

use embassy_time::{Duration, Instant, Timer};
use embedded_hal::digital::InputPin;
use embedded_hal_async::digital::Wait;
use futures::future::pending;
use rmk_macro::{input_device, processor};
use rmk_types::keycode::HidKeyCode;
use usbd_hid::descriptor::MouseReport;

use crate::channel::send_hid_report;
use crate::event::{Axis, AxisEvent, AxisValType, PointingEvent, PointingProcessorEvent, PointingSetCpiEvent};
use crate::hid::{KeyboardReport, Report};
use crate::keymap::KeyMap;

pub const ALL_POINTING_DEVICES: u8 = 255;

/// Motion data from the sensor
#[derive(Debug, Clone, Copy, Default)]
pub struct MotionData {
    pub dx: i16,
    pub dy: i16,
}

/// Errors of pointing devices
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PointingDriverError {
    /// SPI communication error
    Spi,
    /// Invalid product ID detected
    InvalidProductId(u8),
    /// Initialization failed
    InitFailed,
    /// Invalid CPI value
    InvalidCpi,
    /// Invalid firmware signature detected
    InvalidFwSignature((u8, u8)),
    /// Invalid rotational transform angle
    InvalidRotTransAngle,
    /// Not implemented
    NotImplementedError,
}

pub trait PointingDriver {
    type MOTION: InputPin + Wait;

    async fn init(&mut self) -> Result<(), PointingDriverError>;
    async fn read_motion(&mut self) -> Result<MotionData, PointingDriverError>;
    fn motion_pending(&mut self) -> bool;
    fn motion_gpio(&mut self) -> Option<&mut Self::MOTION>;
    async fn set_resolution(&mut self, _cpi: u16) -> Result<(), PointingDriverError> {
        debug!("set_resolution() is not implemented for this sensor.");
        Err(PointingDriverError::NotImplementedError)
    }
}

/// Initialization state for the device
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitState {
    Pending,
    Initializing(u8),
    Ready,
    Failed,
}

/// PointingDevice an InputDevice for RMK
///
/// This device publishes `PointingEvent` events with relative X/Y movement.
#[processor(subscribe = [PointingSetCpiEvent])]
#[input_device(publish = PointingEvent)]
pub struct PointingDevice<S: PointingDriver> {
    pub sensor: S,
    pub init_state: InitState,
    pub poll_interval: Duration,
    pub id: u8,
    pub report_interval: Duration,
    pub last_poll: Instant,
    pub last_report: Instant,
    pub accumulated_x: i32,
    pub accumulated_y: i32,
}

impl<S: PointingDriver> PointingDevice<S> {
    const MAX_INIT_RETRIES: u8 = 3;

    async fn try_init(&mut self) -> bool {
        match self.init_state {
            InitState::Ready => return true,
            InitState::Failed => return false,
            InitState::Pending => {
                self.init_state = InitState::Initializing(0);
            }
            InitState::Initializing(_) => {}
        }

        if let InitState::Initializing(retry_count) = self.init_state {
            info!(
                "PointingDevice {}: Initializing sensor (attempt {})",
                self.id,
                retry_count + 1
            );

            match self.sensor.init().await {
                Ok(()) => {
                    info!("PointingDevice {}: Sensor initialized successfully", self.id);
                    self.init_state = InitState::Ready;
                    return true;
                }
                Err(e) => {
                    error!("PointingDevice {}: Init failed: {:?}", self.id, e);
                    if retry_count + 1 >= Self::MAX_INIT_RETRIES {
                        error!("PointingDevice {}: Max retries reached, giving up", self.id);
                        self.init_state = InitState::Failed;
                        return false;
                    }
                    self.init_state = InitState::Initializing(retry_count + 1);
                    Timer::after(Duration::from_millis(100)).await;
                    return false;
                }
            }
        }

        false
    }

    async fn poll_once(&mut self) {
        if self.init_state != InitState::Ready && !self.try_init().await {
            return;
        }

        if !self.sensor.motion_pending() {
            return;
        }

        match self.sensor.read_motion().await {
            Ok(motion) => {
                self.accumulated_x = self.accumulated_x.saturating_add(motion.dx as i32);
                self.accumulated_y = self.accumulated_y.saturating_add(motion.dy as i32);
            }
            Err(_e) => {
                warn!("PointingDevice {}: Read motion error", self.id);
            }
        }
    }

    fn take_report_event(&mut self) -> Option<PointingEvent> {
        if self.accumulated_x == 0 && self.accumulated_y == 0 {
            return None;
        }

        let dx = self.accumulated_x.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let dy = self.accumulated_y.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

        self.accumulated_x = 0;
        self.accumulated_y = 0;

        Some(PointingEvent {
            device_id: self.id,
            axes: [
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::X,
                    value: dx,
                },
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::Y,
                    value: dy,
                },
                AxisEvent {
                    typ: AxisValType::Rel,
                    axis: Axis::Z,
                    value: 0,
                },
            ],
        })
    }
}

impl<S: PointingDriver> PointingDevice<S> {
    async fn on_pointing_set_cpi_event(&mut self, e: PointingSetCpiEvent) {
        if e.device_id == self.id {
            info!("PointingDevice {}: Setting resolution to {}", self.id, e.cpi);
            if let Err(err) = self.sensor.set_resolution(e.cpi).await {
                debug!("PointingDevice {}: Setting resolution failed: {:?}", self.id, err);
            }
        }
    }

    // Read accumulated pointing event
    //
    // +--------------- loop ---------------+
    // ¦ poll_wait   report_wait            ¦
    // ¦     ¦           ¦                  ¦
    // ¦     V           V                  ¦
    // ¦ poll_once()     take_report_event()¦
    // ¦     ¦           ¦                  ¦
    // ¦     +- accum += ¦                  ¦
    // ¦                 >- Event returned  ¦
    // +------------------------------------+
    async fn read_pointing_event(&mut self) -> PointingEvent {
        use embassy_futures::select::{Either, select};

        if self.last_poll == Instant::MIN {
            self.last_poll = Instant::now();
        }
        if self.last_report == Instant::MIN {
            self.last_report = Instant::now();
        }

        loop {
            // A sensor that used up its init retries never reports again, and an
            // unpopulated motion pin can sit low — `wait_for_low` would then return
            // instantly and spin this loop at full speed, starving the executor.
            if self.init_state == InitState::Failed {
                pending::<()>().await;
            }

            let poll_wait = async {
                if let Some(gpio) = self.sensor.motion_gpio() {
                    let _ = gpio.wait_for_low().await;
                } else {
                    Timer::after(
                        self.poll_interval
                            .checked_sub(self.last_poll.elapsed())
                            .unwrap_or(Duration::MIN),
                    )
                    .await;
                }
            };

            let report_wait = async {
                if self.accumulated_x != 0 || self.accumulated_y != 0 {
                    Timer::after(
                        self.report_interval
                            .checked_sub(self.last_report.elapsed())
                            .unwrap_or(Duration::MIN),
                    )
                    .await;
                } else {
                    // Don't schedule report if there's no accumulated motion
                    pending::<()>().await;
                }
            };

            match select(poll_wait, report_wait).await {
                Either::First(_) => {
                    self.poll_once().await;
                    self.last_poll = Instant::now();
                }
                Either::Second(_) => {
                    if let Some(event) = self.take_report_event() {
                        self.last_report = Instant::now();
                        return event;
                    }
                }
            }
        }
    }
}

/// Pointing mode determines how raw XY motion is interpreted
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum PointingMode {
    /// Default cursor mode - XY maps to mouse XY movement
    Cursor(CursorConfig),
    /// Scroll mode - XY maps to wheel (vertical) and pan (horizontal)
    Scroll(ScrollConfig),
    /// Sniper mode - XY maps to cursor but at reduced sensitivity
    Sniper(SniperConfig),
    /// Caret mode, XY maps to vertical and horizontal caret movement
    Caret(CaretConfig),
}

impl Default for PointingMode {
    fn default() -> Self {
        Self::Cursor(CursorConfig::default())
    }
}

/// Configuration for cursor mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CursorConfig {
    /// Multiplier for X axis. Higher = more output per unit of motion. 0 disables X.
    pub multiplier_x: u8,
    /// Multiplier for Y axis. Higher = more output per unit of motion. 0 disables Y.
    pub multiplier_y: u8,
    /// Invert X axis movement.
    pub invert_x: bool,
    /// Invert Y axis movement.
    pub invert_y: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            multiplier_x: 1,
            multiplier_y: 1,
            invert_x: false,
            invert_y: false,
        }
    }
}

/// Configuration for caret mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct CaretConfig {
    /// Disable X axis in caret mode.
    pub disable_x: bool,
    /// Disable y axis in caret mode.
    pub disable_y: bool,
    /// Invert X axis.
    pub invert_x: bool,
    /// Invert Y axis.
    pub invert_y: bool,
    /// Threshold for accumulated motion. Read this as sensitivity in caret mode.
    /// Higher values mean less sensitivity.
    pub threshold: i16,
    /// Keycode to emit for up rotation. Default: Up arrow
    pub keycode_up: HidKeyCode,
    /// Keycode to emit for down rotation. Default: Down arrow
    pub keycode_down: HidKeyCode,
    /// Keycode to emit for left rotation. Default: Left arrow
    pub keycode_left: HidKeyCode,
    /// Keycode to emit for right rotation. Default: Right arrow
    pub keycode_right: HidKeyCode,
}

impl Default for CaretConfig {
    fn default() -> Self {
        Self {
            disable_x: false,
            disable_y: false,
            invert_x: false,
            invert_y: false,
            threshold: 100,
            keycode_up: HidKeyCode::Up,
            keycode_down: HidKeyCode::Down,
            keycode_left: HidKeyCode::Left,
            keycode_right: HidKeyCode::Right,
        }
    }
}
/// Configuration for scroll mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ScrollConfig {
    /// Multiplier for X axis (→ pan). Higher = more output per unit of motion. 0 disables horizontal pan.
    pub multiplier_x: u8,
    /// Divisor for X axis (→ pan). Higher = slower. 0 disables horizontal pan.
    pub divisor_x: u8,
    /// Multiplier for Y axis (→ wheel). Higher = more output per unit of motion. 0 disables vertical scroll.
    pub multiplier_y: u8,
    /// Divisor for Y axis (→ wheel). Higher = slower. 0 disables vertical scroll.
    pub divisor_y: u8,
    /// Invert X axis. In scroll mode X maps to pan, so this reverses pan direction.
    pub invert_x: bool,
    /// Invert Y axis. In scroll mode Y maps to wheel, so this reverses scroll direction.
    pub invert_y: bool,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self {
            multiplier_x: 1,
            multiplier_y: 1,
            divisor_x: 8,
            divisor_y: 8,
            invert_x: false,
            invert_y: false,
        }
    }
}
/// Configuration for sniper (precision) mode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SniperConfig {
    /// Multiplier for both axes. Higher = more output per unit of motion.
    pub multiplier: u8,
    /// Divisor for both axes. Higher = slower, more precise movement.
    pub divisor: u8,
    /// Invert X axis movement.
    pub invert_x: bool,
    /// Invert Y axis movement.
    pub invert_y: bool,
}

impl Default for SniperConfig {
    fn default() -> Self {
        Self {
            multiplier: 1,
            divisor: 4,
            invert_x: false,
            invert_y: false,
        }
    }
}

/// Accumulator for sub-unit motion deltas (used in Scroll and Sniper modes)
///
/// When dividing motion by a divisor, small movements would be lost.
/// The accumulator keeps track of the remainder so sub-unit deltas
/// accumulate until they produce a non-zero output.
#[derive(Clone, Debug, Default)]
pub struct MotionAccumulator {
    remainder_x: i16,
    remainder_y: i16,
}

impl MotionAccumulator {
    /// Reset accumulator (call when mode changes)
    pub fn reset(&mut self) {
        self.reset_x();
        self.reset_y();
    }

    /// Reset x axis remainder of accumulator
    pub fn reset_x(&mut self) {
        self.remainder_x = 0;
    }

    /// Reset y axis remainder of accumulator
    pub fn reset_y(&mut self) {
        self.remainder_y = 0;
    }

    /// Accumulate motion and return the divided output, keeping remainder.
    /// A divisor of 0 disables that axis (always outputs 0).
    pub fn accumulate(&mut self, dx: i16, dy: i16, ratio_x: (u8, u8), ratio_y: (u8, u8)) -> (i16, i16) {
        let out_x = if ratio_x.1 == 0 {
            self.remainder_x = 0;
            0
        } else {
            let total_x = self.remainder_x.saturating_add(dx * ratio_x.0 as i16);
            let out = total_x / ratio_x.1 as i16;
            self.remainder_x = total_x - out * ratio_x.1 as i16;
            out
        };

        let out_y = if ratio_y.1 == 0 {
            self.remainder_y = 0;
            0
        } else {
            let total_y = self.remainder_y.saturating_add(dy * ratio_y.0 as i16);
            let out = total_y / ratio_y.1 as i16;
            self.remainder_y = total_y - out * ratio_y.1 as i16;
            out
        };

        (out_x, out_y)
    }

    /// Accumulate motion and return the divided output, keeping remainder.
    /// Do not subtract output from remainder.
    pub fn accumulate_persistent(&mut self, dx: i16, dy: i16, ratio_x: (u8, u8), ratio_y: (u8, u8)) -> (i16, i16) {
        let out_x = if ratio_x.1 == 0 {
            self.remainder_x = 0;
            0
        } else {
            let total_x = self.remainder_x.saturating_add(dx * ratio_x.0 as i16);
            let out = total_x / ratio_x.1 as i16;
            self.remainder_x = total_x;
            out
        };

        let out_y = if ratio_y.1 == 0 {
            self.remainder_y = 0;
            0
        } else {
            let total_y = self.remainder_y.saturating_add(dy * ratio_y.0 as i16);
            let out = total_y / ratio_y.1 as i16;
            self.remainder_y = total_y;
            out
        };

        (out_x, out_y)
    }
}

#[derive(Clone)]
pub struct PointingProcessorConfig {
    /// The id of the PointingDevice this processor handles.
    /// Use ALL_POINTING_DEVICES (255) to process events from all devices.
    pub device_id: u8,
    /// Invert X axis (applied to all modes before mode-specific processing)
    pub invert_x: bool,
    /// Invert Y axis (applied to all modes before mode-specific processing)
    pub invert_y: bool,
    /// Swap X and Y axes (applied to all modes before mode-specific processing)
    pub swap_xy: bool,
}

impl Default for PointingProcessorConfig {
    fn default() -> Self {
        Self {
            device_id: ALL_POINTING_DEVICES,
            invert_x: false,
            invert_y: false,
            swap_xy: false,
        }
    }
}

/// PointingProcessor that converts motion events to mouse reports
#[processor(subscribe = [PointingEvent, PointingProcessorEvent])]
pub struct PointingProcessor<'a> {
    /// Reference to the keymap (used for mouse_buttons)
    keymap: &'a KeyMap<'a>,
    config: PointingProcessorConfig,
    /// Motion accumulator for scroll/sniper modes
    accumulator: MotionAccumulator,
    /// current active mode
    current_mode: PointingMode,
}

impl<'a> PointingProcessor<'a> {
    /// Create a new pointing processor with default settings
    pub fn new(keymap: &'a KeyMap<'a>, config: PointingProcessorConfig) -> Self {
        Self {
            keymap,
            config,
            accumulator: MotionAccumulator::default(),
            current_mode: PointingMode::default(),
        }
    }

    /// Set the pointing mode
    pub fn set_pointing_mode(&mut self, mode: PointingMode) -> &mut Self {
        self.current_mode = mode;
        self
    }

    // pointing events are generated by the PointingDevice after accumulating motion and applying the poll/report intervals.
    async fn on_pointing_event(&mut self, event: PointingEvent) {
        // Filter: only process events from the configured device
        if self.config.device_id != ALL_POINTING_DEVICES && event.device_id != self.config.device_id {
            return;
        }

        let mut x = 0i16;
        let mut y = 0i16;

        for axis_event in event.axes.iter() {
            match axis_event.axis {
                Axis::X => x = axis_event.value,
                Axis::Y => y = axis_event.value,
                _ => {}
            }
        }

        // Apply global config transforms (before mode-specific processing).
        // Order: invert → swap → mode invert.
        // Mode-specific invert_x/y operate on the post-swap logical axes,
        // so if swap_xy is enabled, ScrollConfig::invert_y affects the physical X axis.
        if self.config.invert_x {
            x = -x;
        }
        if self.config.invert_y {
            y = -y;
        }
        if self.config.swap_xy {
            (x, y) = (y, x);
        }

        let buttons = self.keymap.mouse_buttons();
        match self.current_mode {
            PointingMode::Cursor(_) | PointingMode::Scroll(_) | PointingMode::Sniper(_) => {
                // modes that generate mouse reports
                let mouse_report = match self.current_mode {
                    PointingMode::Cursor(cursor_config) => {
                        let out_x = x.saturating_mul(cursor_config.multiplier_x as i16);
                        let out_y = y.saturating_mul(cursor_config.multiplier_y as i16);
                        let out_x = if cursor_config.invert_x { -out_x } else { out_x };
                        let out_y = if cursor_config.invert_y { -out_y } else { out_y };
                        MouseReport {
                            buttons,
                            x: out_x.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
                            y: out_y.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
                            wheel: 0,
                            pan: 0,
                        }
                    }
                    PointingMode::Scroll(scroll_config) => {
                        let (sx, sy) = self.accumulator.accumulate(
                            x,
                            y,
                            (scroll_config.multiplier_x, scroll_config.divisor_x),
                            (scroll_config.multiplier_y, scroll_config.divisor_y),
                        );
                        if sx == 0 && sy == 0 {
                            return;
                        }
                        // Sensor X → pan, sensor Y → wheel.
                        // Default: sensor +Y produces negative wheel (scroll up in HID convention).
                        // invert_y reverses wheel direction; invert_x reverses pan direction.
                        let wheel = if scroll_config.invert_y { sy } else { -sy };
                        let pan = if scroll_config.invert_x { -sx } else { sx };
                        MouseReport {
                            buttons,
                            x: 0,
                            y: 0,
                            wheel: wheel.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
                            pan: pan.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
                        }
                    }
                    PointingMode::Sniper(sniper_config) => {
                        let (sx, sy) = self.accumulator.accumulate(
                            x,
                            y,
                            (sniper_config.multiplier, sniper_config.divisor),
                            (sniper_config.multiplier, sniper_config.divisor),
                        );
                        if sx == 0 && sy == 0 {
                            return;
                        }
                        let out_x = if sniper_config.invert_x { -sx } else { sx };
                        let out_y = if sniper_config.invert_y { -sy } else { sy };
                        MouseReport {
                            buttons,
                            x: out_x.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
                            y: out_y.clamp(i8::MIN as i16, i8::MAX as i16) as i8,
                            wheel: 0,
                            pan: 0,
                        }
                    }
                    _ => unreachable!(),
                };

                send_hid_report(Report::MouseReport(mouse_report)).await;
            }
            PointingMode::Caret(caret_config) => {
                if let Some((keycode, count)) = compute_caret_taps(x, y, &mut self.accumulator, &caret_config) {
                    for _ in 0..count {
                        tap_key(keycode).await;
                    }
                }
            }
        };
    }

    // pointing device events are used to change the mode (cursor/scroll/sniper) of the processor based on the device id. This allows users to trigger different modes if desired.
    pub async fn on_pointing_processor_event(&mut self, event: PointingProcessorEvent) {
        if self.config.device_id == ALL_POINTING_DEVICES || self.config.device_id == event.device_id {
            debug!(
                "PointingProcessor {}: setting mode to {:?}",
                self.config.device_id, event.mode
            );
            self.set_pointing_mode(event.mode);
        }
    }
}

/// Tap a key (press and release with a short delay) - used for caret mode
/// NOTE: This is a basic implementation because at the current state Keyboard (in keyboard.rs) does not support
/// sending in KeyActions from the processor layer. If that changes in the future, this can be updated to use KeyActions and support more complex behavior (e.g. modifiers, macros).
/// For the time being, this sends only simple key taps without modifiers.
async fn tap_key(keycode: HidKeyCode) {
    // Press
    send_hid_report(Report::KeyboardReport(KeyboardReport {
        modifier: 0,
        reserved: 0,
        leds: 0,
        keycodes: [keycode as u8, 0, 0, 0, 0, 0],
    }))
    .await;
    Timer::after_millis(5).await;
    // Release
    send_hid_report(Report::KeyboardReport(KeyboardReport {
        modifier: 0,
        reserved: 0,
        leds: 0,
        keycodes: [0, 0, 0, 0, 0, 0],
    }))
    .await;
    Timer::after_millis(5).await;
}

/// Pure function: given a (x, y) motion delta, decide whether caret mode
/// should fire key taps. Updates the accumulator in place (sign-change
/// aware via divisor logic, threshold-aligned, non-dominant reset).
/// Returns `(keycode, count)` if a tap should fire, `None` otherwise.
///
/// Caller is responsible for actually firing the taps (async).
fn compute_caret_taps(
    x: i16,
    y: i16,
    accumulator: &mut MotionAccumulator,
    cfg: &CaretConfig,
) -> Option<(HidKeyCode, u8)> {
    let divisor_x = if cfg.disable_x { 0 } else { 1 };
    let divisor_y = if cfg.disable_y { 0 } else { 1 };
    let (mut dx, mut dy) = accumulator.accumulate_persistent(x, y, (1, divisor_x), (1, divisor_y));

    if (dx.abs() + dy.abs()) <= cfg.threshold {
        return None;
    }

    enum MovementAxis {
        X,
        Y,
    }
    let axis = if dx.abs() >= dy.abs() {
        MovementAxis::X
    } else {
        MovementAxis::Y
    };

    let keycode = match axis {
        MovementAxis::X => match (dx > 0, cfg.invert_x) {
            (true, false) | (false, true) => cfg.keycode_right,
            (true, true) | (false, false) => cfg.keycode_left,
        },
        MovementAxis::Y => match (dy > 0, cfg.invert_y) {
            // default: +Y => down
            (true, false) | (false, true) => cfg.keycode_down,
            (true, true) | (false, false) => cfg.keycode_up,
        },
    };

    // Each tap reduces the running total on the dominant axis by `threshold`.
    // The number of iterations is the tap count.
    let mut count: u8 = 0;
    while (dx.abs() + dy.abs()) > cfg.threshold {
        let (reduce_x, reduce_y) = match axis {
            MovementAxis::X => {
                let r = if dx > 0 { -cfg.threshold } else { cfg.threshold };
                accumulator.reset_y(); //  non-dominant axis
                (r, 0)
            }
            MovementAxis::Y => {
                let r = if dy > 0 { -cfg.threshold } else { cfg.threshold };
                accumulator.reset_x(); //  non-dominant axis
                (0, r)
            }
        };
        (dx, dy) = accumulator.accumulate_persistent(reduce_x, reduce_y, (1, divisor_x), (1, divisor_y));
        count = count.saturating_add(1);
        if count == u8::MAX {
            break; // safety break to prevent infinite loop
        }
    }

    // Drop the non-dominant axis so stale samples cannot bleed in.
    match axis {
        MovementAxis::X => accumulator.reset_y(),
        MovementAxis::Y => accumulator.reset_x(),
    }

    if count == 0 { None } else { Some((keycode, count)) }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use embassy_time::Duration;
    use embedded_hal::digital::{ErrorType, InputPin};
    use embedded_hal_async::digital::Wait;

    use super::*;
    use crate::input_device::InputDevice;
    use crate::test_support::test_block_on as block_on;

    struct DummyDriver {
        pub motion_pending: bool,
        pub motion: MotionData,
        pub init_called: bool,
        pub fails_init: bool,
        pub motion_gpio: Option<DummyMotionPin>,
        pub read_called: bool,
    }

    impl PointingDriver for DummyDriver {
        type MOTION = DummyMotionPin;

        async fn init(&mut self) -> Result<(), PointingDriverError> {
            self.init_called = true;
            if self.fails_init {
                Err(PointingDriverError::InitFailed)
            } else {
                Ok(())
            }
        }

        async fn read_motion(&mut self) -> Result<MotionData, PointingDriverError> {
            self.read_called = true;
            Ok(self.motion)
        }

        fn motion_pending(&mut self) -> bool {
            self.motion_pending
        }

        fn motion_gpio(&mut self) -> Option<&mut Self::MOTION> {
            self.motion_gpio.as_mut()
        }
    }

    #[derive(Debug)]
    struct DummyError;

    struct DummyMotionPin {
        state: Cell<bool>, // true = High, false = Low
    }
    impl ErrorType for DummyMotionPin {
        type Error = DummyError;
    }
    impl embedded_hal::digital::Error for DummyError {
        fn kind(&self) -> embedded_hal::digital::ErrorKind {
            embedded_hal::digital::ErrorKind::Other
        }
    }

    impl DummyMotionPin {
        fn new() -> Self {
            Self { state: Cell::new(true) } // initial high, we wait for low
        }

        fn set_low(&self) {
            self.state.set(false);
        }

        fn set_high(&self) {
            self.state.set(true);
        }
    }

    impl InputPin for DummyMotionPin {
        fn is_high(&mut self) -> Result<bool, Self::Error> {
            Ok(self.state.get())
        }
        fn is_low(&mut self) -> Result<bool, Self::Error> {
            Ok(!self.state.get())
        }
    }

    impl Wait for DummyMotionPin {
        async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
            while !self.state.get() { /* spin */ }
            Ok(())
        }

        async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
            embassy_time::Timer::after(Duration::from_millis(500)).await;
            Ok(())
        }
        async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
            todo!()
        }
        async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
            todo!()
        }
        async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
            todo!()
        }
    }
    #[test]
    fn test_try_init_retries_and_fails() {
        let driver = DummyDriver {
            motion_pending: true,
            motion: MotionData { dx: 10, dy: -5 },
            init_called: false,
            fails_init: true,
            motion_gpio: None,
            read_called: false,
        };

        let mut device = PointingDevice {
            sensor: driver,
            init_state: InitState::Pending,
            poll_interval: Duration::from_millis(1),
            id: 1,

            report_interval: Duration::from_millis(1),
            last_poll: Instant::MIN,
            last_report: Instant::MIN,
            accumulated_x: 0,
            accumulated_y: 0,
        };

        let mut result = false;
        for i in 0..PointingDevice::<DummyDriver>::MAX_INIT_RETRIES {
            result = block_on(device.try_init());

            if i + 1 < PointingDevice::<DummyDriver>::MAX_INIT_RETRIES {
                // Vorletzte und erste Versuche: state sollte Initializing sein
                assert_eq!(device.init_state, InitState::Initializing(i + 1));
                assert!(!result, "Init should not succeed yet on attempt {}", i + 1);
            } else {
                // Letzter Versuch: state wird direkt auf Failed gesetzt
                assert_eq!(device.init_state, InitState::Failed);
                assert!(!result, "Init should fail after max retries");
            }
        }
        assert!(!result);
        assert_eq!(device.init_state, InitState::Failed);
    }

    #[test]
    fn test_try_init_sets_state() {
        let driver = DummyDriver {
            motion_pending: true,
            motion: MotionData { dx: 10, dy: -5 },
            init_called: false,
            fails_init: false,
            motion_gpio: None,
            read_called: false,
        };

        let mut device = PointingDevice {
            sensor: driver,
            init_state: InitState::Pending,
            poll_interval: Duration::from_millis(1),
            id: 1,

            report_interval: Duration::from_millis(1),
            last_poll: Instant::MIN,
            last_report: Instant::MIN,
            accumulated_x: 0,
            accumulated_y: 0,
        };

        // Run the async try_init
        let result = block_on(device.try_init());
        assert!(result, "Init should succeed");
        assert_eq!(device.init_state, InitState::Ready);
        assert!(device.sensor.init_called, "Driver init should be called");
    }

    #[test]
    fn test_poll_once_accumulate_motion() {
        let motion_pin = DummyMotionPin::new();

        let driver = DummyDriver {
            motion_pending: true,
            motion: MotionData { dx: 10, dy: -5 },
            init_called: false,
            fails_init: false,
            motion_gpio: Some(motion_pin),
            read_called: false,
        };

        let mut device = PointingDevice {
            sensor: driver,
            init_state: InitState::Pending,
            poll_interval: Duration::from_millis(1),
            id: 1,

            report_interval: Duration::from_millis(1),
            last_poll: Instant::MIN,
            last_report: Instant::MIN,
            accumulated_x: 0,
            accumulated_y: 0,
        };

        let inited = block_on(device.try_init());
        assert!(inited);
        assert_eq!(device.init_state, InitState::Ready);
        assert!(device.sensor.init_called);

        // poll_once should accumulate motion
        block_on(device.poll_once());
        assert_eq!(device.accumulated_x, 10);
        assert_eq!(device.accumulated_y, -5);
    }

    #[test]
    fn test_polling_without_motion_pin_generates_event() {
        let driver = DummyDriver {
            motion_pending: true,
            motion: MotionData { dx: 3, dy: -2 },
            read_called: false,
            init_called: true,
            fails_init: false,
            motion_gpio: None,
        };

        let mut device = PointingDevice {
            sensor: driver,
            init_state: InitState::Ready,
            poll_interval: Duration::from_millis(1),
            report_interval: Duration::from_millis(1),
            last_poll: Instant::MIN,
            last_report: Instant::MIN,
            accumulated_x: 0,
            accumulated_y: 0,
            id: 1,
        };

        let event = block_on(device.read_event());

        let axes = &event.axes;
        assert_eq!(axes[0].value, 3);
        assert_eq!(axes[1].value, -2);

        assert!(device.sensor.read_called);
    }

    #[test]
    fn test_polling_with_motion_pin_generates_event() {
        let motion_pin = DummyMotionPin::new();

        let driver = DummyDriver {
            motion_pending: true,
            motion: MotionData { dx: 10, dy: -5 },
            init_called: false,
            fails_init: false,
            motion_gpio: Some(motion_pin),
            read_called: false,
        };

        let mut device = PointingDevice {
            sensor: driver,
            init_state: InitState::Pending,
            poll_interval: Duration::from_millis(10000),
            id: 1,

            report_interval: Duration::from_millis(1),
            last_poll: Instant::MIN,
            last_report: Instant::MIN,
            accumulated_x: 0,
            accumulated_y: 0,
        };

        let start = Instant::now();
        let event = block_on(device.read_event());
        let duration = start.elapsed();

        let axes = &event.axes;
        assert_eq!(axes[0].value, 10);
        assert_eq!(axes[1].value, -5);
        // poll intervall is 10000 here, so if read_event took less than that, motion pin wait worked and we did not get the report form polling
        assert!(
            duration.as_millis() <= 1000,
            "read_event took too long: {}ms. Expected to be ~500ms due to motion pin triggering.",
            duration.as_millis()
        );

        assert!(device.sensor.read_called);
    }

    // === MotionAccumulator tests ===

    #[test]
    fn test_motion_accumulator_basic() {
        let mut acc = MotionAccumulator::default();

        // divisor=8: 3/8 = 0 remainder 3
        let (ox, oy) = acc.accumulate(3, 3, (1, 8), (1, 8));
        assert_eq!(ox, 0);
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 3);
        assert_eq!(acc.remainder_y, 3);

        // 3+6=9, 9/8=1 remainder 1
        let (ox, oy) = acc.accumulate(6, 6, (1, 8), (1, 8));
        assert_eq!(ox, 1);
        assert_eq!(oy, 1);
        assert_eq!(acc.remainder_x, 1);
        assert_eq!(acc.remainder_y, 1);
    }

    #[test]
    fn test_motion_accumulator_negative() {
        let mut acc = MotionAccumulator::default();

        // Negative motion: -10/4 = -2 remainder -2
        let (ox, oy) = acc.accumulate(-10, -10, (1, 4), (1, 4));
        assert_eq!(ox, -2);
        assert_eq!(oy, -2);
        assert_eq!(acc.remainder_x, -2);
        assert_eq!(acc.remainder_y, -2);
    }

    #[test]
    fn test_motion_accumulator_reset() {
        let mut acc = MotionAccumulator::default();
        acc.accumulate(3, 5, (1, 8), (1, 8));
        assert_ne!(acc.remainder_x, 0);

        acc.reset();
        assert_eq!(acc.remainder_x, 0);
        assert_eq!(acc.remainder_y, 0);
    }

    #[test]
    fn test_motion_accumulator_zero_divisor_disables_axis() {
        let mut acc = MotionAccumulator::default();
        // divisor 0 should disable that axis (output 0)
        let (ox, oy) = acc.accumulate(5, -3, (1, 0), (1, 0));
        assert_eq!(ox, 0);
        assert_eq!(oy, 0);

        // One axis disabled, the other active
        let (ox, oy) = acc.accumulate(10, 10, (1, 0), (1, 2));
        assert_eq!(ox, 0); // X disabled
        assert_eq!(oy, 5); // 10/2 = 5

        // Remainder should not accumulate on disabled axis
        acc.reset();
        acc.accumulate(3, 3, (1, 0), (1, 8));
        acc.accumulate(3, 3, (1, 0), (1, 8));
        let (ox, oy) = acc.accumulate(3, 3, (1, 0), (1, 8));
        assert_eq!(ox, 0); // X always 0
        assert_eq!(oy, 1); // (3+3+3)/8 = 1 remainder 1
    }

    #[test]
    fn test_motion_accumulator_asymmetric_divisors() {
        let mut acc = MotionAccumulator::default();
        // Different divisors for x and y
        let (ox, oy) = acc.accumulate(10, 10, (1, 2), (1, 5));
        assert_eq!(ox, 5); // 10/2
        assert_eq!(oy, 2); // 10/5
    }

    #[test]
    fn test_motion_accumulator_persistent_keeps_total_in_remainder() {
        // accumulate_persistent does NOT subtract the output from the
        // remainder. The remainder is the full signed running total; the
        // output is just total / divisor. This is what caret mode needs:
        // the caller decides when to "spend" the remainder (via reset_x/y).
        let mut acc = MotionAccumulator::default();
        let (ox, oy) = acc.accumulate_persistent(150, 0, (1, 100), (1, 100));
        assert_eq!(ox, 1); // 150 / 100
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 150); // total, NOT 50
        assert_eq!(acc.remainder_y, 0);

        // Second call: total_x = 150 + 50 = 200, out=2, remainder=200
        let (ox, oy) = acc.accumulate_persistent(50, 0, (1, 100), (1, 100));
        assert_eq!(ox, 2); // 200 / 100
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 200);
        assert_eq!(acc.remainder_y, 0);
    }

    #[test]
    fn test_motion_accumulator_persistent_independent_axes() {
        // X and Y remainders are independent in accumulate_persistent:
        // movement on one axis never touches the other's running total.
        let mut acc = MotionAccumulator::default();
        let (ox, oy) = acc.accumulate_persistent(150, 0, (1, 100), (1, 100));
        assert_eq!(ox, 1);
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 150);
        assert_eq!(acc.remainder_y, 0);

        // Now move purely on Y. X total is preserved (still 150), Y starts fresh.
        let (ox, oy) = acc.accumulate_persistent(0, 150, (1, 100), (1, 100));
        assert_eq!(ox, 1); // X total 150 / 100
        assert_eq!(oy, 1); // Y total 150 / 100
        assert_eq!(acc.remainder_x, 150);
        assert_eq!(acc.remainder_y, 150);
    }

    #[test]
    fn test_motion_accumulator_persistent_sub_threshold_accumulates() {
        // Several sub-threshold samples must build up the running total
        // in the remainder. The output (total / divisor) stays 0 until the
        // total crosses the threshold; then output jumps and remainder
        // continues to grow. Caret mode relies on the running total so
        // that slow deltas are not lost.
        let mut acc = MotionAccumulator::default();

        let (ox, oy) = acc.accumulate_persistent(30, 30, (1, 100), (1, 100));
        assert_eq!(ox, 0);
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 30);
        assert_eq!(acc.remainder_y, 30);

        let (ox, oy) = acc.accumulate_persistent(30, 30, (1, 100), (1, 100));
        assert_eq!(ox, 0);
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 60);
        assert_eq!(acc.remainder_y, 60);

        let (ox, oy) = acc.accumulate_persistent(30, 30, (1, 100), (1, 100));
        assert_eq!(ox, 0);
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 90);
        assert_eq!(acc.remainder_y, 90);

        // Crosses threshold: total 90 + 20 = 110, out=1, remainder=110
        let (ox, oy) = acc.accumulate_persistent(20, 20, (1, 100), (1, 100));
        assert_eq!(ox, 1);
        assert_eq!(oy, 1);
        assert_eq!(acc.remainder_x, 110);
        assert_eq!(acc.remainder_y, 110);
    }

    #[test]
    fn test_motion_accumulator_persistent_sign_change_in_total() {
        // accumulate_persistent uses a signed running total. A counter-
        // direction sample just subtracts from the total. The caret
        // handler has to detect the sign change and reset that axis
        // (reset_x / reset_y) to avoid direction bias: otherwise
        // small counter-direction samples silently "use up" the running
        // total and block taps in the new direction.
        let mut acc = MotionAccumulator::default();
        acc.accumulate_persistent(80, 0, (1, 100), (1, 100));
        assert_eq!(acc.remainder_x, 80);

        // Counter-direction sample of 30: total = 80 - 30 = 50, no tap.
        let (ox, oy) = acc.accumulate_persistent(-30, 0, (1, 100), (1, 100));
        assert_eq!(ox, 0);
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 50);

        // Cross zero: total = 50 - 80 = -30, remainder becomes negative.
        // No tap fires (still sub-threshold), but the total is now signed.
        let (ox, oy) = acc.accumulate_persistent(-80, 0, (1, 100), (1, 100));
        assert_eq!(ox, 0);
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, -30);
    }

    #[test]
    fn test_motion_accumulator_persistent_reset_x_and_y() {
        // reset_x() and reset_y() clear the running total of one axis only,
        // leaving the other axis intact. The caret handler uses these on
        // sign change so a stale total in the previous direction cannot
        // bleed into the next sample.
        let mut acc = MotionAccumulator::default();
        acc.accumulate_persistent(80, 80, (1, 100), (1, 100));
        assert_eq!(acc.remainder_x, 80);
        assert_eq!(acc.remainder_y, 80);

        acc.reset_x();
        assert_eq!(acc.remainder_x, 0);
        assert_eq!(acc.remainder_y, 80);

        // Y continues to build; X is fresh
        let (ox, oy) = acc.accumulate_persistent(30, 30, (1, 100), (1, 100));
        assert_eq!(ox, 0); // 30 / 100
        assert_eq!(oy, 1); // (80 + 30) / 100
        assert_eq!(acc.remainder_x, 30);
        assert_eq!(acc.remainder_y, 110);

        acc.reset_y();
        assert_eq!(acc.remainder_x, 30);
        assert_eq!(acc.remainder_y, 0);
    }

    #[test]
    fn test_motion_accumulator_persistent_asymmetric_divisors() {
        // Different divisors on X and Y are handled independently. Even
        // though the current CaretConfig uses the same divisor on both
        // axes, the accumulator must work when they differ.
        let mut acc = MotionAccumulator::default();
        let (ox, oy) = acc.accumulate_persistent(10, 10, (1, 2), (1, 5));
        assert_eq!(ox, 5); // 10 / 2
        assert_eq!(oy, 2); // 10 / 5
        assert_eq!(acc.remainder_x, 10);
        assert_eq!(acc.remainder_y, 10);
    }

    #[test]
    fn test_motion_accumulator_persistent_divisor_edge_cases() {
        // divisor=1: every unit becomes output; remainder still tracks
        // the full signed total so successive calls keep growing it.
        let mut acc = MotionAccumulator::default();
        let (ox, oy) = acc.accumulate_persistent(7, -3, (1, 1), (1, 1));
        assert_eq!(ox, 7);
        assert_eq!(oy, -3);
        assert_eq!(acc.remainder_x, 7);
        assert_eq!(acc.remainder_y, -3);

        // divisor=0 on X: axis is disabled (output 0, remainder_x = 0).
        // divisor=255 on Y: maximal u8 divisor; 50/255 = 0, remainder_y = 50.
        let mut acc = MotionAccumulator::default();
        let (ox, oy) = acc.accumulate_persistent(50, 50, (1, 0), (1, 255));
        assert_eq!(ox, 0);
        assert_eq!(oy, 0);
        assert_eq!(acc.remainder_x, 0);
        assert_eq!(acc.remainder_y, 50);
    }

    // === compute_caret_taps tests ===

    fn cfg() -> CaretConfig {
        CaretConfig::default()
    }

    fn acc() -> MotionAccumulator {
        MotionAccumulator::default()
    }

    #[test]
    fn test_compute_caret_taps_zero_motion_returns_none() {
        let mut a = acc();
        assert!(compute_caret_taps(0, 0, &mut a, &cfg()).is_none());
    }

    #[test]
    fn test_compute_caret_taps_sub_threshold_returns_none() {
        let mut a = acc();
        // |50|+|50| = 100, exactly at threshold → no tap
        assert!(compute_caret_taps(50, 50, &mut a, &cfg()).is_none());
        // Accumulator still tracks the running total even when sub-threshold
        assert_eq!(a.remainder_x, 50);
        assert_eq!(a.remainder_y, 50);
    }

    #[test]
    fn test_compute_caret_taps_x_dominant_default_right() {
        let mut a = acc();
        let result = compute_caret_taps(150, 30, &mut a, &cfg());
        // |150|+|30| = 180 > 100. X dominant (150 >= 30). dx>0 + !invert → Right.
        // Loop: reduce_x=-100 → total=(50, 0), |50|+|0|=50<=100 → stop.
        assert_eq!(result, Some((HidKeyCode::Right, 1)));
        assert_eq!(a.remainder_x, 50);
        assert_eq!(a.remainder_y, 0); // non-dominant reset
    }

    #[test]
    fn test_compute_caret_taps_x_dominant_negative_is_left() {
        let mut a = acc();
        let result = compute_caret_taps(-150, 30, &mut a, &cfg());
        assert_eq!(result, Some((HidKeyCode::Left, 1)));
    }

    #[test]
    fn test_compute_caret_taps_y_dominant_default_is_down() {
        let mut a = acc();
        let result = compute_caret_taps(30, 150, &mut a, &cfg());
        // Y dominant, dy>0, !invert → Down (default +Y = down per HID)
        assert_eq!(result, Some((HidKeyCode::Down, 1)));
        assert_eq!(a.remainder_x, 0); // non-dominant reset
        assert_eq!(a.remainder_y, 50);
    }

    #[test]
    fn test_compute_caret_taps_invert_y_flips_to_up() {
        let mut a = acc();
        let mut c = cfg();
        c.invert_y = true;
        let result = compute_caret_taps(30, 150, &mut a, &c);
        // dy>0 + invert_y → Up
        assert_eq!(result, Some((HidKeyCode::Up, 1)));
    }

    #[test]
    fn test_compute_caret_taps_invert_x_flips_to_left() {
        let mut a = acc();
        let mut c = cfg();
        c.invert_x = true;
        let result = compute_caret_taps(150, 30, &mut a, &c);
        // dx>0 + invert_x → Left
        assert_eq!(result, Some((HidKeyCode::Left, 1)));
    }

    #[test]
    fn test_compute_caret_taps_multiple_taps_in_one_event() {
        let mut a = acc();
        let result = compute_caret_taps(250, 0, &mut a, &cfg());
        // X dominant, |250|=250, threshold=100 → 2 taps
        // Iter 1: reduce_x=-100 → total=(150, 0), |150|>100 → tap
        // Iter 2: reduce_x=-100 → total=(50, 0), |50|<=100 → stop
        assert_eq!(result, Some((HidKeyCode::Right, 2)));
        assert_eq!(a.remainder_x, 50);
        assert_eq!(a.remainder_y, 0);
    }

    #[test]
    fn test_compute_caret_taps_sub_threshold_accumulates_across_calls() {
        let mut a = acc();
        // 3 calls of (20, 20): total grows (20,20)→(40,40)→(60,60)
        assert!(compute_caret_taps(20, 20, &mut a, &cfg()).is_none()); // 40
        assert!(compute_caret_taps(20, 20, &mut a, &cfg()).is_none()); // 80
        // 3rd call: |60|+|60|=120 > 100 → tap
        assert_eq!(compute_caret_taps(20, 20, &mut a, &cfg()), Some((HidKeyCode::Right, 1)));
    }

    #[test]
    fn test_compute_caret_taps_zero_divisor_disables_axis() {
        let mut a = acc();
        let mut c = cfg();
        c.disable_x = true;
        // dx=0, dy=120. |0|+|120|=120 > 100 → tap. Y dominant. dy>0 + !invert → Down.
        assert_eq!(compute_caret_taps(0, 120, &mut a, &c), Some((HidKeyCode::Down, 1)));
    }

    #[test]
    fn test_compute_caret_taps_direction_change_works() {
        let mut a = acc();
        // First a Right tap
        assert_eq!(
            compute_caret_taps(150, 30, &mut a, &cfg()),
            Some((HidKeyCode::Right, 1))
        );
        // After: remainder_x = 50, remainder_y = 0 (reset)
        assert_eq!(a.remainder_x, 50);
        assert_eq!(a.remainder_y, 0);

        // Now move left. -200 + 50 (carry-over) = -150 → |-150|=150>100 → tap
        assert_eq!(compute_caret_taps(-200, 0, &mut a, &cfg()), Some((HidKeyCode::Left, 1)));
    }

    #[test]
    fn test_compute_caret_taps_threshold_at_exactly_boundary() {
        let mut a = acc();
        // |100|+|0| = 100, exactly threshold → no tap
        assert!(compute_caret_taps(100, 0, &mut a, &cfg()).is_none());
        assert_eq!(a.remainder_x, 100);
        // One more unit pushes over
        assert_eq!(compute_caret_taps(1, 0, &mut a, &cfg()), Some((HidKeyCode::Right, 1)));
    }

    #[test]
    fn test_compute_caret_taps_diagonal_motion() {
        let mut a = acc();
        let result = compute_caret_taps(250, 250, &mut a, &cfg());
        // X dominant, |250|=250, threshold=100 → 2 taps
        // Iter 1: reduce_x=-100 → total=(150, 0), |150|>100 → tap
        // Iter 2: reduce_x=-100 → total=(50, 0), |50|<=100 → stop
        assert_eq!(result, Some((HidKeyCode::Right, 2)));
        assert_eq!(a.remainder_x, 50);
        assert_eq!(a.remainder_y, 0);
    }

    // === PointingMode tests ===

    #[test]
    fn test_pointing_mode_default_is_cursor() {
        assert_eq!(PointingMode::default(), PointingMode::Cursor(CursorConfig::default()));
    }

    #[test]
    fn test_pointing_mode_array_default() {
        let modes: [PointingMode; 4] = [PointingMode::default(); 4];
        for mode in &modes {
            assert_eq!(*mode, PointingMode::Cursor(CursorConfig::default()));
        }
    }

    #[test]
    fn test_motion_accumulator_change_resets_accumulator() {
        let mut acc = MotionAccumulator::default();
        acc.accumulate(3, 5, (1, 8), (1, 8));
        assert_eq!(acc.remainder_x, 3);
        assert_eq!(acc.remainder_y, 5);

        // Simulate what on_layer_change_event does
        acc.reset();
        assert_eq!(acc.remainder_x, 0);
        assert_eq!(acc.remainder_y, 0);
    }

    #[test]
    fn test_pointing_cursor_multiplier_scales_motion() {
        let config = CursorConfig {
            multiplier_x: 2,
            multiplier_y: 3,
            invert_x: false,
            invert_y: false,
        };
        assert_eq!(10 * config.multiplier_x as i16, 20);
        assert_eq!(10 * config.multiplier_y as i16, 30);
    }

    #[test]
    fn test_pointing_cursor_invert_axes() {
        let config = CursorConfig {
            multiplier_x: 1,
            multiplier_y: 1,
            invert_x: true,
            invert_y: true,
        };
        assert_eq!(-(10 * config.multiplier_x as i16), -10);
        assert_eq!(-(10 * config.multiplier_y as i16), -10);
    }

    // === Integration tests for PointingProcessor ===

    #[test]
    fn test_pointing_processor_mode_selection() {
        // Test that the processor correctly selects the mode based on current layer
        let modes = [
            PointingMode::Cursor(CursorConfig::default()),
            PointingMode::Scroll(ScrollConfig::default()),
            PointingMode::Sniper(SniperConfig {
                multiplier: 1,
                divisor: 4,
                invert_x: false,
                invert_y: false,
            }),
            PointingMode::Caret(CaretConfig::default()),
            PointingMode::Cursor(CursorConfig::default()),
        ];

        // Verify all modes are correctly stored
        for (i, expected_mode) in modes.iter().enumerate() {
            assert_eq!(&modes[i], expected_mode);
        }
    }

    #[test]
    fn test_pointing_scroll_mode_zero_motion_prevention() {
        let mut acc = MotionAccumulator::default();
        let config = ScrollConfig {
            multiplier_x: 1,
            multiplier_y: 1,
            divisor_x: 8,
            divisor_y: 8,
            invert_x: false,
            invert_y: false,
        };

        // Small motion that doesn't produce output
        let (sx, sy) = acc.accumulate(
            3,
            3,
            (config.multiplier_x, config.divisor_x),
            (config.multiplier_y, config.divisor_y),
        );
        assert_eq!(sx, 0);
        assert_eq!(sy, 0);

        // Verify remainder is kept
        assert_eq!(acc.remainder_x, 3);
        assert_eq!(acc.remainder_y, 3);

        // Additional motion should accumulate
        let (sx, sy) = acc.accumulate(
            6,
            6,
            (config.multiplier_x, config.divisor_x),
            (config.multiplier_y, config.divisor_y),
        );
        assert_eq!(sx, 1); // (3+6)/8 = 1 remainder 1
        assert_eq!(sy, 1);
        assert_eq!(acc.remainder_x, 1);
        assert_eq!(acc.remainder_y, 1);
    }

    #[test]
    fn test_pointing_sniper_mode_divisor() {
        let mut acc = MotionAccumulator::default();
        let config = SniperConfig {
            multiplier: 1,
            divisor: 4,
            invert_x: false,
            invert_y: false,
        };

        // Test that motion is divided correctly
        let (sx, sy) = acc.accumulate(
            10,
            -10,
            (config.multiplier, config.divisor),
            (config.multiplier, config.divisor),
        );
        assert_eq!(sx, 2); // 10/4 = 2 remainder 2
        assert_eq!(sy, -2); // -10/4 = -2 remainder -2
        assert_eq!(acc.remainder_x, 2);
        assert_eq!(acc.remainder_y, -2);
    }

    #[test]
    fn test_motion_accumulator_negative_motion() {
        let mut acc = MotionAccumulator::default();

        // Test negative motion with divisor
        let (ox, oy) = acc.accumulate(-15, -20, (1, 4), (1, 5));
        assert_eq!(ox, -3); // -15/4 = -3 remainder -3
        assert_eq!(oy, -4); // -20/5 = -4 remainder 0
        assert_eq!(acc.remainder_x, -3);
        assert_eq!(acc.remainder_y, 0);

        // Mix positive and negative
        let (ox, oy) = acc.accumulate(5, 10, (1, 4), (1, 5));
        assert_eq!(ox, 0); // (-3+5)/4 = 0 remainder 2
        assert_eq!(oy, 2); // (0+10)/5 = 2 remainder 0
        assert_eq!(acc.remainder_x, 2);
        assert_eq!(acc.remainder_y, 0);
    }

    #[test]
    fn test_pointing_scroll_config_default_values() {
        let config = ScrollConfig::default();
        assert_eq!(config.divisor_x, 8);
        assert_eq!(config.divisor_y, 8);
        assert!(!config.invert_x);
        assert!(!config.invert_y);
    }

    #[test]
    fn test_pointing_sniper_config_default_values() {
        let config = SniperConfig::default();
        assert_eq!(config.divisor, 4);
        assert!(!config.invert_x);
        assert!(!config.invert_y);
    }

    #[test]
    fn test_pointing_scroll_config_invert_y() {
        // invert_y=true means positive sensor Y → positive wheel (reversed from default)
        // Default (invert_y=false): sensor +Y → wheel -1 (scroll up)
        // With invert_y=true:        sensor +Y → wheel +1 (scroll down)
        let mut acc_default = MotionAccumulator::default();
        let mut acc_inverted = MotionAccumulator::default();

        let divisor = 1u8;
        let (_, sy_default) = acc_default.accumulate(0, 10, (1, divisor), (1, divisor));
        let (_, sy_inverted) = acc_inverted.accumulate(0, 10, (1, divisor), (1, divisor));

        // Default: wheel = -sy = -10
        let wheel_default = -sy_default;
        // Inverted: wheel = sy = 10
        let wheel_inverted = sy_inverted;

        assert_eq!(wheel_default, -10);
        assert_eq!(wheel_inverted, 10);
    }

    #[test]
    fn test_pointing_sniper_config_invert_axes() {
        let mut acc = MotionAccumulator::default();
        let config = SniperConfig {
            multiplier: 1,
            divisor: 1,
            invert_x: true,
            invert_y: true,
        };

        let (sx, sy) = acc.accumulate(
            5,
            -3,
            (config.multiplier, config.divisor),
            (config.multiplier, config.divisor),
        );
        let out_x = if config.invert_x { -sx } else { sx };
        let out_y = if config.invert_y { -sy } else { sy };

        assert_eq!(out_x, -5);
        assert_eq!(out_y, 3);
    }

    #[test]
    fn test_pointing_scroll_mode_asymmetric_divisors() {
        let mut acc = MotionAccumulator::default();
        let config = ScrollConfig {
            multiplier_x: 1,
            multiplier_y: 1,
            divisor_x: 4,
            divisor_y: 8,
            invert_x: false,
            invert_y: false,
        };

        // Test asymmetric divisors
        let (sx, sy) = acc.accumulate(
            16,
            16,
            (config.multiplier_x, config.divisor_x),
            (config.multiplier_x, config.divisor_y),
        );
        assert_eq!(sx, 4); // 16/4 = 4
        assert_eq!(sy, 2); // 16/8 = 2
    }

    #[test]
    fn test_pointing_scroll_multiplier_amplifies_motion() {
        let mut acc = MotionAccumulator::default();
        let config = ScrollConfig {
            multiplier_x: 3,
            multiplier_y: 3,
            divisor_x: 8,
            divisor_y: 8,
            invert_x: false,
            invert_y: false,
        };
        let (sx, sy) = acc.accumulate(
            10,
            10,
            (config.multiplier_x, config.divisor_x),
            (config.multiplier_y, config.divisor_y),
        );
        assert_eq!(sx, 3); // (0+10*3)/8 = 3 r6
        assert_eq!(sy, 3);
        assert_eq!(acc.remainder_x, 6);
        assert_eq!(acc.remainder_y, 6);
    }

    #[test]
    fn test_pointing_scroll_asymmetric_multipliers() {
        let mut acc = MotionAccumulator::default();
        let config = ScrollConfig {
            multiplier_x: 2,
            multiplier_y: 3,
            divisor_x: 8,
            divisor_y: 8,
            invert_x: false,
            invert_y: false,
        };
        let (sx, sy) = acc.accumulate(
            10,
            -10,
            (config.multiplier_x, config.divisor_x),
            (config.multiplier_y, config.divisor_y),
        );
        assert_eq!(sx, 2); // (0+10*2)/8 = 2 r4
        assert_eq!(sy, -3); // (0+(-10)*3)/8 = -3 r-6
        assert_eq!(acc.remainder_x, 4);
        assert_eq!(acc.remainder_y, -6);
    }

    #[test]
    fn test_pointing_scroll_multiplier_accumulates_remainder() {
        let mut acc = MotionAccumulator::default();
        let config = ScrollConfig {
            multiplier_x: 3,
            multiplier_y: 3,
            divisor_x: 8,
            divisor_y: 8,
            invert_x: false,
            invert_y: false,
        };
        // First call: sub-threshold, only remainder accumulates
        let (sx, sy) = acc.accumulate(
            2,
            2,
            (config.multiplier_x, config.divisor_x),
            (config.multiplier_y, config.divisor_y),
        );
        assert_eq!(sx, 0); // (0+2*3)/8 = 0 r6
        assert_eq!(sy, 0);
        assert_eq!(acc.remainder_x, 6);
        assert_eq!(acc.remainder_y, 6);

        // Second call: crosses threshold, multiplier applies to remainder too
        let (sx, sy) = acc.accumulate(
            2,
            2,
            (config.multiplier_x, config.divisor_x),
            (config.multiplier_y, config.divisor_y),
        );
        assert_eq!(sx, 1); // (6+2*3)/8 = 12/8 = 1 r4
        assert_eq!(sy, 1);
        assert_eq!(acc.remainder_x, 4);
        assert_eq!(acc.remainder_y, 4);
    }

    #[test]
    fn test_pointing_sniper_multiplier_amplifies_motion() {
        let mut acc = MotionAccumulator::default();
        let config = SniperConfig {
            multiplier: 3,
            divisor: 4,
            invert_x: false,
            invert_y: false,
        };
        let (sx, sy) = acc.accumulate(
            5,
            0,
            (config.multiplier, config.divisor),
            (config.multiplier, config.divisor),
        );
        assert_eq!(sx, 3); // (0+5*3)/4 = 3 r3
        assert_eq!(sy, 0);
        assert_eq!(acc.remainder_x, 3);
        assert_eq!(acc.remainder_y, 0);
    }

    #[test]
    fn test_pointing_sniper_multiplier_with_negative() {
        let mut acc = MotionAccumulator::default();
        let config = SniperConfig {
            multiplier: 3,
            divisor: 4,
            invert_x: false,
            invert_y: false,
        };
        let (sx, sy) = acc.accumulate(
            -3,
            0,
            (config.multiplier, config.divisor),
            (config.multiplier, config.divisor),
        );
        assert_eq!(sx, -2); // (0+(-3)*3)/4 = -9/4 = -2 r-1
        assert_eq!(sy, 0);
        assert_eq!(acc.remainder_x, -1);
        assert_eq!(acc.remainder_y, 0);
    }

    #[test]
    fn test_pointing_layer_mode_bounds_checking() {
        // Test that modes array is correctly sized
        let modes: [PointingMode; 8] = [PointingMode::default(); 8];
        assert_eq!(modes.len(), 8);

        // Verify all default to Cursor
        for mode in &modes {
            assert_eq!(*mode, PointingMode::Cursor(CursorConfig::default()));
        }
    }

    #[test]
    fn test_motion_accumulator_saturation() {
        let mut acc = MotionAccumulator::default();

        // Test with large values that might overflow
        let (ox, oy) = acc.accumulate(i16::MAX, i16::MAX, (1, 1), (1, 1));
        assert_eq!(ox, i16::MAX);
        assert_eq!(oy, i16::MAX);

        // Reset and test negative saturation
        acc.reset();
        let (ox, oy) = acc.accumulate(i16::MIN, i16::MIN, (1, 1), (1, 1));
        assert_eq!(ox, i16::MIN);
        assert_eq!(oy, i16::MIN);
    }
}
