use core::cell::Cell;
use core::num::NonZeroU32;

use embassy_sync::blocking_mutex::Mutex;
use embedded_hal::digital::InputPin;
use rmk_macro::{input_device, processor};
use rmk_types::battery::{BatteryStatus, ChargeState};

use crate::RawMutex;
use crate::event::{BatteryAdcEvent, BatteryStatusEvent, ChargingStateEvent, publish_event};

/// Cached battery status, updated by [`BatteryProcessor::commit`] alongside every
/// [`BatteryStatusEvent`] publish so host services can read the current value
/// synchronously without subscribing to the event stream.
pub(crate) static BATTERY_STATUS: Mutex<RawMutex, Cell<BatteryStatus>> =
    Mutex::new(Cell::new(BatteryStatus::Unavailable));

/// Latest battery status reported by the battery processor.
pub fn current_battery_status() -> BatteryStatus {
    BATTERY_STATUS.lock(|c| c.get())
}

/// Reads charging state from a GPIO pin and publishes ChargingStateEvent.
///
/// This input device monitors a charging state pin and publishes events when
/// the charging state changes.
#[input_device(publish = ChargingStateEvent)]
pub struct ChargingStateReader<I: InputPin> {
    // Charging state pin or standby pin
    state_input: I,
    // True: low represents charging, False: high represents charging
    low_active: bool,
    current_charging_state: Option<bool>,
}

impl<I: InputPin> ChargingStateReader<I> {
    pub fn new(state_input: I, low_active: bool) -> Self {
        Self {
            state_input,
            low_active,
            current_charging_state: None,
        }
    }

    /// Read the charging state and return an event.
    /// This method waits until there's a state change to report.
    async fn read_charging_state_event(&mut self) -> ChargingStateEvent {
        loop {
            let delay = if self.current_charging_state.is_none() { 2 } else { 5 };
            embassy_time::Timer::after_secs(delay).await;

            let charging_state = if self.low_active {
                self.state_input.is_low().unwrap_or(false)
            } else {
                self.state_input.is_high().unwrap_or(false)
            };

            if self.current_charging_state != Some(charging_state) {
                self.current_charging_state = Some(charging_state);
                return ChargingStateEvent {
                    charging: charging_state,
                };
            }
        }
    }
}

/// BatteryProcessor processes battery adc value and charging state,
/// emits `BatteryStatusEvent` when battery status changes.
#[processor(subscribe = [BatteryAdcEvent, ChargingStateEvent])]
pub struct BatteryProcessor {
    adc_divider: Option<(NonZeroU32, NonZeroU32)>,
    /// Current battery status
    battery_status: BatteryStatus,
}

impl BatteryProcessor {
    pub const fn new(adc_divider_measured: u32, adc_divider_total: u32) -> Self {
        let adc_divider = match (
            NonZeroU32::new(adc_divider_measured),
            NonZeroU32::new(adc_divider_total),
        ) {
            (Some(measured), Some(total)) => Some((measured, total)),
            _ => None,
        };

        Self {
            adc_divider,
            battery_status: BatteryStatus::Unavailable,
        }
    }

    /// Reports charger state without measuring a battery percentage.
    /// Run a `ChargingStateReader` alongside this processor.
    pub fn charging_only() -> Self {
        Self {
            adc_divider: None,
            battery_status: BatteryStatus::Unavailable,
        }
    }

    /// Apply a new battery status: persist on the processor, mirror into
    /// [`BATTERY_STATUS`] for synchronous readers, and broadcast via
    /// [`BatteryStatusEvent`].
    fn commit(&mut self, status: BatteryStatus) {
        self.battery_status = status;
        BATTERY_STATUS.lock(|c| c.set(status));
        publish_event(BatteryStatusEvent::from(status));
    }

    fn get_battery_percent(&self, val: u16) -> Option<u8> {
        let (divider_measured, divider_total) = self.adc_divider?;

        let battery_mv = u64::from(val) * u64::from(divider_total.get()) / u64::from(divider_measured.get());
        Some((battery_mv.saturating_sub(3600) / 6).min(100) as u8)
    }

    async fn on_battery_adc_event(&mut self, event: BatteryAdcEvent) {
        let val = event.0;
        trace!("Detected battery ADC value: {:?}", val);

        let Some(battery_percent) = self.get_battery_percent(val) else {
            return;
        };
        match self.battery_status {
            // Skip ADC updates while charging
            BatteryStatus::Available {
                charge_state: ChargeState::Charging,
                ..
            } => {}
            // Not charging: publish if the percentage changed.
            BatteryStatus::Available { charge_state, level } => {
                if level != Some(battery_percent) {
                    self.commit(BatteryStatus::Available {
                        charge_state,
                        level: Some(battery_percent),
                    });
                }
            }
            // First ADC reading: transition from Unavailable.
            BatteryStatus::Unavailable => {
                self.commit(BatteryStatus::Available {
                    charge_state: ChargeState::Unknown,
                    level: Some(battery_percent),
                });
            }
        }
    }

    async fn on_charging_state_event(&mut self, event: ChargingStateEvent) {
        let charging = event.charging;
        info!("Charging state changed: {:?}", charging);

        let level = match self.battery_status {
            // ADC updates pause during charging, so refresh the level when charging ends.
            BatteryStatus::Available {
                charge_state: ChargeState::Charging,
                ..
            } if !charging => None,
            BatteryStatus::Available { level, .. } => level,
            BatteryStatus::Unavailable => None,
        };

        self.commit(BatteryStatus::Available {
            charge_state: charging.into(),
            level,
        });
    }
}

#[cfg(test)]
mod tests {
    use core::pin::pin;

    use embassy_futures::join::join3;
    use embassy_time::{Duration, MockDriver};
    use embedded_hal_mock::eh1::digital::{Mock, State, Transaction};
    use futures::poll;
    use rmk_types::battery::{BatteryStatus, ChargeState};

    use super::{BatteryProcessor, ChargingStateReader, current_battery_status};
    use crate::core_traits::Runnable;
    use crate::processor::builtin::battery_led::BatteryLedProcessor;
    use crate::test_support::test_block_on;

    #[test]
    fn zero_measured_divider_has_no_battery_percent() {
        let processor = const { BatteryProcessor::new(0, 1) };
        assert_eq!(processor.get_battery_percent(4000), None);
    }

    #[test]
    fn zero_total_divider_has_no_battery_percent() {
        let processor = const { BatteryProcessor::new(1, 0) };
        assert_eq!(processor.get_battery_percent(4000), None);
    }

    #[test]
    fn both_zero_dividers_have_no_battery_percent() {
        let processor = const { BatteryProcessor::new(0, 0) };
        assert_eq!(processor.get_battery_percent(4000), None);
    }

    #[test]
    fn charging_only_has_no_battery_percent() {
        assert_eq!(BatteryProcessor::charging_only().get_battery_percent(4000), None);
    }

    #[test]
    fn valid_adc_reading_can_report_zero_percent() {
        let processor = const { BatteryProcessor::new(1, 1) };
        assert_eq!(processor.get_battery_percent(3600), Some(0));
    }

    #[test]
    fn adc_divider_scales_voltage() {
        assert_eq!(BatteryProcessor::new(1, 5).get_battery_percent(800), Some(66));
        assert_eq!(
            BatteryProcessor::new(u32::MAX, u32::MAX).get_battery_percent(4000),
            Some(66)
        );
    }

    #[test]
    fn initial_discharging_state_is_reported() {
        let mut input = Mock::new(&[Transaction::get(State::High)]);
        let mut charging = ChargingStateReader::new(input.clone(), true);
        let event = test_block_on(charging.read_charging_state_event());
        assert!(!event.charging);
        input.done();
    }

    #[test]
    fn charging_only_pipeline_drives_led_without_inventing_a_battery_level() {
        test_block_on(async {
            let mut input = Mock::new(&[Transaction::get(State::Low), Transaction::get(State::High)]);
            let mut output = Mock::new(&[
                Transaction::set(State::Low),
                // The LED's ready tick runs before its state event; it updates on the next tick.
                Transaction::set(State::Low),
                Transaction::set(State::High),
                Transaction::set(State::High),
                Transaction::set(State::High),
                Transaction::set(State::High),
                Transaction::set(State::High),
                Transaction::set(State::Low),
            ]);
            let mut charging = ChargingStateReader::new(input.clone(), true);
            let mut battery = BatteryProcessor::charging_only();
            let mut led = BatteryLedProcessor::new(output.clone(), false);
            let mut run = pin!(join3(charging.run(), battery.run(), led.run()));
            assert!(poll!(run.as_mut()).is_pending());

            for second in 1..=8 {
                MockDriver::get().advance(Duration::from_secs(1));
                assert!(poll!(run.as_mut()).is_pending());
                assert!(poll!(run.as_mut()).is_pending());
                if matches!(second, 2 | 7) {
                    assert_eq!(
                        current_battery_status(),
                        BatteryStatus::Available {
                            charge_state: if second < 7 {
                                ChargeState::Charging
                            } else {
                                ChargeState::Discharging
                            },
                            level: None,
                        }
                    );
                }
            }
            input.done();
            output.done();
        });
    }
}
