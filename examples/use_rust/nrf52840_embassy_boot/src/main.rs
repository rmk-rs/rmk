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
use embassy_nrf::usb::{self, Driver};
use embassy_nrf::{bind_interrupts, peripherals};
use keymap::{COL, ROW};
use panic_probe as _;
use rmk::config::{BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
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
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("RMK start!");
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
        config_matrix_pins_nrf!(peripherals: p, input: [P0_07, P0_22, P0_11, P0_12], output: [P0_13, P0_17, P0_20]);

    // Flash partition layout comes from rmk-memory.x linker script.
    let flash = async_flash_wrapper(rmk::dfu::init_flash_from_linkerscript(p.NVMC));

    let mut dfu_led_processor = rmk::processor::builtin::dfu_led::DfuLedProcessor::new(
        Output::new(p.P0_15, Level::Low, OutputDrive::Standard),
        false,
    );

    let keyboard_device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x4643,
        manufacturer: "Haobo",
        product_name: "RMK Keyboard nRF52840 embassy-boot use_rust example",
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
        flash,
        &storage_config,
        &mut behavior_config,
        &per_key_config,
    )
    .await;

    rmk::dfu::mark_booted();

    // Optional DFU lock — requires the `dfu_lock` Cargo feature.
    // Specify the physical keys to press simultaneously to unlock DFU firmware
    // download. The keys are (row, col) pairs matching your matrix layout.
    // The lock state is checked by the DFU USB handler on each download start.
    // To use, create a `DfuLock` and poll it periodically:
    //
    // let unlock_keys: &[(u8, u8)] = &[(0, 0), (1, 1)];
    // let mut dfu_lock = ::rmk::dfu::DfuLock::new(unlock_keys, &keymap);
    // add dfu_lock to run_all!()

    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, ROW, COL, true>::new(row_pins, col_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);
    let host_service = HostService::new(&keymap, &rmk_config);

    let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config).with_host_service(&host_service);
    let mut wpm_processor = WpmProcessor::new();

    run_all!(
        matrix,
        storage,
        usb_transport,
        wpm_processor,
        keyboard,
        dfu_led_processor, // , dfu_lock
    )
    .await;
}
