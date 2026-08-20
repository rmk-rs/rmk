#![no_std]
#![no_main]

mod keymap;

#[macro_use]
mod macros;
mod vial;

use core::ptr::addr_of_mut;

use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::otg_fs::Usb;
use esp_hal::otg_fs::asynch::{Config, Driver};
use esp_hal::rng::TrngSource;
use esp_radio::Controller;
use esp_radio::ble::controller::BleConnector;
use esp_storage::FlashStorage;

use keymap::*;
use rmk::ble::build_ble_stack;
use rmk::channel::EVENT_CHANNEL;
use rmk::config::{
    BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig,
};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::futures::future::join5;
use rmk::input_device::Runnable;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::matrix::OffsetMatrixWrapper;
use rmk::split::ble::central::{read_peripheral_addresses, scan_peripherals};
use rmk::split::central::run_peripheral_manager;
use rmk::storage::async_flash_wrapper;
use rmk::{HostResources, initialize_keymap_and_storage, run_devices, run_rmk};
use static_cell::StaticCell;
use {esp_alloc as _, esp_backtrace as _};
use {esp_alloc as _, esp_backtrace as _};

use crate::vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};

::esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_s: Spawner) {
    // Initialize the peripherals and bluetooth controller
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let timg0 = esp_hal::timer::timg::TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0);
    esp_alloc::heap_allocator!(size: 72 * 1024);
    let _trng_source = TrngSource::new(peripherals.RNG, peripherals.ADC1);
    let mut rng = esp_hal::rng::Trng::try_new().unwrap();
    static RADIO: StaticCell<Controller<'static>> = StaticCell::new();
    let radio = RADIO.init(esp_radio::init().unwrap());
    let bluetooth = peripherals.BT;
    let connector = BleConnector::new(radio, bluetooth, Default::default()).unwrap();
    let controller: ExternalController<_, 20> = ExternalController::new(connector);
    let central_addr = [0x18, 0xe2, 0x21, 0x80, 0xc0, 0xc7];
    let mut host_resources = HostResources::new();
    let stack = build_ble_stack(controller, central_addr, &mut rng, &mut host_resources).await;

    // Initialize USB
    static mut EP_MEMORY: [u8; 1024] = [0; 1024];
    let usb = Usb::new(peripherals.USB0, peripherals.GPIO20, peripherals.GPIO19);
    let config = Config::default();
    let usb_driver = Driver::new(usb, unsafe { &mut *addr_of_mut!(EP_MEMORY) }, config);

    // Initialize the flash
    let flash = FlashStorage::new(peripherals.FLASH);
    let flash = async_flash_wrapper(flash);

    // Initialize the IO pins
    let (row_pins, col_pins) = config_matrix_pins_esp!(peripherals: peripherals, input: [GPIO1, GPIO2, GPIO3, GPIO4, GPIO5, GPIO6], output: [GPIO13, GPIO12, GPIO11, GPIO10, GPIO9, GPIO8, GPIO7]);

    let keyboard_device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x4643,
        manufacturer: "Haobo",
        product_name: "RMK Keyboard",
        serial_number: "vial:f64c2b3c:000001",
        ..DeviceConfig::default()
    };

    // RMK config
    const UNLOCK_KEYS: &[(u8, u8)] = &[(0, 0), (1, 1)];
    let vial_config = VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, UNLOCK_KEYS);
    let storage_config = StorageConfig {
        start_addr: 0x3f0000,
        num_sectors: 16,
        ..Default::default()
    };
    let rmk_config = RmkConfig {
        device_config: keyboard_device_config,
        vial_config,
        storage_config,
        ..Default::default()
    };

    // Initialze keyboard stuffs
    // Initialize the storage and keymap
    let mut default_keymap = keymap::get_default_keymap();
    let mut behavior_config = BehaviorConfig::default();
    let mut per_key_config = PositionalConfig::default();
    let (keymap, mut storage) = initialize_keymap_and_storage(
        &mut default_keymap,
        flash,
        &storage_config,
        &mut behavior_config,
        &mut per_key_config,
    )
    .await;

    // Initialize the matrix and keyboard
    let debouncer = DefaultDebouncer::new();
    const OFFSET_ROWS: usize = TOT_ROW - CENTRAL_ROWS;
    const OFFSET_COLS: usize = TOT_COL - CENTRAL_COLS;
    // Suppose that the central matrix is col2row
    let mut matrix = OffsetMatrixWrapper::<
        _,
        _,
        _,
        0, // ROW OFFSET
        0, // COL OFFSET
    >(Matrix::<
        _,
        _,
        _,
        CENTRAL_ROWS, // ROW
        CENTRAL_COLS, // COL
        true,         // COL2ROW = true, set it to false to use ROW2COL matrix
    >::new(row_pins, col_pins, debouncer));

    let mut keyboard = Keyboard::new(&keymap); // Initialize the light controller


    let peripheral_addrs =
        read_peripheral_addresses::<1, _, TOT_ROW, TOT_COL, NUM_LAYER, _>(&mut storage).await;

    join5(
        run_devices! (
            (matrix) => EVENT_CHANNEL,
        ),
        keyboard.run(),
        run_rmk(&keymap, usb_driver, &stack, &mut storage, rmk_config),
        run_peripheral_manager::<PERIPHERAL_ROWS, PERIPHERAL_COLS, OFFSET_ROWS, OFFSET_COLS, _>(
            0,
            &peripheral_addrs,
            &stack,
        ),
        scan_peripherals(&stack, &peripheral_addrs),
    )
    .await;
}
