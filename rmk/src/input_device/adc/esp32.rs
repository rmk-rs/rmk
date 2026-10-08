use esp_hal::Async;
use esp_hal::analog::adc::{Adc, AdcCalLine, AdcChannel, AdcPin};
use esp_hal::peripherals::ADC1;
use rmk_macro::input_device;

use crate::event::BatteryAdcEvent;

/// Samples a calibrated battery channel on ADC1.
///
/// Publishes ADC input millivolts; the battery processor applies the divider ratio.
#[input_device(publish = BatteryAdcEvent)]
pub struct Esp32BatteryAdc<'d, P: AdcChannel> {
    adc: Adc<'d, ADC1<'d>, Async>,
    pin: AdcPin<P, ADC1<'d>, AdcCalLine<ADC1<'d>>>,
}

impl<'d, P: AdcChannel> Esp32BatteryAdc<'d, P> {
    /// Creates an input device that returns calibrated ADC input millivolts.
    pub fn new(adc: Adc<'d, ADC1<'d>, Async>, pin: AdcPin<P, ADC1<'d>, AdcCalLine<ADC1<'d>>>) -> Self {
        Self { adc, pin }
    }

    async fn read_battery_adc_event(&mut self) -> BatteryAdcEvent {
        let input_mv = self.adc.read_oneshot(&mut self.pin).await;
        BatteryAdcEvent(input_mv)
    }
}
