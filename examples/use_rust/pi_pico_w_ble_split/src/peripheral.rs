#![no_std]
#![no_main]

#[macro_use]
mod macros;

use bt_hci::controller::ExternalController;
use cyw43::aligned_bytes;
use cyw43_pio::PioSpi;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::flash::{Async, Flash};
use embassy_rp::gpio::{Input, Level, Output};
use embassy_rp::peripherals::{DMA_CH0, DMA_CH1, DMA_CH2, PIO0};
use embassy_rp::pio::{self, Pio};
use embassy_time as _;
use panic_probe as _;
use rmk::config::StorageConfig;
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::futures::future::join;
use rmk::matrix::Matrix;
use rmk::run_all;
use rmk::split::peripheral::run_rmk_split_peripheral;
use rmk::storage::new_storage_without_keymap;
use rmk::watchdog::Rp2040Watchdog;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<DMA_CH0>, embassy_rp::dma::InterruptHandler<DMA_CH1>, embassy_rp::dma::InterruptHandler<DMA_CH2>;
});

#[embassy_executor::task]
async fn cyw43_task(
    runner: cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, PioSpi<'static, PIO0, 0>>, cyw43::Cyw43439>,
) -> ! {
    runner.run().await
}

const FLASH_SIZE: usize = 2 * 1024 * 1024;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    #[cfg(feature = "skip-cyw43-firmware")]
    let (fw, clm, btfw, nvram) = {
        static EMPTY: &cyw43::Aligned<cyw43::A4, [u8]> = &cyw43::Aligned([0u8; 0]);
        (EMPTY, &[] as &[u8], EMPTY, EMPTY)
    };

    #[cfg(not(feature = "skip-cyw43-firmware"))]
    let (fw, clm, btfw, nvram) = {
        // IMPORTANT
        //
        // Download and make sure these files from https://github.com/embassy-rs/embassy/tree/main/cyw43-firmware
        // are available in `./examples/rp-pico-w`. (should be automatic)
        //
        // IMPORTANT
        let fw = aligned_bytes!("../cyw43-firmware/43439A0.bin");
        let clm = aligned_bytes!("../cyw43-firmware/43439A0_clm.bin");
        let btfw = aligned_bytes!("../cyw43-firmware/43439A0_btfw.bin");
        let nvram = aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin");
        (fw, clm, btfw, nvram)
    };

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);
    let mut pio = Pio::new(p.PIO0, Irqs);
    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        cyw43_pio::DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        embassy_rp::dma::Channel::new(p.DMA_CH0, Irqs),
        embassy_rp::dma::Channel::new(p.DMA_CH2, Irqs),
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (_net_device, bt_device, mut control, runner) =
        cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw, nvram).await;
    spawner.spawn(cyw43_task(runner).unwrap());
    control.init(clm).await;
    let controller: ExternalController<_, 10> = ExternalController::new(bt_device);

    // Storage config
    let flash = Flash::<_, Async, FLASH_SIZE>::new(p.FLASH, p.DMA_CH1, Irqs);
    let storage_config = StorageConfig {
        start_addr: 0x100000, // Start from 1M
        num_sectors: 32,
        ..Default::default()
    };
    let mut storage = new_storage_without_keymap(flash, storage_config).await;

    // Pin config
    let (row_pins, col_pins) = config_matrix_pins_rp!(peripherals: p, input: [PIN_6, PIN_7], output: [PIN_19, PIN_20]);
    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, 2, 2, true>::new(row_pins, col_pins, debouncer);

    let ble_addr = [0x7e, 0xfe, 0x73, 0x9e, 0x66, 0xe3];

    let mut watchdog_runner = Rp2040Watchdog::default_runner(embassy_rp::watchdog::Watchdog::new(p.WATCHDOG));

    // Start
    join(
        run_all!(matrix, storage, watchdog_runner),
        run_rmk_split_peripheral(0, controller, ble_addr),
    )
    .await;
}
