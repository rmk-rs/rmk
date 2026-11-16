#![no_main]
#![no_std]

use core::convert::Infallible;
use core::ptr::NonNull;

use embassy_executor::Spawner;
use embassy_futures::yield_now;
use embedded_io_async::{Read, Write};
use panic_halt as _;
use rmk::config::{BehaviorConfig, LockConfig, PositionalConfig, RmkConfig};
use rmk::keymap::KeymapData;
use rmk::types::action::{EncoderAction, KeyAction};
use rmk::types::fork::{Fork, StateBits};
use rmk::types::modifier::ModifierCombination;
use rmk::types::morse::{Morse, MorseProfile};
use rmk::{initialize_keymap, k, layer};
use semihosting::println;
use static_cell::StaticCell;
use uart_16550::Uart16550;
use uart_16550::backend::MmioBackend;

struct Uart(Uart16550<MmioBackend>);

impl Uart {
    fn new() -> Self {
        let addr = NonNull::new(0x1000_0000usize as *mut u8).unwrap();
        let mut uart = unsafe { Uart16550::new_mmio(addr, 1) }.unwrap();
        uart.init(uart_16550::Config::default()).unwrap();
        Self(uart)
    }
}

impl embedded_io_async::ErrorType for Uart {
    type Error = Infallible;
}

impl Read for Uart {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        loop {
            let n = self.0.receive_bytes(buf);
            if n > 0 { return Ok(n) } else { yield_now().await }
        }
    }
}

impl Write for Uart {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        let mut sent = 0;
        while sent < buf.len() {
            let n = self.0.send_bytes(&buf[sent..]);
            if n > 0 { sent += n } else { yield_now().await }
        }
        Ok(sent)
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

const COL: usize = 3;
const ROW: usize = 3;
const NUM_LAYER: usize = 2;
const NUM_ENCODER: usize = 1;

#[rustfmt::skip]
const fn get_default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        layer!([
            [k!(Kp1), k!(Kp2), k!(Kp3)],
            [k!(Kp4), k!(Kp5), k!(Kp6)],
            [k!(Kp7), k!(Kp8), k!(Kp9)]
        ]),
        layer!([
            [k!(A), k!(B), k!(C)],
            [k!(D), k!(E), k!(F)],
            [k!(G), k!(H), k!(I)]
        ]),
    ]
}

const DEFAULT_ENCODER_MAP: [[EncoderAction; NUM_ENCODER]; NUM_LAYER] = [
    [EncoderAction::new(k!(KpPlus), k!(KpMinus))],
    [EncoderAction::new(k!(AudioVolUp), k!(AudioVolDown))],
];

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    println!("[RMK] starting");

    let mut rx = Uart::new();
    let mut tx = Uart::new();

    let mut keymap_data = KeymapData::new_with_encoder(get_default_keymap(), DEFAULT_ENCODER_MAP);
    let mut behavior_config = BehaviorConfig::default();
    behavior_config
        .fork
        .forks
        .push(Fork::new(
            k!(A),
            k!(B),
            k!(C),
            StateBits::default(),
            StateBits::default(),
            ModifierCombination::default(),
            true,
        ))
        .unwrap();
    behavior_config
        .morse
        .morses
        .push(Morse {
            profile: MorseProfile::const_default(),
            actions: rmk::heapless::LinearMap::new(),
        })
        .unwrap();
    let positional_config = PositionalConfig::default();
    let keymap = initialize_keymap(&mut keymap_data, &mut behavior_config, &positional_config).await;

    static RMK_CONFIG: StaticCell<RmkConfig<'static>> = StaticCell::new();
    let rmk_config = RMK_CONFIG.init(RmkConfig {
        lock_config: LockConfig {
            insecure: true,
            bootloader_requires_unlock: true,
            ..Default::default()
        },
        ..Default::default()
    });

    let service = rmk::host::HostService::new(&keymap, rmk_config);
    // A UART has no "connected" notion, so just restart the framing session on
    // every read/write error, dropping whatever frame was in flight.
    loop {
        service.run_session(&mut rx, &mut tx).await;
    }
}
