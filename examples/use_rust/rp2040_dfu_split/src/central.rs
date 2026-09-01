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
use embassy_rp::gpio::{Input, Level, Output};
use embassy_rp::peripherals::{UART0, USB};
use embassy_rp::uart::{self, BufferedUart};
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_rp::{bind_interrupts, dma};
use panic_probe as _;
use rmk::config::{BehaviorConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::futures::future::join;
use rmk::host::HostService;
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::processor::builtin::dfu_led::DfuLedProcessor;
use rmk::processor::builtin::wpm::WpmProcessor;
use rmk::split::central::run_peripheral_manager;
use rmk::split::{PeripheralMatrixConfig, SPLIT_MESSAGE_MAX_SIZE};
use rmk::storage::async_flash_wrapper;
use rmk::usb::UsbTransport;
use rmk::watchdog::Rp2040Watchdog;
use rmk::{initialize_keymap_and_storage, run_all, KeymapData};
use static_cell::StaticCell;
use vial::{VIAL_KEYBOARD_DEF, VIAL_KEYBOARD_ID};

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
    UART0_IRQ => uart::BufferedInterruptHandler<UART0>;
    DMA_IRQ_0 => dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>;
});

const PERIPHERAL1_BIN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/rmk-rp2040-dfu-split-peripheral.bin"
));

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("RMK start!");
    let p = embassy_rp::init(Default::default());

    let driver = Driver::new(p.USB, Irqs);

    let (row_pins, col_pins) = config_matrix_pins_rp!(peripherals: p, input: [PIN_6, PIN_7], output: [PIN_19, PIN_20]);

    let flash = async_flash_wrapper(rmk::dfu::init_flash_from_linkerscript(p.FLASH));

    let keyboard_device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x4643,
        manufacturer: "Haobo",
        product_name: "RMK Keyboard RP2040 embassy-boot split use_rust example",
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

    // mark the firmware as booted otherwise the bootloader thinks it didn't and will revert to the old firmware
    rmk::dfu::mark_booted();

    // DFU LED processor, optional. Flashes the LED when DFU is active
    let mut dfu_led_processor = DfuLedProcessor::new(Output::new(p.PIN_25, Level::Low), false);

    // Register peripheral firmware for DFU update over split
    if rmk::dfu::set_firmware_update_data(0, PERIPHERAL1_BIN, rmk::crc32::crc32(PERIPHERAL1_BIN)).is_ok() {
        info!("registered peripheral firmware");
    }

    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, 2, 2, true>::new(row_pins, col_pins, debouncer);
    let mut keyboard = Keyboard::new(&keymap);
    let host_service = HostService::new(&keymap, &rmk_config);

    let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config).with_host_service(&host_service);
    let mut wpm_processor = WpmProcessor::new();

    let mut watchdog_runner = Rp2040Watchdog::default_runner(embassy_rp::watchdog::Watchdog::new(p.WATCHDOG));

    // UART for split peripheral communication
    static TX_BUF: StaticCell<[u8; SPLIT_MESSAGE_MAX_SIZE]> = StaticCell::new();
    let tx_buf = &mut TX_BUF.init([0; SPLIT_MESSAGE_MAX_SIZE])[..];
    static RX_BUF: StaticCell<[u8; SPLIT_MESSAGE_MAX_SIZE]> = StaticCell::new();
    let rx_buf = &mut RX_BUF.init([0; SPLIT_MESSAGE_MAX_SIZE])[..];
    let uart_receiver = BufferedUart::new(p.UART0, p.PIN_0, p.PIN_1, Irqs, tx_buf, rx_buf, uart::Config::default());

    join(
        run_all!(
            matrix,
            storage,
            usb_transport,
            wpm_processor,
            dfu_led_processor,
            keyboard,
            watchdog_runner
        ),
        // use UpdatePolicy::Force to force the peripheral update at every start of central
        run_peripheral_manager(
            0,
            uart_receiver,
            PeripheralMatrixConfig {
                rows: 2,
                cols: 1,
                row_offset: 2,
                col_offset: 2,
            },
            rmk::split::central::UpdatePolicy::MatchHash,
        ),
    )
    .await;
}
