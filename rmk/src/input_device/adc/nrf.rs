use embassy_nrf::gpio::Output;
use embassy_nrf::saadc::Saadc;
use embassy_time::{Duration, Instant};
use rmk_macro::{Event, input_device};

use super::{AdcState, AnalogEventType};
use crate::event::{Axis, AxisEvent, AxisValType, BatteryAdcEvent, PointingEvent};
use crate::input_device::joystick_power::{IdleTracker, JoystickPowerConfig, adc_axis, apply_deadzone};

struct PowerSampling<'a, const N: usize> {
    config: JoystickPowerConfig,
    pins: [Option<Output<'a>>; N],
    bias: [[i16; 3]; N],
    deadzones: [u16; N],
    idle: IdleTracker,
    next_sample: Instant,
    battery_due: Instant,
    boot_done: bool,
    sample_valid: bool,
    battery_ready: bool,
    warned: bool,
}

/// A cancelled sampling future must not leave a switched joystick powered.
struct SupplyGuard<'s, 'a, const N: usize> {
    pins: &'s mut [Option<Output<'a>>; N],
    keep_on: bool,
}

impl<const N: usize> Drop for SupplyGuard<'_, '_, N> {
    fn drop(&mut self) {
        if !self.keep_on {
            for pin in self.pins.iter_mut().flatten() {
                pin.set_low();
            }
        }
    }
}

/// Events produced by NrfAdc.
#[derive(Event, Clone, Debug)]
pub enum NrfAdcEvent {
    Pointing(PointingEvent),
    Battery(BatteryAdcEvent),
}

#[input_device(publish = NrfAdcEvent)]
pub struct NrfAdc<'a, const PIN_NUM: usize, const EVENT_NUM: usize> {
    saadc: Saadc<'a, PIN_NUM>,
    polling_interval: Duration,
    light_sleep: Option<Duration>,
    buf: [[i16; PIN_NUM]; 2],
    event_type: [AnalogEventType; EVENT_NUM],
    /// Device id emitted in PointingEvent for each event slot.
    /// Indexed by event_state; irrelevant for Battery slots (use 0).
    event_device_ids: [u8; EVENT_NUM],
    event_state: u8,
    channel_state: u8,
    buf_state: bool,
    adc_state: AdcState,
    active_instant: Instant,
    power: Option<PowerSampling<'a, EVENT_NUM>>,
}

impl<'a, const PIN_NUM: usize, const EVENT_NUM: usize> NrfAdc<'a, PIN_NUM, EVENT_NUM> {
    pub fn new(
        saadc: Saadc<'a, PIN_NUM>,
        event_type: [AnalogEventType; EVENT_NUM],
        event_device_ids: [u8; EVENT_NUM],
        polling_interval: Duration,
        light_sleep: Option<Duration>,
    ) -> Self {
        Self {
            saadc,
            polling_interval,
            event_type,
            event_device_ids,
            light_sleep,
            buf: [[0; PIN_NUM]; 2],
            event_state: 0,
            channel_state: 0,
            buf_state: false,
            adc_state: AdcState::LightSleep,
            active_instant: Instant::MIN,
            power: None,
        }
    }

    pub fn with_power_management(
        mut self,
        config: JoystickPowerConfig,
        pins: [Option<Output<'a>>; EVENT_NUM],
        bias: [[i16; 3]; EVENT_NUM],
        deadzones: [u16; EVENT_NUM],
    ) -> Self {
        self.power = Some(PowerSampling {
            config,
            pins,
            bias,
            deadzones,
            idle: IdleTracker::default(),
            next_sample: Instant::MIN,
            battery_due: Instant::MIN,
            boot_done: false,
            sample_valid: false,
            battery_ready: false,
            warned: false,
        });
        self.event_state = EVENT_NUM as u8;
        self
    }
}

impl<'a, const PIN_NUM: usize, const EVENT_NUM: usize> NrfAdc<'a, PIN_NUM, EVENT_NUM> {
    async fn read_nrf_adc_event(&mut self) -> NrfAdcEvent {
        if self.power.is_some() {
            return self.read_powered_event().await;
        }
        loop {
            if self.active_instant == Instant::MIN {
                self.saadc.sample(&mut self.buf[1]).await;
                self.active_instant = Instant::now();
            } else if let Some(light_sleep) = self.light_sleep
                && self.adc_state == AdcState::LightSleep
            {
                embassy_time::Timer::after(light_sleep).await;
            } else {
                embassy_time::Timer::after(self.polling_interval).await;
            }

            if self.active_instant.elapsed().as_millis() > 1200 {
                self.adc_state = AdcState::LightSleep;
            }

            if self.event_state == EVENT_NUM as u8 {
                if self.channel_state != PIN_NUM as u8 {
                    error!("NrfAdc's pin size and event's required is mismatch");
                }
                self.buf_state = !self.buf_state;
                let buf = if self.buf_state {
                    &mut self.buf[0]
                } else {
                    &mut self.buf[1]
                };
                self.saadc.sample(buf).await;
                for (a, b) in self.buf[0].iter().zip(self.buf[1].iter()) {
                    if i16::abs(a - b) > 150 {
                        debug!("ADC Active");
                        self.adc_state = AdcState::Active;
                        self.active_instant = Instant::now();
                        break;
                    }
                }
                self.channel_state = 0;
                self.event_state = 0;
            }

            let buf = if self.buf_state { &self.buf[0] } else { &self.buf[1] };

            match self.event_type[self.event_state as usize] {
                AnalogEventType::Joystick(sz) => {
                    let mut e = [
                        AxisEvent {
                            typ: AxisValType::Rel,
                            axis: Axis::X,
                            value: 0,
                        },
                        AxisEvent {
                            typ: AxisValType::Rel,
                            axis: Axis::Y,
                            value: 0,
                        },
                        AxisEvent {
                            typ: AxisValType::Rel,
                            axis: Axis::Z,
                            value: 0,
                        },
                    ];
                    if sz > 3 || sz == 0 {
                        error!("Joystick with more than 3 dimensions or empty is not supported. Skip this event");
                        self.event_state += 1;
                        continue;
                    } else {
                        for i in 0..core::cmp::min(sz, 2) {
                            e[i as usize].value = adc_axis(buf[self.channel_state as usize]);
                            self.channel_state += 1;
                        }
                    }
                    let device_id = self.event_device_ids[self.event_state as usize];
                    self.event_state += 1;
                    return NrfAdcEvent::Pointing(PointingEvent { device_id, axes: e });
                }
                AnalogEventType::Battery => {
                    let battery_adc_value = buf[self.channel_state as usize] as u16;
                    self.channel_state += 1;
                    self.event_state += 1;
                    return NrfAdcEvent::Battery(BatteryAdcEvent(battery_adc_value));
                }
            };
        }
    }

    async fn read_powered_event(&mut self) -> NrfAdcEvent {
        loop {
            if self.event_state == EVENT_NUM as u8 {
                self.sample_powered().await;
                self.event_state = 0;
                self.channel_state = 0;
            }
            let event_index = self.event_state as usize;
            self.event_state += 1;
            let power = self.power.as_ref().unwrap();
            match self.event_type[event_index] {
                AnalogEventType::Joystick(size) => {
                    let first = self.channel_state as usize;
                    self.channel_state += size;
                    if !power.sample_valid {
                        continue;
                    }
                    let mut axes = [
                        AxisEvent {
                            typ: AxisValType::Rel,
                            axis: Axis::X,
                            value: 0,
                        },
                        AxisEvent {
                            typ: AxisValType::Rel,
                            axis: Axis::Y,
                            value: 0,
                        },
                        AxisEvent {
                            typ: AxisValType::Rel,
                            axis: Axis::Z,
                            value: 0,
                        },
                    ];
                    for (axis, raw) in axes.iter_mut().zip(&self.buf[0][first..first + size as usize]) {
                        axis.value = adc_axis(*raw);
                    }
                    return NrfAdcEvent::Pointing(PointingEvent {
                        device_id: self.event_device_ids[event_index],
                        axes,
                    });
                }
                AnalogEventType::Battery => {
                    let raw = self.buf[0][self.channel_state as usize];
                    self.channel_state += 1;
                    if power.battery_ready {
                        return NrfAdcEvent::Battery(BatteryAdcEvent(raw.max(0) as u16));
                    }
                }
            }
        }
    }

    async fn sample_powered(&mut self) {
        let power = self.power.as_mut().unwrap();
        embassy_time::Timer::at(power.next_sample).await;
        let config = power.config;
        let was_idle = power.idle.idle;
        let switched = power.pins.iter().any(Option::is_some);
        power.sample_valid = false;
        power.battery_ready = false;

        let mut supply = SupplyGuard {
            pins: &mut power.pins,
            keep_on: false,
        };
        for pin in supply.pins.iter_mut().flatten() {
            pin.set_high();
        }
        if !power.boot_done {
            embassy_time::Timer::after_millis(u64::from(config.boot_settle_ms)).await;
            power.boot_done = true;
        }

        let started = Instant::now();
        let window = if switched {
            config.on_us(was_idle)
        } else {
            config.period_us(was_idle)
        };
        let sample_deadline = started + Duration::from_micros(window);
        if config.can_sample(was_idle, switched) {
            // nRF52's core runs at 64 MHz. Interrupts remain enabled; a preemption can extend this wait.
            cortex_m::asm::delay(config.sample_settle_us.saturating_mul(64));
            if Instant::now() < sample_deadline {
                power.sample_valid = embassy_time::with_deadline(sample_deadline, self.saadc.sample(&mut self.buf[0]))
                    .await
                    .is_ok();
            }
        }

        if power.sample_valid {
            let mut channel = 0usize;
            let mut centered = true;
            for (slot, event) in self.event_type.iter().enumerate() {
                match event {
                    AnalogEventType::Joystick(size) => {
                        for axis in 0..2 {
                            let value = adc_axis(self.buf[0][channel + axis]).saturating_add(power.bias[slot][axis]);
                            centered &= apply_deadzone(value, power.deadzones[slot]) == 0;
                        }
                        channel += *size as usize;
                    }
                    AnalogEventType::Battery => channel += 1,
                }
            }
            power.idle.observe(Instant::now().as_micros(), centered);
            // Use the newly selected mode, including when entering idle from continuous power.
            supply.keep_on = config.duty(power.idle.idle) == 1000;
        } else {
            power.idle.observe(Instant::now().as_micros(), false);
            if !power.warned {
                warn!("Joystick sample skipped: settling/ADC did not fit supply window");
                power.warned = true;
            }
        }
        // ADC is finished: switch off before any further await, without filling the duty budget.
        drop(supply);

        // One SAADC owns all channels. Battery scans use a separate buffer and cannot wake the joystick.
        if Instant::now() >= power.battery_due && self.event_type.iter().any(|e| matches!(e, AnalogEventType::Battery))
        {
            embassy_time::Timer::after_millis(2).await;
            power.battery_ready =
                embassy_time::with_timeout(Duration::from_millis(5), self.saadc.sample(&mut self.buf[1]))
                    .await
                    .is_ok();
            if power.battery_ready {
                let mut channel = 0usize;
                for event in &self.event_type {
                    match event {
                        AnalogEventType::Joystick(size) => channel += *size as usize,
                        AnalogEventType::Battery => {
                            self.buf[0][channel] = self.buf[1][channel];
                            channel += 1;
                        }
                    }
                }
            }
            power.battery_due = Instant::now() + Duration::from_secs(30);
        }
        // Skip missed slots, rather than issuing a burst of catch-up power pulses.
        power.next_sample = (started + Duration::from_micros(config.period_us(power.idle.idle)))
            .max(Instant::now() + Duration::from_ticks(1));
    }
}
