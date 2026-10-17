use core::cell::RefCell;
use core::future::{Future, poll_fn};
use core::marker::PhantomData;
use core::task::Poll;

use embassy_hal_internal::atomic_ring_buffer::RingBuffer;
use embassy_rp::Peri;
use embassy_rp::clocks::clk_sys_freq;
use embassy_rp::gpio::{Drive, Level, Pull, SlewRate};
use embassy_rp::interrupt::Priority;
use embassy_rp::interrupt::typelevel::{Binding, Handler, Interrupt};
use embassy_rp::pac::io::vals::Oeover;
use embassy_rp::pio::{
    Common, Config, Direction, FifoJoin, Instance, InterruptHandler, Pin, Pio, PioPin, ShiftDirection, StateMachine,
};
use embassy_rp::uart::Error;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::waitqueue::AtomicWaker;
use embassy_time::{Duration, Timer};
use embedded_io_async::{ErrorType, Read, Write};
use fixed::traits::ToFixed;

#[derive(Clone, Copy)]
pub struct IrqBinding;
unsafe impl<PIO: Instance> Binding<PIO::Interrupt, InterruptHandler<PIO>> for IrqBinding {}

const BAUD_RATE: u32 = 115_200;

mod StatusBit {
    pub const SM1_RX: u32 = 1 << 1;
    pub const SM0_TX: u32 = 1 << 4;
    pub const SM_IRQ0: u32 = 1 << 8;
    pub const SM_IRQ1: u32 = 1 << 9;
    pub const SM_IRQ2: u32 = 1 << 10;
}

mod IrqBit {
    pub const IRQ0: u8 = 1 << 0;
    pub const IRQ1: u8 = 1 << 1;
    pub const IRQ2: u8 = 1 << 2;
}

pub struct UartBuffer {
    buf_tx: RingBuffer,
    buf_rx: RingBuffer,
    waker_rx: AtomicWaker,
    waker_tx: AtomicWaker,
    idle_line: Mutex<CriticalSectionRawMutex, RefCell<bool>>,
}

impl UartBuffer {
    pub const fn new() -> Self {
        Self {
            buf_rx: RingBuffer::new(),
            buf_tx: RingBuffer::new(),
            waker_rx: AtomicWaker::new(),
            waker_tx: AtomicWaker::new(),
            idle_line: Mutex::new(RefCell::new(true)),
        }
    }
}

pub trait UartPioAccess {
    fn uart_buffer() -> &'static UartBuffer;
    fn regs() -> &'static embassy_rp::pac::pio::Pio;
}

macro_rules! impl_pio_access {
    ($ty:ty, $pio:path) => {
        impl UartPioAccess for $ty {
            fn uart_buffer() -> &'static UartBuffer {
                static BUFFER: UartBuffer = UartBuffer::new();
                &BUFFER
            }
            fn regs() -> &'static embassy_rp::pac::pio::Pio {
                &$pio
            }
        }
    };
}

impl_pio_access!(embassy_rp::peripherals::PIO0, embassy_rp::pac::PIO0);
impl_pio_access!(embassy_rp::peripherals::PIO1, embassy_rp::pac::PIO1);

// PIO Buffered UART serial driver
pub struct BufferedUart<'a, PIO: Instance + UartPioAccess> {
    full_duplex: bool,
    pin_rx: Pin<'a, PIO>,
    pin_tx: Option<Pin<'a, PIO>>,
    common: Common<'a, PIO>,
    sm_tx: StateMachine<'a, PIO, 0>,
    sm_rx: StateMachine<'a, PIO, 1>,
}

impl<'a, PIO: Instance + UartPioAccess> BufferedUart<'a, PIO> {
    /// Create a new half-duplex 'BufferedUart' serial driver instance
    ///
    /// # Arguments
    ///
    /// * `pio` - Programmable IO (PIO) block peripheral
    /// * `pin` - RX/TX pin
    /// * `rx_buf` - RX buffer
    /// * `irq` - Interrupt handler binding
    ///
    /// # Returns
    ///
    /// A new instance of 'BufferedUart' driver
    pub fn new_half_duplex<T: PioPin>(
        pio: Peri<'a, PIO>,
        pin: Peri<'a, T>,
        rx_buf: &mut [u8],
        irq: impl Binding<PIO::Interrupt, UartInterruptHandler<PIO>>,
    ) -> Self {
        Self::new(pio, pin, None::<Peri<'a, T>>, rx_buf, None, false, irq)
    }

    /// Create a new full-duplex 'BufferedUart' serial driver instance
    ///
    /// # Arguments
    ///
    /// * `pio` - Programmable IO (PIO) block peripheral
    /// * `pin_tx` - TX pin
    /// * `pin_rx` - RX pin
    /// * `tx_buf` - TX buffer
    /// * `rx_buf` - RX buffer
    /// * `irq` - Interrupt handler binding
    ///
    /// # Returns
    ///
    /// A new instance of 'BufferedUart' driver
    pub fn new_full_duplex(
        pio: Peri<'a, PIO>,
        pin_tx: Peri<'a, impl PioPin>,
        pin_rx: Peri<'a, impl PioPin>,
        tx_buf: &mut [u8],
        rx_buf: &mut [u8],
        irq: impl Binding<PIO::Interrupt, UartInterruptHandler<PIO>>,
    ) -> Self {
        Self::new(pio, pin_tx, Some(pin_rx), rx_buf, Some(tx_buf), true, irq)
    }

    fn new(
        pio: Peri<'a, PIO>,
        pin_rx: Peri<'a, impl PioPin>,
        pin_tx: Option<Peri<'a, impl PioPin>>,
        rx_buf: &mut [u8],
        tx_buf: Option<&mut [u8]>,
        full_duplex: bool,
        _irq: impl Binding<PIO::Interrupt, UartInterruptHandler<PIO>>,
    ) -> Self {
        let Pio {
            mut common,
            sm0: sm_tx,
            sm1: sm_rx,
            ..
        } = Pio::new(pio, IrqBinding);

        let pio_pin_rx = common.make_pio_pin(pin_rx);
        let pio_pin_tx = pin_tx.map(|pin| common.make_pio_pin(pin));

        let buffer = PIO::uart_buffer();
        unsafe { buffer.buf_rx.init(rx_buf.as_mut_ptr(), rx_buf.len()) };
        if let Some(buf) = tx_buf {
            unsafe { buffer.buf_tx.init(buf.as_mut_ptr(), buf.len()) };
        }

        let mut uart = Self {
            full_duplex,
            pin_rx: pio_pin_rx,
            pin_tx: pio_pin_tx,
            common,
            sm_tx,
            sm_rx,
        };

        uart.setup_interrupts();
        uart.setup_pins();
        uart.setup_sm_tx();
        uart.setup_sm_rx();

        uart
    }

    fn setup_interrupts(&self) {
        PIO::Interrupt::disable();
        PIO::Interrupt::set_priority(Priority::P0);
        PIO::regs().irqs(0).inte().write(|i| {
            i.set_sm0(true);
            i.set_sm1(!self.full_duplex);
            i.set_sm2(!self.full_duplex);
            i.set_sm1_rxnempty(true);
        });
        PIO::Interrupt::unpend();
        unsafe { PIO::Interrupt::enable() };
    }

    fn setup_pins(&mut self) {
        let pins = [Some(&mut self.pin_rx), self.pin_tx.as_mut()];

        for pin in pins.into_iter().flatten() {
            embassy_rp::pac::IO_BANK0
                .gpio(pin.pin() as _)
                .ctrl()
                // `from_bits(1)` = the INVERT/Invert variant; compiles against both
                // registry and git rp-pac (which renamed the variant casing).
                .modify(|f| f.set_oeover(Oeover::from_bits(1)));
            pin.set_schmitt(true);
            pin.set_pull(Pull::Up);
            pin.set_slew_rate(SlewRate::Fast);
        }
    }

    fn setup_sm_tx(&mut self) {
        let prg = pio::pio_asm!(
            ".side_set 1 opt pindirs",
            ".wrap_target",
            "pull   block           side 1 [7]",
            "set    x, 7            side 0 [7]",
            "out    pindirs, 1",
            "jmp    x--, 2                 [6]"
            ".wrap",
        );

        let pin_tx = self.pin_tx.as_ref().unwrap_or(&self.pin_rx);

        let mut cfg = Config::default();
        cfg.use_program(&self.common.load_program(&prg.program), &[pin_tx]);
        cfg.set_out_pins(&[pin_tx]);
        let div = clk_sys_freq() / (BAUD_RATE as u32 * 8u32);
        cfg.clock_divider = div.to_fixed();
        cfg.shift_out.auto_fill = false;
        cfg.shift_out.direction = ShiftDirection::Right;
        cfg.shift_out.threshold = 32;
        cfg.fifo_join = FifoJoin::TxOnly;
        self.sm_tx.set_config(&cfg);

        if self.full_duplex {
            self.set_pin_tx();
            self.sm_tx.set_enable(true);
        }
    }

    fn setup_sm_rx(&mut self) {
        let prg = pio::pio_asm!(
            ".wrap_target",
            "wait_idle:",
            "    wait 0 pin, 0",
            "    irq  1                         [2]",
            "start:"
            "    set  x, 7",
            "    set  y, 10                     [6]",
            "read_loop:",
            "    in   pins, 1",
            "    jmp  x-- read_loop             [6]",
            "    jmp  pin good_stop",
            "    irq  wait 0",
            "    wait 1 pin, 0",
            "    jmp  check_idle",
            "good_stop:",
            "    push block",
            "check_idle:",
            "    jmp  pin check_idle_continue",
            "    jmp  start"
            "check_idle_continue:"
            "    jmp  y-- check_idle"
            "    irq  2",
            "    jmp  wait_idle",
            ".wrap"
        );

        let mut cfg = Config::default();
        cfg.use_program(&self.common.load_program(&prg.program), &[]);
        cfg.set_in_pins(&[&self.pin_rx]);
        cfg.set_jmp_pin(&self.pin_rx);
        let div = clk_sys_freq() / (BAUD_RATE as u32 * 8u32);
        cfg.clock_divider = div.to_fixed();
        cfg.shift_in.auto_fill = false;
        cfg.shift_in.direction = ShiftDirection::Right;
        cfg.shift_in.threshold = 32;
        cfg.fifo_join = FifoJoin::RxOnly;
        self.sm_rx.set_config(&cfg);

        self.set_pin_rx();
        self.sm_rx.set_enable(true);
    }

    async fn enable_sm_tx(&mut self) {
        while !PIO::uart_buffer().idle_line.lock(|b| *b.borrow()) {
            Timer::after(Duration::from_micros(((1_000_000u32 * 1) / BAUD_RATE) as u64)).await;
        }
        self.sm_rx.set_enable(false);
        self.set_pin_tx();
        self.sm_tx.restart();
        self.sm_tx.set_enable(true);
    }

    async fn enable_sm_rx(&mut self) {
        while !self.sm_tx.tx().empty() {}
        Timer::after(Duration::from_micros(((1_000_000u32 * 11) / BAUD_RATE) as u64)).await;
        self.sm_tx.set_enable(false);

        self.set_pin_rx();

        PIO::uart_buffer().idle_line.lock(|b| *b.borrow_mut() = true);
        self.sm_rx.set_enable(true);
    }

    fn set_pin_tx(&mut self) {
        self.sm_tx.set_pin_dirs(Direction::Out, &[&self.pin_rx]);
        // OEOVER set to INVERT, Direction::Out inverted to Direction:In
        self.sm_tx.set_pins(Level::Low, &[&self.pin_rx]);

        let pin_tx = self.pin_tx.as_mut().unwrap_or(&mut self.pin_rx);
        // unset our fake-pull-up trickery
        pin_tx.set_drive_strength(Drive::_12mA);
    }

    fn set_pin_rx(&mut self) {
        // The rp2040 has weak pull up resistors, from 80k to 50k. This does not provide enough
        // current to provide fast rise times at high baud rates with any moderately high
        // capacitance, even as little capacitance as can be found with long traces, a few vias, or
        // a longer TRRS cable. The solution is to also drive the line high at a weak drive current
        // from the reciving side, providing plenty of current to drive the line high quickly while
        // still being weak enough to be driven low from the tx side.
        self.pin_rx.set_drive_strength(Drive::_2mA);

        // OEOVER set to INVERT, Direction::In inverted to Direction:Out
        self.sm_rx.set_pins(Level::High, &[&self.pin_rx]);
        self.sm_rx.set_pin_dirs(Direction::In, &[&self.pin_rx]);
    }

    fn read_buffer<'c>(&'c self, buf: &'c mut [u8]) -> impl Future<Output = Result<usize, Error>> + 'c {
        poll_fn(move |cx| {
            if let Poll::Ready(r) = self.try_read(buf) {
                return Poll::Ready(r);
            }
            PIO::uart_buffer().waker_rx.register(cx.waker());
            Poll::Pending
        })
    }

    fn try_read(&self, buf: &mut [u8]) -> Poll<Result<usize, Error>> {
        if buf.len() == 0 {
            return Poll::Ready(Ok(0));
        }
        self.read_ring(buf)
    }

    fn read_ring(&self, buf: &mut [u8]) -> Poll<Result<usize, Error>> {
        let mut reader = unsafe { PIO::uart_buffer().buf_rx.reader() };
        let data = reader.pop_slice();
        if data.len() == 0 {
            return Poll::Pending;
        };
        let n = data.len().min(buf.len());
        buf[..n].copy_from_slice(&data[..n]);
        reader.pop_done(n);
        Poll::Ready(Ok(n))
    }

    async fn write_buffer(&mut self, buf: &[u8]) -> Result<usize, Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.full_duplex {
            return poll_fn(|cx| {
                PIO::uart_buffer().waker_tx.register(cx.waker());
                let result = self.write_ring(buf);
                PIO::regs().irqs(0).inte().modify(|i| i.set_sm0_txnfull(true));
                result
            })
            .await;
        } else {
            if !self.sm_tx.is_enabled() {
                self.enable_sm_tx().await;
            }
            let result = self.write_fifo(buf).await;
            self.enable_sm_rx().await;
            return result;
        }
    }

    fn write_ring(&self, buf: &[u8]) -> Poll<Result<usize, Error>> {
        let mut writer = unsafe { PIO::uart_buffer().buf_tx.writer() };
        let data = writer.push_slice();
        if data.is_empty() {
            return Poll::Pending;
        }
        let n = data.len().min(buf.len());
        data[..n].copy_from_slice(&buf[..n]);
        writer.push_done(n);
        Poll::Ready(Ok(n))
    }

    async fn write_fifo(&mut self, buf: &[u8]) -> Result<usize, Error> {
        for byte in buf {
            self.wait_push(*byte as u32).await;
        }
        Ok(buf.len())
    }

    fn wait_push<'b>(&'b mut self, byte: u32) -> impl Future<Output = ()> + 'b + use<'b, 'a, PIO> {
        poll_fn(move |cx| {
            if self.sm_tx.tx().try_push(byte) {
                return Poll::Ready(());
            }
            PIO::regs().irqs(0).inte().modify(|i| i.set_sm0_txnfull(true));
            PIO::uart_buffer().waker_tx.register(cx.waker());
            Poll::Pending
        })
    }

    async fn flush(&mut self) -> Result<(), Error> {
        if !self.sm_tx.tx().empty() {
            while !self.sm_tx.tx().empty() {}
            Timer::after(Duration::from_micros(((1_000_000u32 * 11) / BAUD_RATE) as u64)).await;
        }
        Ok(())
    }
}

pub struct UartInterruptHandler<PIO: Instance + UartPioAccess> {
    _pio: PhantomData<PIO>,
}

impl<PIO: Instance + UartPioAccess> Handler<PIO::Interrupt> for UartInterruptHandler<PIO> {
    unsafe fn on_interrupt() {
        let pio = PIO::regs();
        let ints = PIO::regs().irqs(0).ints().read().0;

        if PIO::uart_buffer().buf_rx.is_available() {
            if ints & StatusBit::SM1_RX != 0 {
                let mut writer = unsafe { PIO::uart_buffer().buf_rx.writer() };
                let rx_buf = writer.push_slice();
                if rx_buf.len() > 0 {
                    // SM1 RX FIFO is joined (FifoJoin::RxOnly), so depth is 8.
                    let mut n = 0;
                    while n < rx_buf.len() {
                        let avail = pio.flevel().read().rx1() as usize;
                        if avail == 0 {
                            break;
                        }
                        let take = avail.min(rx_buf.len() - n);
                        for slot in &mut rx_buf[n..n + take] {
                            *slot = (pio.rxf(1).read() >> 24) as u8;
                        }
                        n += take;
                    }
                    writer.push_done(n);
                    PIO::Interrupt::unpend();
                    PIO::uart_buffer().waker_rx.wake();
                }
            }
        }
        if ints & StatusBit::SM0_TX != 0 {
            // TX_SM FIFO Not Full
            PIO::Interrupt::unpend();
            if PIO::uart_buffer().buf_tx.is_available() {
                // Full-Duplex Mode
                let mut reader = unsafe { PIO::uart_buffer().buf_tx.reader() };
                let tx_buf = reader.pop_slice();
                // SM0 TX FIFO is joined (FifoJoin::TxOnly), so depth is 8.
                const TX_FIFO_DEPTH: usize = 8;
                let mut n = 0;
                while n < tx_buf.len() {
                    let free = TX_FIFO_DEPTH - pio.flevel().read().tx0() as usize;
                    if free == 0 {
                        break;
                    }
                    let push = free.min(tx_buf.len() - n);
                    for byte in &tx_buf[n..n + push] {
                        pio.txf(0).write(|f| *f = *byte as u32);
                    }
                    n += push;
                }
                reader.pop_done(n);
                PIO::uart_buffer().waker_tx.wake();
                if PIO::uart_buffer().buf_tx.is_empty() {
                    pio.irqs(0).inte().modify(|i| i.set_sm0_txnfull(false));
                }
            } else {
                // Half-Duplex Mode
                pio.irqs(0).inte().modify(|i| i.set_sm0_txnfull(false));
                PIO::uart_buffer().waker_tx.wake();
            }
        }
        if ints & StatusBit::SM_IRQ0 != 0 {
            // RX_SM Invalid Stop Bit Raised IRQ 0
            pio.irq().write(|f| f.set_irq(IrqBit::IRQ0));
            PIO::Interrupt::unpend();
        }
        if ints & StatusBit::SM_IRQ1 != 0 {
            // Line Non-Idle Toogle Raised IRQ 1
            PIO::uart_buffer().idle_line.lock(|b| *b.borrow_mut() = false);
            pio.irq().write(|f| f.set_irq(IrqBit::IRQ1));
            PIO::Interrupt::unpend();
        }
        if ints & StatusBit::SM_IRQ2 != 0 {
            // Line Idle Toogle Raised IRQ 2
            PIO::uart_buffer().idle_line.lock(|b| *b.borrow_mut() = true);
            pio.irq().write(|f| f.set_irq(IrqBit::IRQ2));
            PIO::Interrupt::unpend();
        }
    }
}

impl<'a, PIO: Instance + UartPioAccess> Read for BufferedUart<'a, PIO> {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
        self.read_buffer(buf).await.map_err(|e| e.into())
    }
}

impl<'a, PIO: Instance + UartPioAccess> Write for BufferedUart<'a, PIO> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        self.write_buffer(buf).await.map_err(|e| e.into())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.flush().await.map_err(|e| e.into())
    }
}

#[derive(Debug)]
// TODO: Remove NewError after embassy-rp updates to embedded-io-async 0.7
pub struct NewError(Error);

impl core::fmt::Display for NewError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl core::error::Error for NewError {}

impl embedded_io_async::Error for NewError {
    fn kind(&self) -> embedded_io_async::ErrorKind {
        embedded_io_async::ErrorKind::Other
    }
}

impl From<embassy_rp::uart::Error> for NewError {
    fn from(value: embassy_rp::uart::Error) -> Self {
        Self(value)
    }
}

impl<'a, PIO: Instance + UartPioAccess> ErrorType for BufferedUart<'a, PIO> {
    type Error = NewError;
}
