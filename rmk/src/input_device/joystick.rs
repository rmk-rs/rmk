use rmk_macro::processor;

use crate::channel::send_hid_report;
use crate::event::PointingEvent;
use crate::hid::{MouseReport, Report};
use crate::input_device::pointing::ALL_POINTING_DEVICES;
use crate::keymap::KeyMap;

#[processor(subscribe = [PointingEvent])]
pub struct JoystickProcessor<'a, const N: usize> {
    /// Only process events from this device id. Use ALL_POINTING_DEVICES (255) to accept all.
    device_id: u8,
    transform: [[i16; N]; N],
    bias: [i16; N],
    keymap: &'a KeyMap<'a>,
    record: [i16; N],
    resolution: u16,
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
        }
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
            *rec = rec.saturating_add(*b);
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
            x: report[0],
            y: report[1],
            wheel: 0,
            pan: 0,
        };

        // Send mouse report directly
        send_hid_report(Report::MouseReport(mouse_report)).await;
    }
}
