use embassy_rp::adc::{Adc, Async, Channel};
use embassy_time::{Duration, Timer};
use rmk_macro::input_device;

use crate::event::BatteryAdcEvent;

/// Samples a battery ADC channel using a 3.3 V reference.
///
/// Publishes ADC input millivolts; the battery processor applies the divider ratio.
#[input_device(publish = BatteryAdcEvent)]
pub struct Rp2040BatteryAdc<'d> {
    adc: Adc<'d, Async>,
    channel: Channel<'d>,
    interval: Duration,
    first_read: bool,
}

impl<'d> Rp2040BatteryAdc<'d> {
    /// Creates an input device that samples immediately, then waits `interval` between attempts.
    ///
    /// # Panics
    ///
    /// Panics if `interval` is zero.
    pub fn new(adc: Adc<'d, Async>, channel: Channel<'d>, interval: Duration) -> Self {
        assert!(interval.as_ticks() > 0, "battery ADC interval must be nonzero");
        Self {
            adc,
            channel,
            interval,
            first_read: true,
        }
    }

    async fn read_battery_adc_event(&mut self) -> BatteryAdcEvent {
        loop {
            if self.first_read {
                self.first_read = false;
            } else {
                Timer::after(self.interval).await;
            }
            match self.adc.read(&mut self.channel).await {
                Ok(raw) => {
                    let input_mv = (u32::from(raw) * 3300 / 4096) as u16;
                    return BatteryAdcEvent(input_mv);
                }
                Err(_) => warn!("Battery ADC read failed"),
            }
        }
    }
}
