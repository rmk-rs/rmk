#![no_main]
#![no_std]

#[macro_use]
mod macros;

use defmt::*;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Input, Level, Output};
use embassy_rp::peripherals::{UART0, USB};
use embassy_rp::uart::{self, BufferedUart};
use embassy_rp::usb::InterruptHandler;
use panic_probe as _;
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::futures::future::join;
use rmk::matrix::Matrix;
use rmk::processor::builtin::dfu_led::DfuLedProcessor;
use rmk::run_all;
use rmk::split::peripheral::run_rmk_split_peripheral;
use rmk::split::SPLIT_MESSAGE_MAX_SIZE;
use rmk::watchdog::Rp2040Watchdog;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
    UART0_IRQ => uart::BufferedInterruptHandler<UART0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    info!("RMK peripheral start!");
    let p = embassy_rp::init(Default::default());

    let (row_pins, col_pins) = config_matrix_pins_rp!(peripherals: p, input: [PIN_8, PIN_9], output: [PIN_10]);

    rmk::dfu::init_flash_from_linkerscript(p.FLASH);

    // mark the firmware as booted otherwise the bootloader thinks it didn't and will revert to the old firmware
    rmk::dfu::mark_booted();

    // DFU LED processor, optional. Flashes the LED when DFU is active
    let mut dfu_led_processor = DfuLedProcessor::new(Output::new(p.PIN_25, Level::Low), false);

    // UART for split peripheral communication
    static TX_BUF: StaticCell<[u8; SPLIT_MESSAGE_MAX_SIZE]> = StaticCell::new();
    let tx_buf = &mut TX_BUF.init([0; SPLIT_MESSAGE_MAX_SIZE])[..];
    static RX_BUF: StaticCell<[u8; SPLIT_MESSAGE_MAX_SIZE]> = StaticCell::new();
    let rx_buf = &mut RX_BUF.init([0; SPLIT_MESSAGE_MAX_SIZE])[..];
    let uart_instance = BufferedUart::new(p.UART0, p.PIN_0, p.PIN_1, Irqs, tx_buf, rx_buf, uart::Config::default());

    let debouncer = DefaultDebouncer::new();
    let mut matrix = Matrix::<_, _, _, 2, 1, true>::new(row_pins, col_pins, debouncer);

    let mut watchdog_runner = Rp2040Watchdog::default_runner(embassy_rp::watchdog::Watchdog::new(p.WATCHDOG));

    join(
        run_all!(matrix, dfu_led_processor, watchdog_runner),
        run_rmk_split_peripheral(uart_instance),
    )
    .await;
}
