#![no_main]
#![no_std]

#[macro_use]
mod keymap;
#[macro_use]
mod macros;
mod vial;

use defmt::info;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::flash::Flash;
use embassy_rp::gpio::{Input, Level, Output};
use embassy_rp::spi::{self, Spi};
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_rp::{bind_interrupts, peripherals};
use keymap::{COL, ROW};
use panic_probe as _;
use rmk::config::{BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::dfu::{FlashDfuHandler, FlashMutex, Partition, partitions_from_linkerscript};
use rmk::driver::w25q::W25qNorFlash;
use rmk::host::HostService;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::processor::builtin::wpm::WpmProcessor;
use rmk::storage::async_flash_wrapper;
use rmk::usb::UsbTransport;
use rmk::{KeymapData, initialize_keymap_and_storage, run_all};
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<peripherals::USB>;
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH3>,
               embassy_rp::dma::InterruptHandler<embassy_rp::peripherals::DMA_CH4>;
});

type ExternalFlash = W25qNorFlash<spi::Spi<'static, peripherals::SPI0, spi::Async>, Output<'static>>;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("RMK DFU ext start!");
    let p = embassy_rp::init(Default::default());

    let driver = Driver::new(p.USB, Irqs);

    // Matrix pins. PIN_16–19 are used by the external DFU flash (SPI0) and
    // PIN_25 by the DFU LED, so the matrix uses other pins.
    let (row_pins, col_pins) =
        config_matrix_pins_rp!(peripherals: p, input: [PIN_6, PIN_7, PIN_8, PIN_9], output: [PIN_10, PIN_11, PIN_12]);

    // External DFU flash via SPI0 (async with DMA)
    let dfu_spi = Spi::new(
        p.SPI0,
        p.PIN_18,
        p.PIN_19,
        p.PIN_16,
        p.DMA_CH3,
        p.DMA_CH4,
        Irqs,
        spi::Config::default(),
    );
    let dfu_cs = Output::new(p.PIN_17, Level::High);
    let ext_flash = ExternalFlash::new(dfu_spi, dfu_cs, 8 * 1024 * 1024);

    // Park the external flash behind a mutex. The external flash becomes the
    // DFU download partition; the internal flash only holds the boot state
    // and storage partitions. Offsets come from the DFU symbols in memory.x.
    let dfu_mutex =
        embassy_sync::mutex::Mutex::<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, _>::new(ext_flash);
    let dfu_partition = Partition::new(&dfu_mutex, 0, 8 * 1024 * 1024);

    // Internal flash: only the boot state and storage partitions are used, so
    // the internal DFU partition from the linkerscript layout is discarded.
    let flash_mutex = FlashMutex::new(async_flash_wrapper(Flash::<
        _,
        embassy_rp::flash::Blocking,
        { rmk::dfu::FLASH_SIZE },
    >::new_blocking(p.FLASH)));
    let (storage_partition, state_partition, _) = partitions_from_linkerscript(&flash_mutex);

    let mut dfu_led_processor =
        rmk::processor::builtin::dfu_led::DfuLedProcessor::new(Output::new(p.PIN_25, Level::Low), false);

    let keyboard_device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x4643,
        manufacturer: "Haobo",
        product_name: "RMK Keyboard RP2040 DFU ext",
        serial_number: "vial:f64c2b3c:000001",
    };

    let vial_config = VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, &[(0, 0), (1, 1)]);

    let rmk_config = RmkConfig {
        device_config: keyboard_device_config,
        vial_config,
        ..Default::default()
    };

    let mut keymap_data = KeymapData::new(keymap::get_default_keymap());
    let storage_config = StorageConfig {
        num_sectors: 8,
        start_addr: 0,
        clear_storage: false,
        clear_layout: false,
    };
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

    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, ROW, COL, true>::new(row_pins, col_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);
    let host_service = HostService::new(&keymap, &rmk_config);

    let mut dfu_iface = FlashDfuHandler::new(dfu_partition, state_partition);
    let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config).with_host_service(&host_service);
    let mut wpm_processor = WpmProcessor::new();

    run_all!(
        matrix,
        storage,
        usb_transport,
        dfu_iface,
        wpm_processor,
        keyboard,
        dfu_led_processor,
    )
    .await;
}
