use rmk_macro::processor;
use usbd_hid::descriptor::MouseReport;

use crate::channel::send_hid_report;
use crate::event::PointingEvent;
use crate::hid::Report;
use crate::input_device::pointing::ALL_POINTING_DEVICES;
use crate::keymap::KeyMap;

/// Suppress repeated stationary reports, never repeated relative movement.
#[derive(Default)]
pub(crate) struct IdleReportFilter {
    last_rest: Option<(u32, u8)>,
}

impl IdleReportFilter {
    fn should_send(&self, connection_epoch: u32, buttons: u8, moving: bool) -> bool {
        moving || self.last_rest != Some((connection_epoch, buttons))
    }

    fn record_queued(&mut self, connection_epoch: u32, buttons: u8, moving: bool) {
        self.last_rest = (!moving).then_some((connection_epoch, buttons));
    }
}

#[derive(Clone, Copy, Debug)]
pub struct JoystickPowerConfig {
    pub polling_rate_hz: u16,
    pub idle_polling_rate_hz: u16,
    pub sample_settle_us: u32,
    pub boot_settle_ms: u32,
}

impl JoystickPowerConfig {
    pub(crate) fn period_us(&self, idle: bool) -> u64 {
        let hz = if idle {
            self.idle_polling_rate_hz
        } else {
            self.polling_rate_hz
        };
        1_000_000 / u64::from(hz.max(1))
    }
}

#[derive(Default)]
pub(crate) struct IdleTracker {
    centered_since_us: Option<u64>,
    pub idle: bool,
}

impl IdleTracker {
    pub(crate) fn observe(&mut self, now_us: u64, centered: bool) {
        if !centered {
            self.centered_since_us = None;
            self.idle = false;
        } else {
            let since = *self.centered_since_us.get_or_insert(now_us);
            self.idle = now_us.saturating_sub(since) >= 1_200_000;
        }
    }
}

pub(crate) fn adc_axis(raw: i16) -> i16 {
    ((i32::from(raw) - 16384) * 2).clamp(i16::MIN as i32, i16::MAX as i32) as i16
}

pub(crate) fn apply_deadzone(value: i16, deadzone: u16) -> i16 {
    let value = i32::from(value);
    let magnitude = (value.abs() - i32::from(deadzone)).max(0);
    (value.signum() * magnitude) as i16
}

#[processor(subscribe = [PointingEvent])]
pub struct JoystickProcessor<'a, const N: usize> {
    /// Only process events from this device id. Use ALL_POINTING_DEVICES (255) to accept all.
    device_id: u8,
    transform: [[i16; N]; N],
    bias: [i16; N],
    keymap: &'a KeyMap<'a>,
    record: [i16; N],
    resolution: u16,
    deadzone: u16,
    idle_report_filter: IdleReportFilter,
}

impl<'a, const N: usize> JoystickProcessor<'a, N> {
    pub fn new(
        device_id: u8,
        transform: [[i16; N]; N],
        bias: [i16; N],
        resolution: u16,
        keymap: &'a KeyMap<'a>,
    ) -> Self {
        Self {
            device_id,
            transform,
            bias,
            resolution,
            keymap,
            record: [0; N],
            deadzone: 0,
            idle_report_filter: IdleReportFilter::default(),
        }
    }

    pub fn with_deadzone(mut self, deadzone: u16) -> Self {
        self.deadzone = deadzone;
        self
    }

    async fn on_pointing_event(&mut self, event: PointingEvent) {
        if self.device_id != ALL_POINTING_DEVICES && event.device_id != self.device_id {
            return;
        }
        for (rec, e) in self.record.iter_mut().zip(event.axes.iter()) {
            *rec = e.value;
        }
        debug!("Joystick info: {:#?}", self.record);
        self.generate_report().await;
    }

    async fn generate_report(&mut self) {
        let mut report = [0i16; N];

        debug!("JoystickProcessor::generate_report: record = {:?}", self.record);
        for (rec, b) in self.record.iter_mut().zip(self.bias.iter()) {
            *rec = apply_deadzone(rec.saturating_add(*b), self.deadzone);
        }

        for (rep, transform) in report.iter_mut().zip(self.transform.iter()) {
            for (w, v) in transform.iter().zip(self.record) {
                if *w == 0 {
                    // ignore zero weight
                    continue;
                }
                *rep = rep.saturating_add(v.saturating_div(*w));
                *rep = *rep - *rep % self.resolution as i16;
            }
        }

        debug!("JoystickProcessor::generate_report: report = {:?}", report);
        // map to mouse
        let buttons = self.keymap.mouse_buttons();
        let mouse_report = MouseReport {
            buttons,
            x: (report[0].clamp(i8::MIN as i16, i8::MAX as i16)) as i8,
            y: (report[1].clamp(i8::MIN as i16, i8::MAX as i16)) as i8,
            wheel: 0,
            pan: 0,
        };

        // Do not remember reports dropped while no host is selected.
        let epoch = crate::state::CONNECTION_EPOCH.lock(|epoch| epoch.get());
        if crate::state::active_transport().is_none() {
            self.idle_report_filter = IdleReportFilter::default();
            return;
        }
        let moving = mouse_report.x != 0 || mouse_report.y != 0 || mouse_report.wheel != 0 || mouse_report.pan != 0;
        if !self.idle_report_filter.should_send(epoch, buttons, moving) {
            return;
        }

        send_hid_report(Report::MouseReport(mouse_report)).await;
        // A transport change can abort queueing while this task is waiting for capacity.
        if crate::state::CONNECTION_EPOCH.lock(|current| current.get()) == epoch {
            self.idle_report_filter.record_queued(epoch, buttons, moving);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_filter_preserves_motion_buttons_and_reconnection() {
        let mut filter = IdleReportFilter::default();
        assert!(filter.should_send(0, 0, false));
        filter.record_queued(0, 0, false);
        assert!(!filter.should_send(0, 0, false));
        assert!(filter.should_send(0, 0, true));
        assert!(filter.should_send(0, 1, false));
        assert!(filter.should_send(1, 0, false));
    }

    #[test]
    fn deadzone_is_symmetric_and_continuous() {
        for value in -100..=100 {
            assert_eq!(apply_deadzone(value, 100), 0);
        }
        assert_eq!(apply_deadzone(101, 100), 1);
        assert_eq!(apply_deadzone(-101, 100), -1);
        assert_eq!(apply_deadzone(i16::MIN, 100), -32668);
    }

    #[test]
    fn idle_requires_continuous_centering_and_wakes_immediately() {
        let mut tracker = IdleTracker::default();
        tracker.observe(0, true);
        tracker.observe(1_199_999, true);
        assert!(!tracker.idle);
        tracker.observe(1_200_000, true);
        assert!(tracker.idle);
        tracker.observe(1_200_001, false);
        assert!(!tracker.idle);
    }

    #[test]
    fn polling_period_follows_idle_state() {
        let config = JoystickPowerConfig {
            polling_rate_hz: 50,
            idle_polling_rate_hz: 5,
            sample_settle_us: 3,
            boot_settle_ms: 2,
        };
        assert_eq!(config.period_us(false), 20_000);
        assert_eq!(config.period_us(true), 200_000);
    }
}
