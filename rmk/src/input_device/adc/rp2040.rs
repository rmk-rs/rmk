use embassy_rp::adc::{Adc, Async, Channel};
use embassy_time::{Duration, Timer};

use crate::core_traits::Runnable;
use crate::event::{BatteryAdcEvent, publish_event_async};

/// Samples a battery ADC channel using a 3.3 V reference.
///
/// Publishes ADC input millivolts; the battery processor applies the divider ratio.
pub struct Rp2040BatteryAdc<'d> {
    adc: Adc<'d, Async>,
    channel: Channel<'d>,
    interval: Duration,
}

impl<'d> Rp2040BatteryAdc<'d> {
    /// Creates a task that samples immediately, then waits `interval` between attempts.
    ///
    /// # Panics
    ///
    /// Panics if `interval` is zero.
    pub fn new(adc: Adc<'d, Async>, channel: Channel<'d>, interval: Duration) -> Self {
        assert!(interval.as_ticks() > 0, "battery ADC interval must be nonzero");
        Self { adc, channel, interval }
    }
}

impl Runnable for Rp2040BatteryAdc<'_> {
    async fn run(&mut self) -> ! {
        loop {
            match self.adc.read(&mut self.channel).await {
                Ok(raw) => {
                    let input_mv = (u32::from(raw) * 3300 / 4096) as u16;
                    publish_event_async(BatteryAdcEvent(input_mv)).await;
                }
                Err(_) => warn!("Battery ADC read failed"),
            }
            Timer::after(self.interval).await;
        }
    }
}
