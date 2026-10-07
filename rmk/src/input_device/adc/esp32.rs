use embassy_time::{Duration, Timer};
use esp_hal::Async;
use esp_hal::analog::adc::{Adc, AdcCalLine, AdcChannel, AdcPin};
use esp_hal::peripherals::ADC1;

use crate::core_traits::Runnable;
use crate::event::{BatteryAdcEvent, publish_event_async};

/// Samples a calibrated battery channel on ADC1.
///
/// Publishes ADC input millivolts; the battery processor applies the divider ratio.
pub struct Esp32BatteryAdc<'d, P: AdcChannel> {
    adc: Adc<'d, ADC1<'d>, Async>,
    pin: AdcPin<P, ADC1<'d>, AdcCalLine<ADC1<'d>>>,
    interval: Duration,
}

impl<'d, P: AdcChannel> Esp32BatteryAdc<'d, P> {
    /// Creates a task that samples immediately, then waits `interval` between readings.
    ///
    /// # Panics
    ///
    /// Panics if `interval` is zero.
    pub fn new(
        adc: Adc<'d, ADC1<'d>, Async>,
        pin: AdcPin<P, ADC1<'d>, AdcCalLine<ADC1<'d>>>,
        interval: Duration,
    ) -> Self {
        assert!(interval.as_ticks() > 0, "battery ADC interval must be nonzero");
        Self { adc, pin, interval }
    }
}

impl<P: AdcChannel> Runnable for Esp32BatteryAdc<'_, P> {
    async fn run(&mut self) -> ! {
        loop {
            let input_mv = self.adc.read_oneshot(&mut self.pin).await;
            publish_event_async(BatteryAdcEvent(input_mv)).await;
            Timer::after(self.interval).await;
        }
    }
}
