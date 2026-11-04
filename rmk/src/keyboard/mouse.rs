//! Mouse keys sample the current speed at input events and repeat deadlines.

use embassy_time::{Duration, Instant};
use rmk_types::keycode::HidKeyCode;

use crate::config::MouseKeyConfig;
use crate::event::{KeyboardEvent, KeyboardEventSource};
use crate::hid::MouseReport;

const INTERVAL: Duration = Duration::from_millis(20);
// A stalled HID writer must not replay the entire pause as one large motion.
const MAX_ELAPSED_MS: u64 = 4 * INTERVAL.as_millis();
// Milliseconds, quarter-speed gears, and a 181/256 diagonal projection share this denominator.
const UNITS_PER_COUNT: u64 = 1000 * 4 * 256;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Speed {
    Normal,
    Slow,
    Medium,
    Sprint,
}

#[derive(Clone, Copy)]
enum Rate {
    Ramp(MouseKeyConfig),
    /// Quarter-counts per second, also representing wheel rates below one unit per second.
    Fixed(u32),
}

impl Speed {
    fn rates(self, config: MouseKeyConfig) -> (Rate, Rate) {
        let cursor = u32::from(config.max_speed());
        let wheel = u32::from(config.scroll_speed());
        match self {
            Self::Normal => (Rate::Ramp(config), Rate::Fixed(4 * wheel)),
            Self::Slow => (Rate::Fixed(cursor), Rate::Fixed(wheel)),
            Self::Medium => (Rate::Fixed(2 * cursor), Rate::Fixed(4 * wheel)),
            Self::Sprint => (Rate::Fixed(8 * cursor), Rate::Fixed(8 * wheel)),
        }
    }
}

/// A negative/positive net holder count selects the negative/positive direction of each axis.
struct Motion {
    held: [i8; 2],
    /// Fractional distance along each axis, cleared when its direction changes.
    remainder: [u32; 2],
    started_ms: u64,
    last_update: Instant,
}

impl Motion {
    const IDLE: Self = Self {
        held: [0; 2],
        remainder: [0; 2],
        started_ms: 0,
        last_update: Instant::MIN,
    };

    fn deadline(&self) -> Option<Instant> {
        (self.held != [0, 0]).then(|| self.last_update + INTERVAL)
    }

    /// Settle motion under the old direction and rate up to this event.
    fn advance(&mut self, now: Instant, rate: Rate) -> [i16; 2] {
        if self.held == [0, 0] {
            return [0, 0];
        }
        let end = now.as_millis();
        let begin = self.last_update.as_millis().max(end.saturating_sub(MAX_ELAPSED_MS));
        self.last_update = now;
        let elapsed = end - begin;
        let quarter_millicounts = match rate {
            Rate::Fixed(quarters_per_second) => u64::from(quarters_per_second) * elapsed,
            Rate::Ramp(config) => {
                let initial = u32::from(config.initial_speed());
                let max = u32::from(config.max_speed());
                let duration = u32::from(config.acceleration_time_ms());
                let age = (end - self.started_ms).min(u64::from(duration)) as u32;
                let speed = if age < duration {
                    initial + (max - initial) * age / duration
                } else {
                    max
                };
                4 * u64::from(speed) * elapsed
            }
        };

        let projection = if self.held[0] != 0 && self.held[1] != 0 {
            181
        } else {
            256
        };
        core::array::from_fn(|axis| {
            let direction = i16::from(self.held[axis].signum());
            if direction == 0 {
                return 0;
            }
            let total = u64::from(self.remainder[axis]) + quarter_millicounts * projection;
            self.remainder[axis] = (total % UNITS_PER_COUNT) as u32;
            // Even at 2 × u16::MAX counts/s an 80 ms interval fits in a signed 16-bit HID axis.
            direction * (total / UNITS_PER_COUNT) as i16
        })
    }

    fn direction(&mut self, axis: usize, sign: i8, pressed: bool, now: Instant, rate: Rate) -> [i16; 2] {
        let current = self.held[axis];
        let next = current.saturating_add(if pressed { sign } else { -sign });
        if current.signum() == next.signum() {
            self.held[axis] = next;
            return [0, 0];
        }

        let was_idle = self.held == [0, 0];
        let mut delta = self.advance(now, rate);
        self.held[axis] = next;
        self.remainder[axis] = 0;
        if was_idle {
            self.started_ms = now.as_millis();
            self.last_update = now;
            if pressed {
                delta[axis] = i16::from(sign);
            }
        }
        delta
    }
}

pub(crate) struct MouseState {
    pub buttons: u8,
    speed_keys: [u8; 3],
    cursor: Motion,
    wheel: Motion,
}

impl MouseState {
    pub fn new() -> Self {
        Self {
            buttons: 0,
            speed_keys: [0; 3],
            cursor: Motion::IDLE,
            wheel: Motion::IDLE,
        }
    }

    fn speed(&self) -> Speed {
        match self.speed_keys {
            [0, 0, 0] => Speed::Normal,
            [0, 0, _] => Speed::Sprint,
            [0, _, _] => Speed::Medium,
            _ => Speed::Slow,
        }
    }

    pub fn process(
        &mut self,
        key: HidKeyCode,
        event: KeyboardEvent,
        now: Instant,
        config: MouseKeyConfig,
    ) -> Option<MouseReport> {
        use HidKeyCode::*;
        let pressed = event.pressed;
        let old_speed = self.speed();
        let (cursor_rate, wheel_rate) = old_speed.rates(config);
        let mut report = MouseReport {
            buttons: self.buttons,
            ..Default::default()
        };
        let (wheel, axis, sign) = match key {
            MouseLeft => (false, 0, -1),
            MouseRight => (false, 0, 1),
            MouseUp => (false, 1, -1),
            MouseDown => (false, 1, 1),
            MouseWheelLeft => (true, 0, -1),
            MouseWheelRight => (true, 0, 1),
            MouseWheelUp => (true, 1, 1),
            MouseWheelDown => (true, 1, -1),
            MouseAccel0 | MouseAccel1 | MouseAccel2 => {
                let count = &mut self.speed_keys[(key as u8 - MouseAccel0 as u8) as usize];
                *count = if pressed {
                    count.saturating_add(1)
                } else {
                    count.saturating_sub(1)
                };
                let speed = self.speed();
                if speed == old_speed {
                    return None;
                }
                [report.x, report.y] = self.cursor.advance(now, cursor_rate);
                [report.pan, report.wheel] = self.wheel.advance(now, wheel_rate);
                if speed == Speed::Normal && matches!(old_speed, Speed::Slow | Speed::Medium) {
                    self.cursor.started_ms = now.as_millis();
                }
                return motion_report(report);
            }
            key if (MouseBtn1..=MouseBtn8).contains(&key) => {
                let mask = 1 << (key as u8 - MouseBtn1 as u8);
                self.buttons = if pressed {
                    self.buttons | mask
                } else {
                    self.buttons & !mask
                };
                if report.buttons == self.buttons {
                    return None;
                }
                report.buttons = self.buttons;
                return Some(report);
            }
            _ => return None,
        };

        if wheel && matches!(event.source, KeyboardEventSource::RotaryEncoder(_)) {
            if !pressed {
                return None;
            }
            let mut step = [0; 2];
            step[axis] = i16::from(sign);
            [report.pan, report.wheel] = step;
        } else if wheel {
            [report.pan, report.wheel] = self.wheel.direction(axis, sign, pressed, now, wheel_rate);
        } else {
            [report.x, report.y] = self.cursor.direction(axis, sign, pressed, now, cursor_rate);
        }
        motion_report(report)
    }

    pub fn fire_repeats(&mut self, now: Instant, config: MouseKeyConfig) -> Option<MouseReport> {
        let (cursor_rate, wheel_rate) = self.speed().rates(config);
        let mut report = MouseReport {
            buttons: self.buttons,
            ..Default::default()
        };
        if self.cursor.deadline().is_some_and(|deadline| deadline <= now) {
            [report.x, report.y] = self.cursor.advance(now, cursor_rate);
        }
        if self.wheel.deadline().is_some_and(|deadline| deadline <= now) {
            [report.pan, report.wheel] = self.wheel.advance(now, wheel_rate);
        }
        motion_report(report)
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        match (self.cursor.deadline(), self.wheel.deadline()) {
            (Some(cursor), Some(wheel)) => Some(cursor.min(wheel)),
            (cursor, wheel) => cursor.or(wheel),
        }
    }
}

fn motion_report(report: MouseReport) -> Option<MouseReport> {
    (report.x != 0 || report.y != 0 || report.pan != 0 || report.wheel != 0).then_some(report)
}

#[cfg(test)]
mod tests {
    use HidKeyCode::{MouseAccel0, MouseAccel1, MouseAccel2, MouseDown, MouseLeft, MouseRight, MouseWheelDown};

    use super::*;

    impl MouseState {
        fn key(&mut self, key: HidKeyCode, pressed: bool, ms: u64, config: MouseKeyConfig) -> Option<MouseReport> {
            self.process(
                key,
                KeyboardEvent::keycode(key, pressed),
                Instant::from_millis(ms),
                config,
            )
        }

        fn tick(&mut self, ms: u64, config: MouseKeyConfig) -> Option<MouseReport> {
            self.fire_repeats(Instant::from_millis(ms), config)
        }
    }

    #[test]
    fn ramp_samples_speed_at_each_update_including_release() {
        // A 50 ms press sampled at 20, 40 and 50 ms starts with one count, then
        // adds 120 × .020 + 240 × .020 + 300 × .010 = 10.2 counts from a zero start.
        for (initial, hold, interval, expected) in [
            (0, 1, 20, 1),
            (150, 1, 20, 1),
            (0, 50, 20, 11),
            (150, 50, 20, 17),
            (0, 50, 37, 13), // a late tick, then 13 ms of motion on release
            (0, 200, 20, 133),
            (0, 207, 37, 150), // the final interval crosses the end of the ramp
            (0, 1000, 20, 1093),
        ] {
            let cfg = MouseKeyConfig::new(initial, 1200, 200, 10);
            let mut mouse = MouseState::new();
            let mut x = mouse.key(MouseRight, true, 0, cfg).unwrap().x;
            for t in (interval as u64..hold).step_by(interval) {
                x += mouse.tick(t, cfg).map_or(0, |r| r.x);
            }
            x += mouse.key(MouseRight, false, hold, cfg).map_or(0, |r| r.x);
            assert_eq!(x, expected, "initial={initial}, hold={hold}, interval={interval}");
            assert_eq!(mouse.next_deadline(), None);
        }
    }

    #[test]
    fn sprint_doubles_cruising_speed_and_releases_to_the_normal_curve() {
        let cfg = MouseKeyConfig::new(0, 1200, 200, 10);
        let mut mouse = MouseState::new();
        mouse.key(MouseAccel2, true, 0, cfg);
        mouse.key(MouseRight, true, 0, cfg);
        for t in (20..=100).step_by(20) {
            assert_eq!(mouse.tick(t, cfg).unwrap().x, 48);
        }
        assert!(mouse.key(MouseAccel2, false, 100, cfg).is_none());
        assert_eq!(mouse.tick(120, cfg).unwrap().x, 14); // 720 counts/s for the last 20 ms
        for t in (140..=200).step_by(20) {
            mouse.tick(t, cfg);
        }
        assert_eq!(mouse.tick(220, cfg).unwrap().x, 24);
        mouse.key(MouseAccel2, true, 220, cfg);
        assert_eq!(mouse.tick(240, cfg).unwrap().x, 48);
        mouse.key(MouseAccel2, false, 240, cfg);
        assert_eq!(mouse.tick(260, cfg).unwrap().x, 24);
    }

    #[test]
    fn held_speed_keys_select_the_slowest_then_restart_from_precision() {
        let cfg = MouseKeyConfig::new(0, 1200, 200, 10);
        let mut mouse = MouseState::new();
        mouse.key(MouseRight, true, 0, cfg);
        for key in [MouseAccel2, MouseAccel1, MouseAccel0, MouseAccel0] {
            mouse.key(key, true, 0, cfg);
        }
        mouse.key(MouseAccel0, false, 0, cfg);
        assert_eq!(mouse.tick(20, cfg).unwrap().x, 6);
        mouse.key(MouseAccel0, false, 20, cfg);
        assert_eq!(mouse.tick(40, cfg).unwrap().x, 12);
        mouse.key(MouseAccel2, false, 40, cfg);
        mouse.key(MouseAccel1, false, 40, cfg);
        assert_eq!(mouse.tick(60, cfg).unwrap().x, 2); // 20 ms after restarting the ramp
    }

    #[test]
    fn slow_scrolling_accumulates_subunits_and_stops_on_release() {
        let cfg = MouseKeyConfig::new(0, 600, 200, 1);
        let mut mouse = MouseState::new();
        mouse.key(MouseAccel0, true, 0, cfg);
        assert_eq!(mouse.key(MouseWheelDown, true, 0, cfg).unwrap().wheel, -1);
        for t in (20..4000).step_by(20) {
            assert!(mouse.tick(t, cfg).is_none());
        }
        assert_eq!(mouse.tick(4000, cfg).unwrap().wheel, -1);
        assert!(mouse.key(MouseWheelDown, false, 4000, cfg).is_none());
        assert!(mouse.tick(8000, cfg).is_none());
    }

    #[test]
    fn cursor_and_pan_report_only_the_axes_due_at_each_deadline() {
        let cfg = MouseKeyConfig::new(0, 1000, 0, 100);
        let mut mouse = MouseState::new();
        mouse.key(MouseRight, true, 0, cfg);
        let first_pan = mouse.key(HidKeyCode::MouseWheelLeft, true, 10, cfg).unwrap();
        assert_eq!((first_pan.x, first_pan.pan), (0, -1));
        for (ms, expected) in [(20, (20, 0)), (30, (0, -2)), (55, (35, -2))] {
            let r = mouse.tick(ms, cfg).unwrap();
            assert_eq!((r.x, r.pan), expected);
        }
    }

    #[test]
    fn diagonal_travel_keeps_its_speed_and_opposite_keys_cancel() {
        let cfg = MouseKeyConfig::new(1000, 1000, 0, 10);
        let mut mouse = MouseState::new();
        mouse.key(MouseLeft, true, 0, cfg);
        mouse.key(MouseLeft, true, 0, cfg);
        mouse.key(MouseLeft, false, 0, cfg);
        assert!(mouse.key(MouseDown, true, 0, cfg).is_none());
        let (mut x, mut y) = (0, 0);
        for t in (20..=1000).step_by(20) {
            let r = mouse.tick(t, cfg).unwrap();
            x += r.x;
            y += r.y;
        }
        assert_eq!((x, y), (-707, 707));
        mouse.key(MouseDown, false, 1000, cfg);
        mouse.key(MouseRight, true, 1000, cfg);
        assert_eq!(mouse.next_deadline(), None);
        assert!(mouse.key(MouseLeft, false, 1020, cfg).is_none());
        assert_eq!(mouse.tick(1040, cfg).unwrap().x, 20);
    }

    #[test]
    fn late_wakeup_limits_catch_up_at_full_range_cursor_and_scroll_speeds() {
        let cfg = MouseKeyConfig::new(0, u16::MAX, u16::MAX, u16::MAX);
        // One millisecond before the ramp ends, the cursor does 65_534 counts/s,
        // or 131_070 in sprint. Only the last 80 ms belong in this report.
        for (sprint, expected) in [(false, 5242), (true, 10485)] {
            let mut mouse = MouseState::new();
            if sprint {
                mouse.key(MouseAccel2, true, 0, cfg);
            }
            mouse.key(MouseRight, true, 0, cfg);
            mouse.key(MouseWheelDown, true, 0, cfg);
            let r = mouse.tick(65_534, cfg).unwrap();
            assert_eq!((r.x, r.wheel), (expected, -expected));
            assert!(mouse.tick(65_534, cfg).is_none());
            assert_eq!(mouse.next_deadline(), Some(Instant::from_millis(65_554)));
        }
    }
}
