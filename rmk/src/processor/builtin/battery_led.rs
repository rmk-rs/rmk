//! Battery LED processor for RMK
use embedded_hal::digital::StatefulOutputPin;
use rmk_macro::processor;
use rmk_types::battery::{BatteryStatus, ChargeState};

use crate::driver::gpio::OutputController;
use crate::event::BatteryStatusEvent;

#[processor(subscribe = [BatteryStatusEvent], poll_interval = 1000)]
pub struct BatteryLedProcessor<P: StatefulOutputPin> {
    pin: OutputController<P>,
    state: BatteryStatus,
}

impl<P: StatefulOutputPin> BatteryLedProcessor<P> {
    pub fn new(pin: P, low_active: bool) -> Self {
        Self {
            pin: OutputController::new(pin, low_active),
            state: BatteryStatus::Unavailable,
        }
    }

    async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
        self.state = event.into();
    }

    async fn poll(&mut self) {
        match self.state {
            BatteryStatus::Available {
                charge_state: ChargeState::Charging,
                ..
            } => self.pin.activate(),
            BatteryStatus::Available { level: Some(level), .. } if level < 10 => {
                // Battery low, blinking the LED
                self.pin.toggle();
            }
            _ => self.pin.deactivate(),
        }
    }
}
