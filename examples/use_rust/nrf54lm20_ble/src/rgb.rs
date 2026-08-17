//! Status lighting for the board's two WS2812 LEDs.
//!
//! LED 0 mirrors the battery: breathing green while charging, blinking red when
//! low. LED 1 mirrors the BLE link: breathing in the profile's colour while
//! advertising, off once connected. Both dark otherwise, which also drops the
//! `RGB_PWR` rail.
//!
//! The data line `RGB` is `P0.04`, and `P0` belongs to the low-power power
//! domain — `PWM20`–`PWM22` live in the peripheral domain and cannot reach it.
//! `SPIM30` can, so each WS2812 bit is sent as five 4 MHz SPI bits: `0` becomes
//! `11000` (500 ns high) and `1` becomes `11100` (750 ns high), both inside the
//! LED's window, with the 1.25 µs bit period exact. Five bits per bit also means
//! one colour byte is exactly five SPI bytes.

use defmt::error;
use embassy_nrf::gpio::Output;
use embassy_nrf::spim::Spim;
use embassy_time::Timer;
use rmk::event::{BatteryStatusEvent, ConnectionStatusChangeEvent};
use rmk::macros::processor;
use rmk::types::battery::{BatteryStatus, ChargeState};
use rmk::types::ble::{BleState, BleStatus};

pub(crate) const LEDS: usize = 2;

/// Matches `poll_interval` below; the animations count frames, not milliseconds.
const FPS: u32 = 30;
const FRAMES_PER_BREATH: u32 = 2 * FPS;
const FRAMES_PER_BLINK: u32 = FPS;

/// Peak channel value. Two indicator LEDs at full scale would be both blinding
/// and a waste of battery.
const PEAK: u8 = 48;

const BATTERY_LOW_PERCENT: u8 = 10;

#[derive(Clone, Copy, PartialEq)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

impl Rgb {
    const OFF: Self = Self { r: 0, g: 0, b: 0 };
    const RED: Self = Self { r: PEAK, g: 0, b: 0 };
    const GREEN: Self = Self { r: 0, g: PEAK, b: 0 };
    const BLUE: Self = Self { r: 0, g: 0, b: PEAK };

    fn scaled(self, level: u8) -> Self {
        let scale = |c: u8| ((c as u16 * level as u16) >> 8) as u8;
        Self {
            r: scale(self.r),
            g: scale(self.g),
            b: scale(self.b),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Pattern {
    Off,
    Breathe(Rgb),
    Blink(Rgb),
}

const SPI_BITS_PER_BIT: usize = 5;
const CODE_0: u64 = 0b11000;
const CODE_1: u64 = 0b11100;
const FRAME_LEN: usize = LEDS * 3 * SPI_BITS_PER_BIT;

/// Drives the LED chain, and gates the `RGB_PWR` rail through `RGB_EN`.
pub(crate) struct Ws2812 {
    spi: Spim<'static>,
    power: Output<'static>,
    powered: bool,
    frame: [u8; FRAME_LEN],
}

impl Ws2812 {
    pub(crate) fn new(spi: Spim<'static>, power: Output<'static>) -> Self {
        Self {
            spi,
            power,
            powered: false,
            frame: [0; FRAME_LEN],
        }
    }

    async fn show(&mut self, colors: &[Rgb; LEDS]) {
        if colors.iter().all(|c| *c == Rgb::OFF) {
            if self.powered {
                self.power.set_low();
                self.powered = false;
            }
            return;
        }
        if !self.powered {
            self.power.set_high();
            self.powered = true;
            // Let the rail settle before the first frame reaches the LEDs.
            Timer::after_millis(1).await;
        }

        for (color, led_out) in colors.iter().zip(self.frame.chunks_exact_mut(3 * SPI_BITS_PER_BIT)) {
            // WS2812 wants green first.
            for (byte, byte_out) in [color.g, color.r, color.b]
                .iter()
                .zip(led_out.chunks_exact_mut(SPI_BITS_PER_BIT))
            {
                let mut bits: u64 = 0;
                for i in (0..8).rev() {
                    bits = (bits << SPI_BITS_PER_BIT) | if byte >> i & 1 == 1 { CODE_1 } else { CODE_0 };
                }
                byte_out.copy_from_slice(&bits.to_be_bytes()[3..]);
            }
        }

        if self.spi.write(&self.frame).await.is_err() {
            error!("WS2812 SPI write failed");
        }
    }
}

#[processor(subscribe = [BatteryStatusEvent, ConnectionStatusChangeEvent], poll_interval = 33)]
pub(crate) struct RgbService {
    leds: Ws2812,
    pattern: [Pattern; LEDS],
    tick: u32,
}

impl RgbService {
    pub(crate) fn new(leds: Ws2812) -> Self {
        Self {
            leds,
            pattern: [Pattern::Off; LEDS],
            tick: 0,
        }
    }

    async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
        let BatteryStatus::Available { charge_state, level } = event.into() else {
            return;
        };
        self.pattern[0] = if charge_state == ChargeState::Charging {
            Pattern::Breathe(Rgb::GREEN)
        } else if level.is_some_and(|l| l <= BATTERY_LOW_PERCENT) {
            Pattern::Blink(Rgb::RED)
        } else {
            Pattern::Off
        };
    }

    async fn on_connection_status_change_event(&mut self, event: ConnectionStatusChangeEvent) {
        let BleStatus { profile, state } = event.0.ble;
        self.pattern[1] = match state {
            // One colour per profile, so a glance says which host is being offered.
            BleState::Advertising => match profile {
                0 => Pattern::Breathe(Rgb::GREEN),
                1 => Pattern::Breathe(Rgb::RED),
                _ => Pattern::Breathe(Rgb::BLUE),
            },
            BleState::Connected | BleState::Inactive => Pattern::Off,
        };
    }

    async fn poll(&mut self) {
        let mut colors = [Rgb::OFF; LEDS];
        for (color, pattern) in colors.iter_mut().zip(self.pattern) {
            *color = match pattern {
                Pattern::Off => Rgb::OFF,
                Pattern::Breathe(c) => {
                    let half = FRAMES_PER_BREATH / 2;
                    let phase = self.tick % FRAMES_PER_BREATH;
                    let rising = if phase < half { phase } else { FRAMES_PER_BREATH - phase };
                    c.scaled((rising * 255 / half) as u8)
                }
                Pattern::Blink(c) => {
                    if self.tick % FRAMES_PER_BLINK < FRAMES_PER_BLINK / 2 {
                        c
                    } else {
                        Rgb::OFF
                    }
                }
            };
        }
        self.leds.show(&colors).await;
        self.tick = self.tick.wrapping_add(1);
    }
}
