//! Pure joystick timing, axis math, and idle report policy.

/// Suppress repeated stationary reports, never repeated relative movement.
#[derive(Default)]
pub struct IdleReportFilter {
    last_rest: Option<(u32, u8)>,
}

impl IdleReportFilter {
    pub fn should_send(&self, connection_epoch: u32, buttons: u8, moving: bool) -> bool {
        moving || self.last_rest != Some((connection_epoch, buttons))
    }

    /// Record only after enqueueing, with no intervening connection transition.
    pub fn record_queued(&mut self, connection_epoch: u32, buttons: u8, moving: bool) {
        self.last_rest = if moving {
            None
        } else {
            Some((connection_epoch, buttons))
        };
    }
}

#[derive(Clone, Copy, Debug)]
pub struct JoystickPowerConfig {
    pub polling_rate_hz: u16,
    pub power_on_duty: u16,
    pub idle_polling_rate_hz: u16,
    pub idle_power_on_duty: u16,
    pub sample_settle_us: u32,
    pub boot_settle_ms: u32,
}

impl JoystickPowerConfig {
    pub fn period_us(&self, idle: bool) -> u64 {
        let hz = if idle {
            self.idle_polling_rate_hz
        } else {
            self.polling_rate_hz
        };
        1_000_000 / u64::from(hz.max(1))
    }

    pub fn duty(&self, idle: bool) -> u16 {
        if idle {
            self.idle_power_on_duty
        } else {
            self.power_on_duty
        }
    }

    pub fn on_us(&self, idle: bool) -> u64 {
        self.period_us(idle) * u64::from(self.duty(idle)) / 1000
    }

    pub fn can_sample(&self, idle: bool, switched: bool) -> bool {
        let budget = if switched {
            self.on_us(idle)
        } else {
            self.period_us(idle)
        };
        u64::from(self.sample_settle_us) + 20 <= budget
    }
}

#[derive(Default)]
pub struct IdleTracker {
    centered_since_us: Option<u64>,
    pub idle: bool,
}

impl IdleTracker {
    pub fn observe(&mut self, now_us: u64, centered: bool) {
        if !centered {
            self.centered_since_us = None;
            self.idle = false;
        } else {
            let since = *self.centered_since_us.get_or_insert(now_us);
            self.idle = now_us.saturating_sub(since) >= 1_200_000;
        }
    }
}

/// Preserve the existing ADC-to-axis mapping, without overflowing on negative ADC noise.
pub fn adc_axis(raw: i16) -> i16 {
    ((i32::from(raw) - 16384) * 2).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

/// Square deadzone: each axis is independently suppressed and remapped continuously.
pub fn apply_deadzone(value: i16, deadzone: u16) -> i16 {
    let value = i32::from(value);
    let magnitude = (value.abs() - i32::from(deadzone)).max(0);
    (value.signum() * magnitude) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_reports_send_once_then_stay_silent() {
        let mut filter = IdleReportFilter::default();
        assert!(filter.should_send(0, 0, false));
        filter.record_queued(0, 0, false);
        for _ in 0..100 {
            assert!(!filter.should_send(0, 0, false));
        }
    }

    #[test]
    fn repeated_relative_motion_is_never_deduplicated() {
        let mut filter = IdleReportFilter::default();
        for _ in 0..100 {
            assert!(filter.should_send(0, 0, true));
            filter.record_queued(0, 0, true);
        }
        // Send one stationary report after movement, then stop repeating it.
        assert!(filter.should_send(0, 0, false));
        filter.record_queued(0, 0, false);
        assert!(!filter.should_send(0, 0, false));
    }

    #[test]
    fn every_button_change_including_release_is_sent() {
        let mut filter = IdleReportFilter::default();
        for buttons in [0, 1, 3, 2, 0] {
            assert!(filter.should_send(0, buttons, false));
            filter.record_queued(0, buttons, false);
            assert!(!filter.should_send(0, buttons, false));
        }
    }

    #[test]
    fn wake_motion_and_drag_are_not_delayed_by_idle_filter() {
        let mut filter = IdleReportFilter::default();
        filter.record_queued(0, 1, false);
        assert!(filter.should_send(0, 1, true));
        filter.record_queued(0, 1, true);
        assert!(filter.should_send(0, 1, true));
        assert!(filter.should_send(0, 0, false));
    }

    #[test]
    fn a_new_connection_resynchronizes_even_with_identical_buttons() {
        let mut filter = IdleReportFilter::default();
        filter.record_queued(10, 0, false);
        assert!(!filter.should_send(10, 0, false));
        // Two transitions can occur between polls, ending on the same transport.
        assert!(filter.should_send(12, 0, false));
        filter.record_queued(12, 0, false);
        assert!(!filter.should_send(12, 0, false));
        assert!(filter.should_send(13, 0, false));
    }

    #[test]
    fn an_unqueued_or_cancelled_report_does_not_suppress_the_next_attempt() {
        let mut filter = IdleReportFilter::default();
        assert!(filter.should_send(1, 0, false));
        assert!(filter.should_send(1, 0, false));
        filter.record_queued(1, 1, false);
        assert!(filter.should_send(1, 0, false));
        assert!(filter.should_send(2, 1, false));
    }

    #[test]
    fn filter_state_is_independent_for_each_joystick() {
        let mut first = IdleReportFilter::default();
        let second = IdleReportFilter::default();
        first.record_queued(0, 0, false);
        assert!(!first.should_send(0, 0, false));
        assert!(second.should_send(0, 0, false));
    }

    fn config() -> JoystickPowerConfig {
        JoystickPowerConfig {
            polling_rate_hz: 50,
            power_on_duty: 100,
            idle_polling_rate_hz: 10,
            idle_power_on_duty: 20,
            sample_settle_us: 20,
            boot_settle_ms: 2,
        }
    }

    #[test]
    fn high_and_low_duty_are_permille() {
        let mut c = config();
        assert_eq!(c.period_us(false), 20_000);
        assert_eq!(c.period_us(true), 100_000);
        assert_eq!(c.on_us(false), 2_000);
        assert_eq!(c.on_us(true), 2_000);
        c.power_on_duty = 1;
        assert_eq!(c.on_us(false), 20);
        assert!(!c.can_sample(false, true));
        c.power_on_duty = 10;
        assert_eq!(c.on_us(false), 200);
        assert!(c.can_sample(false, true));
        c.power_on_duty = 1000;
        assert_eq!(c.on_us(false), c.period_us(false));
    }

    #[test]
    fn waits_must_fit_both_modes() {
        let mut c = config();
        c.sample_settle_us = 1980;
        assert!(c.can_sample(false, true));
        c.sample_settle_us += 1;
        assert!(!c.can_sample(false, true));
        assert!(!c.can_sample(true, true));
        assert!(c.can_sample(false, false));
    }

    #[test]
    fn deadzone_is_symmetric_continuous_and_handles_minimum() {
        for v in -100..=100 {
            assert_eq!(apply_deadzone(v, 100), 0);
        }
        assert_eq!(apply_deadzone(101, 100), 1);
        assert_eq!(apply_deadzone(-101, 100), -1);
        assert_eq!(apply_deadzone(i16::MIN, 0), i16::MIN);
        assert_eq!(apply_deadzone(i16::MIN, 100), -32668);
    }

    #[test]
    fn inactivity_requires_continuous_centering_and_wakes_immediately() {
        let mut state = IdleTracker::default();
        state.observe(10_000, true);
        state.observe(1_209_999, true);
        assert!(!state.idle);
        state.observe(1_210_000, true);
        assert!(state.idle);
        state.observe(1_210_001, false);
        assert!(!state.idle);
        state.observe(2_000_000, true);
        state.observe(3_000_000, false);
        state.observe(4_000_000, true);
        assert!(!state.idle);
        state.observe(5_200_000, true);
        assert!(state.idle);
    }

    #[test]
    fn a_held_off_center_axis_never_goes_idle() {
        let mut state = IdleTracker::default();
        for now in [0, 1_200_000, 10_000_000] {
            let centered = apply_deadzone(101, 100) == 0 && apply_deadzone(0, 100) == 0;
            state.observe(now, centered);
            assert!(!state.idle);
        }
    }

    #[test]
    fn continuous_supply_follows_the_new_mode_on_both_transitions() {
        for (active_duty, idle_duty) in [(1000, 20), (100, 1000), (1000, 1000), (100, 20)] {
            let mut c = config();
            c.power_on_duty = active_duty;
            c.idle_power_on_duty = idle_duty;
            let mut state = IdleTracker::default();
            state.observe(0, true);
            assert_eq!(c.duty(state.idle) == 1000, active_duty == 1000);
            state.observe(1_200_000, true);
            assert!(state.idle);
            assert_eq!(c.duty(state.idle) == 1000, idle_duty == 1000);
            state.observe(1_200_001, false);
            assert!(!state.idle);
            assert_eq!(c.duty(state.idle) == 1000, active_duty == 1000);
        }
    }

    #[test]
    fn fixed_bias_is_applied_in_the_existing_axis_units() {
        assert_eq!(adc_axis(1819).saturating_add(29130), 0);
        assert_eq!(adc_axis(-1), i16::MIN);
        assert_eq!(adc_axis(i16::MAX), 32766);
        assert_eq!(apply_deadzone(adc_axis(1869).saturating_add(29130), 100), 0);
    }
}
