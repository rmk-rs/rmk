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
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive};
use embassy_nrf::interrupt::InterruptExt;
use embassy_nrf::nvmc::Nvmc;
use embassy_nrf::usb::{self, Driver};
use embassy_nrf::{bind_interrupts, peripherals, spim};
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
    USBD => usb::InterruptHandler<peripherals::USBD>;
    CLOCK_POWER => usb::vbus_detect::InterruptHandler;
    TWISPI0 => spim::InterruptHandler<peripherals::TWISPI0>;
});

type ExternalFlash = W25qNorFlash<spim::Spim<'static>, Output<'static>>;

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("RMK DFU ext start!");
    let mut config = embassy_nrf::config::Config::default();
    config.gpiote_interrupt_priority = embassy_nrf::interrupt::Priority::P3;
    config.time_interrupt_priority = embassy_nrf::interrupt::Priority::P3;
    embassy_nrf::interrupt::USBD.set_priority(embassy_nrf::interrupt::Priority::P2);
    embassy_nrf::interrupt::CLOCK_POWER.set_priority(embassy_nrf::interrupt::Priority::P2);
    config.debug = embassy_nrf::config::Debug::NotConfigured;
    let p = embassy_nrf::init(config);
    embassy_nrf::pac::CLOCK.tasks_hfclkstart().write_value(1);
    while embassy_nrf::pac::CLOCK.events_hfclkstarted().read() != 1 {}

    let driver = Driver::new(p.USBD, Irqs, usb::vbus_detect::HardwareVbusDetect::new(Irqs));

    let (row_pins, col_pins) =
        config_matrix_pins_nrf!(peripherals: p, input: [P0_07, P0_02, P0_11, P0_12], output: [P1_15, P0_31, P0_29]);

    // External DFU flash via SPIM0 (TWISPI0) — nRF SPIM is natively async.
    let mut dfu_spi_cfg = spim::Config::default();
    dfu_spi_cfg.frequency = spim::Frequency::M8;
    let dfu_spi = spim::Spim::new(p.TWISPI0, Irqs, p.P0_17, p.P0_20, p.P0_22, dfu_spi_cfg);
    let dfu_cs = Output::new(p.P0_24, Level::High, OutputDrive::Standard);

    let ext_flash = ExternalFlash::new(dfu_spi, dfu_cs, 8 * 1024 * 1024);

    let dfu_mutex =
        embassy_sync::mutex::Mutex::<embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex, _>::new(ext_flash);
    let dfu_partition = Partition::new(&dfu_mutex, 0, 8 * 1024 * 1024);

    // Internal flash: only the boot state and storage partitions are used, so
    // the internal DFU partition from the linkerscript layout is discarded.
    let flash_mutex = FlashMutex::new(async_flash_wrapper(Nvmc::new(p.NVMC)));
    let (storage_partition, state_partition, _) = partitions_from_linkerscript(&flash_mutex);

    let mut dfu_led_processor = rmk::processor::builtin::dfu_led::DfuLedProcessor::new(
        Output::new(p.P0_15, Level::Low, OutputDrive::Standard),
        false,
    );

    let keyboard_device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x4643,
        manufacturer: "Haobo",
        product_name: "RMK Keyboard nRF52840 DFU ext",
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
