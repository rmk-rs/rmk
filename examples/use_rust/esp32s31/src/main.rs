#![no_std]
#![no_main]

mod keymap;
#[macro_use]
mod macros;
mod vial;

use core::ptr::addr_of_mut;

use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::timer::timg::TimerGroup;
use esp_hal::usb::otg::Usb;
use esp_hal::usb::otg::embassy_usb_device::{Config, Driver};
use esp_storage::FlashStorage;
use rmk::config::{BehaviorConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::host::HostService;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::processor::builtin::wpm::WpmProcessor;
use rmk::storage::async_flash_wrapper;
use rmk::usb::UsbTransport;
use rmk::{KeymapData, initialize_keymap_and_storage, run_all};

use crate::keymap::*;
use crate::vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};

::esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_s: Spawner) {
    // Initialize the peripherals
    esp_println::logger::init_logger_from_env();
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    // Initialize USB. The high-speed OTG port has dedicated pins, so it takes no GPIOs.
    static mut EP_MEMORY: [u8; 1024] = [0; 1024];
    let usb = Usb::new_hs(peripherals.USB_HS);
    // Create the driver, from the HAL.
    let config = Config::default();
    let usb_driver = Driver::new(usb, unsafe { &mut *addr_of_mut!(EP_MEMORY) }, config);

    // Initialize the flash. ESP32-S31 is dual-core: park the other core during flash writes.
    let flash = FlashStorage::new(peripherals.FLASH).multicore_auto_park();
    let flash = async_flash_wrapper(flash);

    // Initialize the IO pins
    let (row_pins, col_pins) = config_matrix_pins_esp!(peripherals: peripherals, input: [GPIO2, GPIO3, GPIO4, GPIO5], output: [GPIO6, GPIO7, GPIO8]);

    // RMK config
    let vial_config = VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, &[(0, 0), (1, 1)]);
    let storage_config = StorageConfig {
        start_addr: 0x3f0000,
        num_sectors: 16,
        ..Default::default()
    };
    let rmk_config = RmkConfig {
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
        flash,
        &storage_config,
        &mut behavior_config,
        &per_key_config,
    )
    .await;

    // Initialize the matrix and keyboard
    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, ROW, COL, true>::new(row_pins, col_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);
    let host_service = HostService::new(&keymap, &rmk_config);

    let mut usb_transport = UsbTransport::new(usb_driver, rmk_config.device_config).with_host_service(&host_service);
    let mut wpm_processor = WpmProcessor::new();

    run_all!(matrix, storage, usb_transport, wpm_processor, keyboard).await;
}
