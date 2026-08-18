use defmt::error;
use embassy_nrf::gpio::Output;
use embassy_nrf::spim::Spim;
use embassy_time::Timer;
use rmk::event::ConnectionStatusChangeEvent;
use rmk::macros::processor;
use rmk::types::ble::{BleState, BleStatus};
use rmk::types::constants::NUM_BLE_PROFILE;

pub(crate) const LEDS: usize = 2;

/// The bond slot `SwitchToDongle` selects, which profile cycling never reaches.
const DONGLE_PROFILE: u8 = NUM_BLE_PROFILE as u8;

/// Matches `poll_interval` below; the blink counts frames, not milliseconds.
const FPS: u32 = 30;
/// One second on, one second off.
const FRAMES_PER_BLINK: u32 = 2 * FPS;

/// Peak channel value. Two indicator LEDs at full scale would be both blinding
/// and a waste of battery.
const PEAK: u8 = 48;

#[derive(Clone, Copy, PartialEq)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

impl Rgb {
    const OFF: Self = Self { r: 0, g: 0, b: 0 };
    const WHITE: Self = Self {
        r: PEAK,
        g: PEAK,
        b: PEAK,
    };
    const BLUE: Self = Self {
        r: 0,
        g: 0,
        b: PEAK,
    };
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

    async fn show(&mut self, color: Rgb) {
        if color == Rgb::OFF {
            if self.powered {
                self.write_frame(Rgb::OFF).await;
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
        self.write_frame(color).await;
    }

    async fn write_frame(&mut self, color: Rgb) {
        for led_out in self.frame.chunks_exact_mut(3 * SPI_BITS_PER_BIT) {
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

#[processor(subscribe = [ConnectionStatusChangeEvent], poll_interval = 33)]
pub(crate) struct RgbService {
    leds: Ws2812,
    /// The colour to blink, or `Rgb::OFF` to stay dark.
    blink: Rgb,
    tick: u32,
}

impl RgbService {
    pub(crate) fn new(leds: Ws2812) -> Self {
        Self {
            leds,
            blink: Rgb::OFF,
            tick: 0,
        }
    }

    async fn on_connection_status_change_event(&mut self, event: ConnectionStatusChangeEvent) {
        let BleStatus { profile, state } = event.0.ble;
        self.blink = match state {
            // Both the dongle-seeking and the plain-host advertising paths report
            // `Advertising`; only the active slot tells them apart.
            BleState::Advertising if profile == DONGLE_PROFILE => Rgb::WHITE,
            BleState::Advertising => Rgb::BLUE,
            BleState::Connected | BleState::Inactive => Rgb::OFF,
        };
    }

    async fn poll(&mut self) {
        let lit = self.blink != Rgb::OFF && self.tick % FRAMES_PER_BLINK < FRAMES_PER_BLINK / 2;
        self.leds.show(if lit { self.blink } else { Rgb::OFF }).await;
        self.tick = self.tick.wrapping_add(1);
    }
}
