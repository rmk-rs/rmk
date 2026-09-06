#![no_std]
#![no_main]

#[macro_use]
mod keymap;
#[macro_use]
mod macros;
mod vial;

use bt_hci::controller::ExternalController;
use cyw43::aligned_bytes;
use cyw43_pio::PioSpi;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::flash::Flash;
use embassy_rp::gpio::{Input, Level, Output};
use embassy_rp::peripherals::{DMA_CH0, DMA_CH1, DMA_CH2, PIO0, USB};
use embassy_rp::pio::{self, Pio};
use embassy_rp::usb::{self, Driver};
use embassy_time as _;
use keymap::{COL, ROW};
use panic_probe as _;
use rmk::ble::BleTransport;
use rmk::config::{BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::dfu::{FlashDfuHandler, FlashMutex, partitions_from_linkerscript};
use rmk::host::HostService;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::processor::builtin::wpm::WpmProcessor;
use rmk::split::PeripheralMatrixConfig;
use rmk::usb::UsbTransport;
use rmk::watchdog::Rp2040Watchdog;
use rmk::{KeymapData, initialize_keymap_and_storage, run_all};
use static_cell::StaticCell;
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => usb::InterruptHandler<USB>;
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

    // Create the usb driver, from the HAL
    let driver = Driver::new(p.USB, Irqs);

    // Pin config
    let (row_pins, col_pins) =
        config_matrix_pins_rp!(peripherals: p, input: [PIN_6, PIN_7, PIN_8, PIN_9], output: [PIN_19, PIN_20, PIN_21]);

    // Use internal flash to emulate eeprom
    // DFU requires blocking flash + async_flash_wrapper
    let flash_mutex = FlashMutex::new(rmk::storage::async_flash_wrapper(Flash::<
        _,
        embassy_rp::flash::Blocking,
        FLASH_SIZE,
    >::new_blocking(p.FLASH)));
    let (storage_partition, state_partition, dfu_partition) = partitions_from_linkerscript(&flash_mutex);

    let keyboard_device_config = DeviceConfig {
        vid: 0x4c4c,
        pid: 0x464c,
        manufacturer: "Haobo",
        product_name: "RMK PicoW Split",
        ..DeviceConfig::default()
    };

    let vial_config = VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, &[(0, 0), (1, 1)]);

    let storage_config = StorageConfig {
        start_addr: 0x100000, // Start from 1M
        num_sectors: 32,
        ..Default::default()
    };

    let rmk_config = RmkConfig {
        device_config: keyboard_device_config,
        vial_config,
        storage_config,
        ..Default::default()
    };

    // Initialize the storage and keymap
    let mut keymap_data = KeymapData::new(keymap::get_default_keymap());
    let mut behavior_config = BehaviorConfig::default();
    let per_key_config = PositionalConfig::default();
    let (keymap, mut storage) = initialize_keymap_and_storage(
        &mut keymap_data,
        storage_partition,
        &storage_config,
        &mut behavior_config,
        &per_key_config,
    )
    .await;

    // Initialize the matrix + keyboard
    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, ROW, COL, true>::new(row_pins, col_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);
    let host_service = HostService::new(&keymap, &rmk_config);

    let ble_addr = [0x18, 0xe2, 0x21, 0x88, 0xc0, 0xc7];

    let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config).with_host_service(&host_service);
    // The other half: 4x7 at row offset 4 (keymap rows 4..8).
    let mut ble_transport = BleTransport::new(
        controller,
        ble_addr,
        rmk_config,
        [PeripheralMatrixConfig {
            rows: 4,
            cols: 7,
            row_offset: 4,
            col_offset: 0,
        }],
    )
    .with_host_service(&host_service);
    let mut wpm_processor = WpmProcessor::new();
    let mut dfu_handler = FlashDfuHandler::new(dfu_partition, state_partition);
    let mut watchdog_runner = Rp2040Watchdog::default_runner(embassy_rp::watchdog::Watchdog::new(p.WATCHDOG));

    // Start
    run_all!(
        matrix,
        storage,
        usb_transport,
        ble_transport,
        wpm_processor,
        keyboard,
        watchdog_runner,
        dfu_handler
    )
    .await;
}
