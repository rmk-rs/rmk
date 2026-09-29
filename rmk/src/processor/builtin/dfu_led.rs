//! DFU LED processor for RMK
use embedded_hal::digital::StatefulOutputPin;
use rmk_macro::processor;
use rmk_types::dfu::DfuStatus;

use crate::driver::gpio::OutputController;
use crate::event::DfuStatusEvent;

/// Number of poll cycles (at 200 ms each) the error LED stays active
/// before automatically turning off — 5 seconds total.
const ERROR_BLINK_CYCLES: u8 = 25;

#[processor(subscribe = [DfuStatusEvent], poll_interval = 200)]
pub struct DfuLedProcessor<P: StatefulOutputPin> {
    pin: OutputController<P>,
    blink: bool,
    auto_off_polls: u8,
}

impl<P: StatefulOutputPin> DfuLedProcessor<P> {
    pub fn new(pin: P, low_active: bool) -> Self {
        Self {
            pin: OutputController::new(pin, low_active),
            blink: false,
            auto_off_polls: 0,
        }
    }

    async fn on_dfu_status_event(&mut self, event: DfuStatusEvent) {
        match *event {
            DfuStatus::Idle | DfuStatus::Finished => {
                self.blink = false;
                self.auto_off_polls = 0;
                self.pin.deactivate();
            }
            DfuStatus::Started => {
                self.blink = false;
                self.auto_off_polls = 0;
                self.pin.activate();
            }
            DfuStatus::Downloading => self.pin.toggle(),
            DfuStatus::Error => {
                self.blink = true;
                self.auto_off_polls = ERROR_BLINK_CYCLES;
            }
            DfuStatus::LockWaiting => {
                self.blink = false;
                self.auto_off_polls = 0;
                self.pin.activate();
            }
            DfuStatus::LockUnlocked => {
                self.blink = true;
                self.auto_off_polls = 0;
            }
        }
    }

    async fn poll(&mut self) {
        if self.blink {
            self.pin.toggle();
        }
        if self.auto_off_polls > 0 {
            self.auto_off_polls -= 1;
            if self.auto_off_polls == 0 {
                self.blink = false;
                self.pin.deactivate();
            }
        }
    }
}
