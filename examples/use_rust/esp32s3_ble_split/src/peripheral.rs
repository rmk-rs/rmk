#![no_std]
#![no_main]

mod vial;
#[macro_use]
mod macros;
mod keymap;

use bt_hci::controller::ExternalController;
use embassy_executor::Spawner;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::rng::TrngSource;
use esp_radio::Controller;
use esp_radio::ble::controller::BleConnector;
use esp_storage::FlashStorage;

use keymap::{PERIPHERAL_COLS, PERIPHERAL_ROWS};
use rmk::ble::build_ble_stack;
use rmk::channel::EVENT_CHANNEL;
use rmk::config::StorageConfig;
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::futures::future::join;
use rmk::matrix::Matrix;
use rmk::split::peripheral::run_rmk_split_peripheral;
use rmk::storage::async_flash_wrapper;
use rmk::storage::new_storage_for_split_peripheral;
use rmk::{HostResources, run_devices};
use static_cell::StaticCell;
use {esp_alloc as _, esp_backtrace as _};
use {esp_alloc as _, esp_backtrace as _};

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
    let peripheral_addr = [0x7e, 0xfe, 0x73, 0x9e, 0x66, 0xe3];
    let mut host_resources = HostResources::new();
    let stack = build_ble_stack(controller, peripheral_addr, &mut rng, &mut host_resources).await;

    // Initialize the flash
    let flash = FlashStorage::new(peripherals.FLASH);
    let flash = async_flash_wrapper(flash);

    // Initialize the IO pins
    let (row_pins, col_pins) = config_matrix_pins_esp!(peripherals: peripherals, input: [GPIO6, GPIO5, GPIO4, GPIO3, GPIO2, GPIO1], output: [GPIO7, GPIO8, GPIO9, GPIO10, GPIO11, GPIO12, GPIO13]);

    // RMK config
    let storage_config = StorageConfig {
        start_addr: 0x3f0000,
        num_sectors: 16,
        ..Default::default()
    };

    // Initialze keyboard stuffs
    // Initialize the storage
    let mut storage = new_storage_for_split_peripheral(flash, storage_config).await;

    // Initialize the matrix and keyboard
    let debouncer = DefaultDebouncer::new();

    // Suppose that the central matrix is col2row

    let mut matrix = Matrix::<_, _, _, PERIPHERAL_ROWS, PERIPHERAL_COLS, true>::new(
        row_pins, col_pins, debouncer,
    );

    join(
        run_devices! (
            (matrix) => EVENT_CHANNEL,
        ),
        run_rmk_split_peripheral(0, &stack, &mut storage),
    )
    .await;
}
