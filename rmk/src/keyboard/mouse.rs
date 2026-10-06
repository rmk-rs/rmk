//! Mouse key state machine and HID report generation.
//!
//! This module tracks movement, wheel, acceleration, and button states, then
//! derives `MouseReport` values from key press/release events.
//! Pressing a mouse direction/wheel key activates an automatic repeat mechanism:
//! it schedules per-category repeat ticks and updates movement speed from
//! `MouseKeyConfig` as repeat count increases.

use embassy_time::{Duration, Instant};
use rmk_types::keycode::HidKeyCode;

use crate::config::MouseKeyConfig;
use crate::hid::MouseReport;

/// Result of processing a mouse key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseAction {
    /// Send report.
    SendReport,
    /// No report needed.
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy)]
enum MouseCategory {
    /// Mouse movement
    Movement(Direction),
    /// Mouse wheel movement
    Wheel(Direction),
    /// Accel bit
    Accel(u8),
    /// Mouse button index
    Button(u8),
}

impl TryFrom<HidKeyCode> for MouseCategory {
    type Error = ();

    fn try_from(value: HidKeyCode) -> Result<Self, Self::Error> {
        match value {
            HidKeyCode::MouseUp => Ok(MouseCategory::Movement(Direction::Up)),
            HidKeyCode::MouseDown => Ok(MouseCategory::Movement(Direction::Down)),
            HidKeyCode::MouseLeft => Ok(MouseCategory::Movement(Direction::Left)),
            HidKeyCode::MouseRight => Ok(MouseCategory::Movement(Direction::Right)),
            HidKeyCode::MouseWheelUp => Ok(MouseCategory::Wheel(Direction::Up)),
            HidKeyCode::MouseWheelDown => Ok(MouseCategory::Wheel(Direction::Down)),
            HidKeyCode::MouseWheelLeft => Ok(MouseCategory::Wheel(Direction::Left)),
            HidKeyCode::MouseWheelRight => Ok(MouseCategory::Wheel(Direction::Right)),

            HidKeyCode::MouseAccel0 => Ok(MouseCategory::Accel(1 << 0)),
            HidKeyCode::MouseAccel1 => Ok(MouseCategory::Accel(1 << 1)),
            HidKeyCode::MouseAccel2 => Ok(MouseCategory::Accel(1 << 2)),
            HidKeyCode::MouseBtn1 => Ok(MouseCategory::Button(0)),
            HidKeyCode::MouseBtn2 => Ok(MouseCategory::Button(1)),
            HidKeyCode::MouseBtn3 => Ok(MouseCategory::Button(2)),
            HidKeyCode::MouseBtn4 => Ok(MouseCategory::Button(3)),
            HidKeyCode::MouseBtn5 => Ok(MouseCategory::Button(4)),
            HidKeyCode::MouseBtn6 => Ok(MouseCategory::Button(5)),
            HidKeyCode::MouseBtn7 => Ok(MouseCategory::Button(6)),
            HidKeyCode::MouseBtn8 => Ok(MouseCategory::Button(7)),
            _ => Err(()),
        }
    }
}

/// Per-category (movement or wheel) axis state, repeat counter and deadline.
///
/// `x` and `y` track the net number of pressed keys per axis:
///   - Right/WheelRight increments `x`, Left/WheelLeft decrements `x`
///   - Down/WheelDown increments `y`, Up/WheelUp decrements `y`
///
/// When opposite directions cancel out (e.g. Left+Right → x=0), the category
/// becomes inactive and repeat stops. Direction is extracted via `signum()`.
#[derive(Default)]
struct DirectionState {
    x: i8,
    y: i8,
    repeat: u8,
    deadline: Option<Instant>,
}

impl DirectionState {
    /// Update direction state and manage repeat scheduling.
    fn update(&mut self, direction: Direction, pressed: bool, repeat_delay: u16) {
        let was_active = self.has_active_direction();

        // Update x/y value according to direction and the pressed state
        let delta: i8 = if pressed { 1 } else { -1 };
        match direction {
            Direction::Right => self.x = self.x.saturating_add(delta),
            Direction::Left => self.x = self.x.saturating_sub(delta),
            Direction::Down => self.y = self.y.saturating_add(delta),
            Direction::Up => self.y = self.y.saturating_sub(delta),
        }

        let now_active = self.has_active_direction();

        match (was_active, now_active) {
            (false, true) => {
                // Start at 1: the initial press counts as the first occurrence,
                // so fire_repeats sees repeat >= 1 and uses repeat_interval_ms.
                self.repeat = 1;
                self.deadline = Some(Instant::now() + Duration::from_millis(repeat_delay as u64));
            }
            (true, false) => {
                self.repeat = 0;
                self.deadline = None;
            }
            _ => {}
        }
    }

    /// Returns `true` if this `DirectionState` is active, i.e. at least one direction has movement.
    fn has_active_direction(&self) -> bool {
        self.x != 0 || self.y != 0
    }

    /// Compute axis values by applying `unit` magnitude to the active directions.
    fn axis_values(&self, unit: i16) -> (i16, i16) {
        (
            (self.x.signum() as i16).saturating_mul(unit),
            (self.y.signum() as i16).saturating_mul(unit),
        )
    }

    /// Handle a repeat tick: increment repeat counter and schedule next deadline.
    fn on_repeat_tick(&mut self, ticks_to_max: u8, repeat_delay: u16) {
        if self.repeat < ticks_to_max {
            self.repeat += 1;
        }
        self.deadline = Some(Instant::now() + Duration::from_millis(repeat_delay as u64));
    }
}

pub(crate) struct MouseState {
    pub report: MouseReport,
    accel: u8,
    movement: DirectionState,
    wheel: DirectionState,
}

impl Default for MouseState {
    fn default() -> Self {
        Self::new()
    }
}

impl MouseState {
    pub fn new() -> Self {
        MouseState {
            report: MouseReport {
                buttons: 0,
                x: 0,
                y: 0,
                wheel: 0,
                pan: 0,
            },
            accel: 0,
            movement: DirectionState::default(),
            wheel: DirectionState::default(),
        }
    }

    /// Process a mouse key event (press or release).
    pub fn process(&mut self, key: HidKeyCode, pressed: bool, config: &MouseKeyConfig) -> MouseAction {
        if let Ok(category) = MouseCategory::try_from(key) {
            match category {
                MouseCategory::Movement(direction) => {
                    // delay is only consumed on new-activation (false→true) inside update()
                    let delay = config.get_movement_delay(self.movement.repeat);
                    self.movement.update(direction, pressed, delay);
                }
                MouseCategory::Wheel(direction) => {
                    // delay is only consumed on new-activation (false→true) inside update()
                    let delay = config.get_wheel_delay(self.wheel.repeat);
                    self.wheel.update(direction, pressed, delay);
                }
                MouseCategory::Accel(bit) => {
                    if pressed {
                        self.accel |= bit;
                    } else {
                        self.accel &= !bit;
                    }
                    return MouseAction::None;
                }
                MouseCategory::Button(index) => {
                    if pressed {
                        self.report.buttons |= 1 << index;
                    } else {
                        self.report.buttons &= !(1 << index);
                    }
                    return MouseAction::SendReport;
                }
            }

            let old_report = self.report;
            self.recalculate_report(config);

            if old_report != self.report {
                MouseAction::SendReport
            } else {
                MouseAction::None
            }
        } else {
            MouseAction::None
        }
    }

    /// Recompute report axes from direction state + acceleration + accel multiplier.
    /// Buttons are NOT touched (managed directly by press/release).
    pub fn recalculate_report(&mut self, config: &MouseKeyConfig) {
        if self.movement.has_active_direction() {
            let unit = self.calculate_move_unit(config);
            let (x, y) = self.movement.axis_values(unit);
            self.report.x = x;
            self.report.y = y;
        } else {
            self.report.x = 0;
            self.report.y = 0;
        }

        if self.wheel.has_active_direction() {
            let unit = self.calculate_wheel_unit(config);
            let (pan, wheel) = self.wheel.axis_values(unit);
            self.report.wheel = wheel;
            self.report.pan = pan;
        } else {
            self.report.wheel = 0;
            self.report.pan = 0;
        }
    }

    /// Check which categories have expired deadlines and fire them.
    /// Only fires categories whose deadline has actually passed, so movement
    /// and wheel repeat at their own independent intervals.
    /// Returns a masked `MouseReport` containing only the axes whose repeat
    /// actually fired, or `None` if nothing fired.
    pub fn fire_repeats(&mut self, config: &MouseKeyConfig) -> Option<MouseReport> {
        let now = Instant::now();
        let fire_movement = self.movement.deadline.is_some_and(|d| now >= d) && self.movement.has_active_direction();
        let fire_wheel = self.wheel.deadline.is_some_and(|d| now >= d) && self.wheel.has_active_direction();

        if fire_movement {
            let delay = config.get_movement_delay(self.movement.repeat);
            self.movement.on_repeat_tick(config.ticks_to_max, delay);
        }
        if fire_wheel {
            let delay = config.get_wheel_delay(self.wheel.repeat);
            self.wheel.on_repeat_tick(config.wheel_ticks_to_max, delay);
        }

        if !fire_movement && !fire_wheel {
            return None;
        }

        self.recalculate_report(config);

        let mut report = self.get_report();
        if !fire_movement {
            report.x = 0;
            report.y = 0;
        }
        if !fire_wheel {
            report.wheel = 0;
            report.pan = 0;
        }
        Some(report)
    }

    /// Returns the earliest pending repeat deadline, if any.
    pub fn next_deadline(&self) -> Option<Instant> {
        match (self.movement.deadline, self.wheel.deadline) {
            (Some(m), Some(w)) => Some(if m <= w { m } else { w }),
            (Some(m), None) => Some(m),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        }
    }

    /// Return a copy of the current report with diagonal compensation applied.
    /// The internal `self.report` retains raw (uncompensated) axis values so that
    /// repeated recalculations do not shrink the non-repeated axis over time.
    pub fn get_report(&self) -> MouseReport {
        let mut r = self.report;
        if r.x != 0 && r.y != 0 {
            let (x, y) = Self::apply_diagonal_compensation(r.x, r.y);
            r.x = x;
            r.y = y;
        }
        if r.wheel != 0 && r.pan != 0 {
            let (w, p) = Self::apply_diagonal_compensation(r.wheel, r.pan);
            r.wheel = w;
            r.pan = p;
        }
        r
    }

    /// Two-step speed calculation:
    /// Step 1: acceleration curve based on repeat count
    /// Step 2: accel multiplier (Accel0=0.25x, Accel1=0.5x, Accel2=2.0x, highest wins)
    fn calculate_unit(accel: u8, repeat: u8, delta: u8, max_speed: u8, ticks_to_max: u8, max: u8) -> i16 {
        // Step 1: Base value from acceleration curve
        let max_unit = (delta as u16).saturating_mul(max_speed as u16);
        let base: u16 = if repeat == 0 {
            delta as u16
        } else if repeat >= ticks_to_max {
            max_unit
        } else {
            let repeat_count = repeat as u32;
            let ttm = ticks_to_max as u32;
            let min_unit = delta as u32;
            let unit_range = (max_unit as u32).saturating_sub(min_unit);
            let linear_term = 2 * repeat_count * ttm;
            let quadratic_term = repeat_count * repeat_count;
            let progress_num = linear_term.saturating_sub(quadratic_term);
            let progress_den = ttm * ttm;
            (min_unit + (unit_range * progress_num / progress_den.max(1))) as u16
        };

        // Step 2: Apply accel multiplier (highest active wins)
        let multiplied: u16 = if accel & (1 << 2) != 0 {
            // Accel2: 2.0x
            base.saturating_mul(2)
        } else if accel & (1 << 1) != 0 {
            // Accel1: 0.5x (round up to avoid zero)
            base.div_ceil(2)
        } else if accel & (1 << 0) != 0 {
            // Accel0: 0.25x (round up to avoid zero)
            base.div_ceil(4)
        } else {
            base
        };

        // Step 3: Clamp to [1, max]
        let clamped = if max == 0 {
            1u16
        } else if multiplied > max as u16 {
            max as u16
        } else if multiplied == 0 {
            1
        } else {
            multiplied
        };
        clamped as i16
    }

    /// Calculate mouse movement distance based on current repeat count and acceleration settings
    fn calculate_move_unit(&self, config: &MouseKeyConfig) -> i16 {
        Self::calculate_unit(
            self.accel,
            self.movement.repeat,
            config.move_delta,
            config.max_speed,
            config.ticks_to_max,
            config.move_max,
        )
    }

    /// Calculate mouse wheel movement distance based on current repeat count and acceleration settings
    fn calculate_wheel_unit(&self, config: &MouseKeyConfig) -> i16 {
        Self::calculate_unit(
            self.accel,
            self.wheel.repeat,
            config.wheel_delta,
            config.wheel_max_speed,
            config.wheel_ticks_to_max,
            config.wheel_max,
        )
    }

    /// Apply diagonal movement compensation (approximation of 1/sqrt(2))
    fn apply_diagonal_compensation(mut x: i16, mut y: i16) -> (i16, i16) {
        if x != 0 && y != 0 {
            // Widen so the 181/256 multiply can't overflow.
            let x32 = x as i32;
            let y32 = y as i32;
            let x_bias: i32 = if x32 >= 0 { 128 } else { -128 };
            let y_bias: i32 = if y32 >= 0 { 128 } else { -128 };
            let x_compensated = (x32 * 181 + x_bias) / 256;
            let y_compensated = (y32 * 181 + y_bias) / 256;
            x = if x_compensated == 0 {
                if x > 0 { 1 } else { -1 }
            } else {
                x_compensated as i16
            };
            y = if y_compensated == 0 {
                if y > 0 { 1 } else { -1 }
            } else {
                y_compensated as i16
            };
        }
        (x, y)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::MouseKeyConfig;

    fn default_config() -> MouseKeyConfig {
        MouseKeyConfig::default()
    }

    // -- A. Basic press/release -----------------------------------------------

    #[test]
    fn press_right_sets_positive_x() {
        let mut state = MouseState::new();
        let config = default_config();
        let action = state.process(HidKeyCode::MouseRight, true, &config);
        assert!(state.report.x > 0);
        assert_eq!(state.report.y, 0);
        assert_eq!(action, MouseAction::SendReport);
    }

    #[test]
    fn press_up_sets_negative_y() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseUp, true, &config);
        assert!(state.report.y < 0);
        assert_eq!(state.report.x, 0);
    }

    #[test]
    fn release_clears_axis() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        let action = state.process(HidKeyCode::MouseRight, false, &config);
        assert_eq!(state.report.x, 0);
        assert_eq!(action, MouseAction::SendReport);
    }

    #[test]
    fn button_press_and_release() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseBtn1, true, &config);
        assert_eq!(state.report.buttons, 1);
        state.process(HidKeyCode::MouseBtn1, false, &config);
        assert_eq!(state.report.buttons, 0);
    }

    // -- B. Opposite direction cancellation (req 4.5) -------------------------

    #[test]
    fn opposite_x_cancels() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        assert!(state.report.x > 0);
        state.process(HidKeyCode::MouseLeft, true, &config);
        assert_eq!(state.report.x, 0, "Left+Right should cancel to 0");
    }

    #[test]
    fn opposite_y_cancels() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseDown, true, &config);
        state.process(HidKeyCode::MouseUp, true, &config);
        assert_eq!(state.report.y, 0, "Up+Down should cancel to 0");
    }

    #[test]
    fn opposite_release_restores() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseLeft, true, &config);
        assert_eq!(state.report.x, 0);
        state.process(HidKeyCode::MouseLeft, false, &config);
        assert!(state.report.x > 0, "Releasing Left should restore Right");
    }

    #[test]
    fn opposite_wheel_cancels() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseWheelUp, true, &config);
        state.process(HidKeyCode::MouseWheelDown, true, &config);
        assert_eq!(state.report.wheel, 0);
    }

    // -- C. Acceleration continuity (req 4.1) ---------------------------------

    #[test]
    fn new_direction_preserves_repeat() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.movement.repeat = 10;
        state.process(HidKeyCode::MouseDown, true, &config);
        assert_eq!(
            state.movement.repeat, 10,
            "Adding a new direction should not reset repeat"
        );
    }

    #[test]
    fn direction_change_resets_repeat_on_cancel() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseDown, true, &config);
        state.movement.repeat = 10;
        state.process(HidKeyCode::MouseUp, true, &config);
        // Opposite directions cancel → inactive → repeat resets
        assert_eq!(state.movement.repeat, 0, "Opposite cancel should reset repeat");
    }

    #[test]
    fn repeat_resets_only_when_all_released() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseDown, true, &config);
        state.movement.repeat = 10;
        state.process(HidKeyCode::MouseDown, false, &config);
        assert_eq!(state.movement.repeat, 10, "Repeat should stay while Right is held");
        state.process(HidKeyCode::MouseRight, false, &config);
        assert_eq!(state.movement.repeat, 0, "Repeat should reset when all released");
    }

    #[test]
    fn wheel_repeat_independent() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseWheelUp, true, &config);
        state.movement.repeat = 10;
        state.wheel.repeat = 5;
        state.process(HidKeyCode::MouseRight, false, &config);
        assert_eq!(state.movement.repeat, 0, "Movement repeat should reset");
        assert_eq!(state.wheel.repeat, 5, "Wheel repeat should be independent");
    }

    // -- D. Duplicate keys (req 4.4) ------------------------------------------

    #[test]
    fn duplicate_key_single_effect() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        let x_single = state.report.x;
        let action = state.process(HidKeyCode::MouseRight, true, &config);
        assert_eq!(state.report.x, x_single, "Duplicate should not change magnitude");
        assert_eq!(
            action,
            MouseAction::None,
            "Duplicate same-direction press should not send report"
        );
    }

    #[test]
    fn duplicate_release_one_keeps_axis() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseRight, false, &config);
        assert!(state.report.x > 0, "x should stay positive with one still held");
    }

    #[test]
    fn duplicate_release_both_clears() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseRight, true, &config);
        state.movement.repeat = 5;
        state.process(HidKeyCode::MouseRight, false, &config);
        state.process(HidKeyCode::MouseRight, false, &config);
        assert_eq!(state.report.x, 0);
        assert_eq!(state.movement.repeat, 0, "Repeat should reset when all released");
    }

    // -- E. Diagonal (req 4.6) ------------------------------------------------

    #[test]
    fn diagonal_both_axes_set() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseDown, true, &config);
        assert!(state.report.x > 0);
        assert!(state.report.y > 0);
    }

    #[test]
    fn diagonal_compensation_reduces() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseDown, true, &config);
        let comp = state.get_report();
        assert!(comp.x < state.report.x, "Compensation should reduce diagonal x");
        assert!(comp.y < state.report.y, "Compensation should reduce diagonal y");
    }

    #[test]
    fn diagonal_repeat_both_axes_accelerate() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseDown, true, &config);
        let initial_x = state.report.x;
        let initial_y = state.report.y;

        for _ in 0..10 {
            state
                .movement
                .on_repeat_tick(config.ticks_to_max, config.get_movement_delay(state.movement.repeat));
            state.recalculate_report(&config);
        }

        assert!(state.report.x > initial_x, "x should accelerate");
        assert!(state.report.y > initial_y, "y should accelerate");
        assert_eq!(state.report.x, state.report.y);
    }

    #[test]
    fn single_axis_no_compensation() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        let comp = state.get_report();
        assert_eq!(comp.x, state.report.x);
        assert_eq!(comp.y, 0);
    }

    // -- F. Accel multiplier (req 3) ------------------------------------------

    #[test]
    fn accel0_reduces_speed() {
        let result = MouseState::calculate_unit(1, 0, 6, 3, 50, 20);
        assert_eq!(result, 2); // (6+3)/4 = 2
    }

    #[test]
    fn accel1_halves_speed() {
        let result = MouseState::calculate_unit(2, 0, 6, 3, 50, 20);
        assert_eq!(result, 3); // (6+1)/2 = 3
    }

    #[test]
    fn accel2_doubles_speed() {
        let result = MouseState::calculate_unit(4, 0, 6, 3, 50, 20);
        assert_eq!(result, 12); // 6*2 = 12
    }

    #[test]
    fn accel_multiplies_accelerated_value() {
        // At repeat=25 (mid-curve), accel2 should roughly double the base
        // Use a high max to avoid clamping
        let base = MouseState::calculate_unit(0, 25, 6, 3, 50, 127);
        let with_accel2 = MouseState::calculate_unit(4, 25, 6, 3, 50, 127);
        assert!(with_accel2 > base);
        // Should be approximately 2x (integer rounding may cause ±1)
        assert_eq!(with_accel2, base * 2);
    }

    #[test]
    fn accel_respects_max_clamp() {
        let result = MouseState::calculate_unit(4, 50, 6, 3, 50, 20);
        assert_eq!(result, 20); // 18*2=36, clamped to 20
    }

    #[test]
    fn accel_highest_wins() {
        let accel2_only = MouseState::calculate_unit(4, 0, 6, 3, 50, 20);
        let accel0_and_2 = MouseState::calculate_unit(5, 0, 6, 3, 50, 20);
        assert_eq!(accel0_and_2, accel2_only);
    }

    #[test]
    fn accel_press_only_modifies_state() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        let x_before = state.report.x;
        let action = state.process(HidKeyCode::MouseAccel2, true, &config);
        assert_eq!(action, MouseAction::None, "Accel should not send report");
        assert_eq!(
            state.report.x, x_before,
            "Report should not change until next repeat tick"
        );
        assert_eq!(state.accel, 1 << 2, "Accel state should be updated");
    }

    #[test]
    fn accel_without_direction_is_noop() {
        let mut state = MouseState::new();
        let config = default_config();
        let action = state.process(HidKeyCode::MouseAccel0, true, &config);
        assert_eq!(action, MouseAction::None);
        assert_eq!(state.report.x, 0);
    }

    #[test]
    fn accel_release_only_modifies_state() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseAccel2, true, &config);
        let x_before = state.report.x;
        let action = state.process(HidKeyCode::MouseAccel2, false, &config);
        assert_eq!(action, MouseAction::None, "Accel release should not send report");
        assert_eq!(
            state.report.x, x_before,
            "Report should not change until next repeat tick"
        );
        assert_eq!(state.accel, 0, "Accel state should be cleared");
    }

    // -- G. Edge cases --------------------------------------------------------

    #[test]
    fn no_directions_report_zero() {
        let state = MouseState::new();
        assert_eq!(state.report.x, 0);
        assert_eq!(state.report.y, 0);
        assert_eq!(state.report.wheel, 0);
        assert_eq!(state.report.pan, 0);
    }

    #[test]
    fn all_four_directions_cancel() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseUp, true, &config);
        state.process(HidKeyCode::MouseDown, true, &config);
        state.process(HidKeyCode::MouseLeft, true, &config);
        state.process(HidKeyCode::MouseRight, true, &config);
        assert_eq!(state.report.x, 0);
        assert_eq!(state.report.y, 0);
    }

    #[test]
    fn three_directions_one_cancels() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseUp, true, &config);
        state.process(HidKeyCode::MouseDown, true, &config);
        state.process(HidKeyCode::MouseRight, true, &config);
        assert_eq!(state.report.y, 0, "Up+Down should cancel");
        assert!(state.report.x > 0, "Right should still be active");
    }

    #[test]
    fn calculate_unit_never_zero() {
        let result = MouseState::calculate_unit(0, 0, 0, 1, 50, 20);
        assert_eq!(result, 1);
    }

    #[test]
    fn calculate_unit_clamps_to_max() {
        // `max` is the only cap; the report axes are 16-bit, so a `move_max`
        // above 127 survives instead of being truncated.
        let result = MouseState::calculate_unit(4, 50, 100, 10, 50, 255);
        assert_eq!(result, 255);
    }

    // -- H. Return value semantics --------------------------------------------

    #[test]
    fn first_direction_schedules_repeat() {
        let mut state = MouseState::new();
        let config = default_config();
        let action = state.process(HidKeyCode::MouseRight, true, &config);
        assert_eq!(action, MouseAction::SendReport);
        assert!(state.movement.deadline.is_some());
    }

    #[test]
    fn second_direction_does_not_reschedule() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        let deadline_after_first = state.movement.deadline;
        let action = state.process(HidKeyCode::MouseDown, true, &config);
        assert_eq!(action, MouseAction::SendReport);
        // Deadline unchanged — repeat was already running
        assert_eq!(state.movement.deadline, deadline_after_first);
    }

    #[test]
    fn wheel_first_schedules_repeat() {
        let mut state = MouseState::new();
        let config = default_config();
        let action = state.process(HidKeyCode::MouseWheelUp, true, &config);
        assert_eq!(action, MouseAction::SendReport);
        assert!(state.wheel.deadline.is_some());
    }

    #[test]
    fn button_returns_send_report() {
        let mut state = MouseState::new();
        let config = default_config();
        let action = state.process(HidKeyCode::MouseBtn1, true, &config);
        assert_eq!(action, MouseAction::SendReport);
    }

    #[test]
    fn release_accel_without_active_returns_none() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseAccel0, true, &config);
        let action = state.process(HidKeyCode::MouseAccel0, false, &config);
        assert_eq!(action, MouseAction::None);
    }

    // -- I. on_repeat_tick ----------------------------------------------------

    #[test]
    fn on_repeat_tick_increments_and_recalculates() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        assert_eq!(state.movement.repeat, 1);
        let x_initial = state.report.x;

        state
            .movement
            .on_repeat_tick(config.ticks_to_max, config.get_movement_delay(state.movement.repeat));
        state.recalculate_report(&config);
        assert_eq!(state.movement.repeat, 2);
        assert!(state.report.x >= x_initial);
    }

    #[test]
    fn on_repeat_tick_schedules_next_deadline() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseRight, true, &config);
        // Clear the deadline set by process so we can verify on_repeat_tick sets it
        state.movement.deadline = None;
        state
            .movement
            .on_repeat_tick(config.ticks_to_max, config.get_movement_delay(state.movement.repeat));
        assert!(state.movement.deadline.is_some());
    }

    // -- J. Diagonal compensation static tests --------------------------------

    #[test]
    fn diagonal_compensation_reduces_magnitude() {
        let (x, y) = MouseState::apply_diagonal_compensation(10, 10);
        assert!(x < 10 && x > 0);
        assert!(y < 10 && y > 0);
        assert_eq!(x, y);
    }

    #[test]
    fn diagonal_compensation_preserves_sign() {
        let (x, y) = MouseState::apply_diagonal_compensation(-10, 10);
        assert!(x < 0);
        assert!(y > 0);
    }

    #[test]
    fn diagonal_compensation_small_values_never_zero() {
        let (x, y) = MouseState::apply_diagonal_compensation(1, 1);
        assert_eq!(x, 1);
        assert_eq!(y, 1);
    }

    // -- K. calculate_unit curve tests ----------------------------------------

    #[test]
    fn calculate_unit_initial_returns_delta() {
        let result = MouseState::calculate_unit(0, 0, 6, 3, 50, 20);
        assert_eq!(result, 6);
    }

    #[test]
    fn calculate_unit_at_max_speed() {
        let result = MouseState::calculate_unit(0, 50, 6, 3, 50, 20);
        assert_eq!(result, 18);
    }

    #[test]
    fn calculate_unit_clamped_to_max() {
        let result = MouseState::calculate_unit(0, 50, 6, 3, 50, 10);
        assert_eq!(result, 10);
    }

    // -- L. Repeat scheduling / deadline behavior -----------------------------

    #[test]
    fn fire_repeats_only_due_category_fires() {
        let mut state = MouseState::new();
        let config = default_config();

        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseWheelUp, true, &config);

        // Only movement is due.
        state.movement.deadline = Some(Instant::MIN);
        state.wheel.deadline = Some(Instant::now() + Duration::from_secs(60));

        let report = state.fire_repeats(&config);

        let report = report.expect("Movement was due, should return Some");
        assert!(report.x != 0, "Movement axis should be non-zero");
        assert_eq!(report.wheel, 0, "Wheel should be masked to 0");
        assert_eq!(state.movement.repeat, 2, "Movement repeat should tick");
        assert_eq!(state.wheel.repeat, 1, "Wheel repeat should not tick");
    }

    #[test]
    fn fire_repeats_none_due_no_fire() {
        let mut state = MouseState::new();
        let config = default_config();

        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseWheelUp, true, &config);

        state.movement.deadline = Some(Instant::now() + Duration::from_secs(60));
        state.wheel.deadline = Some(Instant::now() + Duration::from_secs(60));

        let report = state.fire_repeats(&config);

        assert!(report.is_none(), "Nothing due, should return None");
        assert_eq!(state.movement.repeat, 1);
        assert_eq!(state.wheel.repeat, 1);
    }

    #[test]
    fn fire_repeats_both_due_fire_both() {
        let mut state = MouseState::new();
        let config = default_config();

        state.process(HidKeyCode::MouseRight, true, &config);
        state.process(HidKeyCode::MouseWheelUp, true, &config);

        state.movement.deadline = Some(Instant::MIN);
        state.wheel.deadline = Some(Instant::MIN);

        let report = state.fire_repeats(&config);

        let report = report.expect("Both due, should return Some");
        assert!(report.x != 0, "Movement axis should be non-zero");
        assert!(report.wheel != 0, "Wheel axis should be non-zero");
        assert_eq!(state.movement.repeat, 2);
        assert_eq!(state.wheel.repeat, 2);
    }

    #[test]
    fn next_deadline_selects_earliest_and_handles_none() {
        let mut state = MouseState::new();
        let now = Instant::now();

        assert_eq!(state.next_deadline(), None);

        let movement_deadline = now + Duration::from_millis(10);
        state.movement.deadline = Some(movement_deadline);
        assert_eq!(state.next_deadline(), Some(movement_deadline));

        let wheel_deadline = now + Duration::from_millis(20);
        state.wheel.deadline = Some(wheel_deadline);
        assert_eq!(state.next_deadline(), Some(movement_deadline));

        state.movement.deadline = None;
        assert_eq!(state.next_deadline(), Some(wheel_deadline));
    }

    // -- M. Additional behavior guards ---------------------------------------

    #[test]
    fn non_mouse_key_is_noop() {
        let mut state = MouseState::new();
        let config = default_config();
        let before = state.report;

        let action = state.process(HidKeyCode::A, true, &config);

        assert_eq!(action, MouseAction::None);
        assert_eq!(state.report, before);
        assert_eq!(state.next_deadline(), None);
    }

    #[test]
    fn wheel_diagonal_compensation_reduces_magnitude() {
        let mut state = MouseState::new();
        let config = default_config();

        state.process(HidKeyCode::MouseWheelUp, true, &config);
        state.process(HidKeyCode::MouseWheelRight, true, &config);

        // Ensure wheel/pan magnitude is high enough so compensation is visible.
        state.accel = 1 << 2;
        state.wheel.repeat = config.wheel_ticks_to_max;
        state.recalculate_report(&config);

        let raw = state.report;
        let compensated = state.get_report();

        assert!(raw.wheel != 0 && raw.pan != 0);
        assert!(compensated.wheel.abs() < raw.wheel.abs());
        assert!(compensated.pan.abs() < raw.pan.abs());
    }

    // -- N. Wheel sign convention tests ----------------------------------------

    #[test]
    fn wheel_up_sets_negative_wheel() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseWheelUp, true, &config);
        assert!(state.report.wheel < 0, "WheelUp should produce negative wheel");
        assert_eq!(state.report.pan, 0);
    }

    #[test]
    fn wheel_right_sets_positive_pan() {
        let mut state = MouseState::new();
        let config = default_config();
        state.process(HidKeyCode::MouseWheelRight, true, &config);
        assert!(state.report.pan > 0, "WheelRight should produce positive pan");
        assert_eq!(state.report.wheel, 0);
    }
}
