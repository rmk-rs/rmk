#![no_main]
#![no_std]

#[macro_use]
mod macros;

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_futures::join::join3;
use embassy_rp::gpio::{Input, Level, Output};
use embassy_rp::peripherals::{UART0, USB};
use embassy_rp::uart::{self, BufferedUart};
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_rp::{bind_interrupts, dma};
use panic_probe as _;
use rmk::config::DeviceConfig;
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::matrix::Matrix;
use rmk::processor::builtin::dfu_led::DfuLedProcessor;
use rmk::run_all;
use rmk::split::SPLIT_MESSAGE_MAX_SIZE;
use rmk::split::peripheral::run_rmk_split_peripheral;
use rmk::storage::{async_flash_wrapper, new_storage_without_keymap};
use rmk::watchdog::Rp2040Watchdog;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
    UART0_IRQ => uart::BufferedInterruptHandler<UART0>;
    DMA_IRQ_0 => dma::InterruptHandler<embassy_rp::peripherals::DMA_CH0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("RMK peripheral start!");
    let p = embassy_rp::init(Default::default());

    let (row_pins, col_pins) = config_matrix_pins_rp!(peripherals: p, input: [PIN_9, PIN_11], output: [PIN_10, PIN_12]);

    let flash = async_flash_wrapper(rmk::dfu::init_flash_from_linkerscript(p.FLASH));

    rmk::dfu::mark_booted();

    // DFU USB device so the peripheral can be firmware-updated via USB
    let dfu_driver = Driver::new(p.USB, Irqs);

    let dfu_device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x4643,
        manufacturer: "Haobo",
        product_name: "RMK Keyboard Peripheral",
        serial_number: "vial:f64c2b3c:000001",
    };

    let mut dfu_led = DfuLedProcessor::new(Output::new(p.PIN_25, Level::Low), false);

    static TX_BUF: StaticCell<[u8; SPLIT_MESSAGE_MAX_SIZE]> = StaticCell::new();
    let tx_buf = &mut TX_BUF.init([0; SPLIT_MESSAGE_MAX_SIZE])[..];
    static RX_BUF: StaticCell<[u8; SPLIT_MESSAGE_MAX_SIZE]> = StaticCell::new();
    let rx_buf = &mut RX_BUF.init([0; SPLIT_MESSAGE_MAX_SIZE])[..];
    let uart_instance = BufferedUart::new(p.UART0, p.PIN_0, p.PIN_1, Irqs, tx_buf, rx_buf, uart::Config::default());

    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, 2, 2, true>::new(row_pins, col_pins, debouncer);

    let storage_config = rmk::config::StorageConfig {
        num_sectors: 8,
        start_addr: 0,
        clear_storage: false,
        clear_layout: false,
    };
    let mut storage = new_storage_without_keymap(flash, storage_config).await;

    let mut watchdog_runner = Rp2040Watchdog::default_runner(embassy_rp::watchdog::Watchdog::new(p.WATCHDOG));

    join3(
        run_all!(matrix, storage, dfu_led, watchdog_runner),
        run_rmk_split_peripheral(uart_instance),
        rmk::usb::run_peripheral_usb(dfu_driver, dfu_device_config),
    )
    .await;
}
