//! 74HC595 Shift Register Matrix
//!
//! A keyboard matrix implementation that uses SPI-connected 74HC595 shift
//! registers as column drivers. Row pins are regular GPIO inputs.
//!
//! The 595 chain length is derived from `COL`: one 595 provides 8 outputs,
//! so two daisy-chained 595s handle up to 16 columns, etc.
//!
//! # Wiring
//!
//! ```text
//!   MCU                   74HC595 (xN)          Matrix
//!   ---------------------------------------------------
//!   SPI MOSI  -------->  SER (data in)
//!   SPI SCK   -------->  SRCLK (shift clock)
//!   GPIO Out  -------->  RCLK  (storage latch)  COL 0..N
//!   GPIO In   <------------------------------  ROW 0..M
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use rmk::matrix::hc595_matrix::Hc595Matrix;
//!
//! let mut matrix = Hc595Matrix::<_, _, _, _, 2, 16>::new(
//!     spi_device,   // impl SpiDevice<u8>
//!     latch_pin,    // impl OutputPin (RCLK)
//!     row_pins,     // [impl InputPin; ROW]
//!     debouncer,
//! )
//! .await;
//! ```

use embassy_time::Timer;
use embedded_hal::digital::{InputPin, OutputPin};
#[cfg(feature = "async_matrix")]
use embedded_hal_async::digital::Wait;
use embedded_hal_async::spi::SpiDevice;
use rmk_macro::input_device;

use crate::debounce::{DebounceState, DebouncerTrait};
use crate::event::KeyboardEvent;
use crate::matrix::{KeyState, MatrixTrait};

const SR_MAX_BYTES: usize = 4;
const SR_CLEAR_SETTLE_US: u64 = 3;
const SR_COLUMN_SETTLE_US: u64 = 40;

#[input_device(publish = KeyboardEvent)]
pub struct Hc595Matrix<
    SPI: SpiDevice<u8>,
    LATCH: OutputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize = 0,
    const COL_OFFSET: usize = 0,
> {
    spi: SPI,
    latch: LATCH,
    row_pins: [In; ROW],
    debouncer: D,
    key_states: [[KeyState; ROW]; COL],
    scan_pos: (usize, usize),
    #[cfg(feature = "async_matrix")]
    rescan_needed: bool,
}

impl<
    SPI: SpiDevice<u8>,
    LATCH: OutputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> Hc595Matrix<SPI, LATCH, In, D, ROW, COL, ROW_OFFSET, COL_OFFSET>
{
    const NUM_BYTES: usize = {
        if COL > SR_MAX_BYTES * 8 {
            panic!("Hc595Matrix supports up to 32 columns");
        }
        COL.div_ceil(8)
    };

    pub async fn new(spi: SPI, latch: LATCH, row_pins: [In; ROW], debouncer: D) -> Self {
        let mut matrix = Self {
            spi,
            latch,
            row_pins,
            debouncer,
            key_states: [[KeyState::new(); ROW]; COL],
            scan_pos: (0, 0),
            #[cfg(feature = "async_matrix")]
            rescan_needed: false,
        };
        matrix.clear_columns().await;
        matrix
    }

    async fn pulse_latch(&mut self, data: &[u8]) {
        self.latch.set_low().ok();
        let _ = self.spi.write(data).await;
        self.latch.set_high().ok();
    }

    fn col_bitmask(col_idx: usize) -> [u8; SR_MAX_BYTES] {
        let mut buf = [0u8; SR_MAX_BYTES];
        let byte_pos = col_idx / 8;
        let bit_pos = col_idx % 8;
        buf[Self::NUM_BYTES - 1 - byte_pos] = 1 << bit_pos;
        buf
    }

    async fn clear_columns(&mut self) {
        let zeros = [0u8; SR_MAX_BYTES];
        self.pulse_latch(&zeros[..Self::NUM_BYTES]).await;
    }

    async fn read_keyboard_event(&mut self) -> KeyboardEvent {
        loop {
            let (col_start, row_start) = self.scan_pos;
            #[cfg(not(feature = "async_matrix"))]
            let mut any_active = false;

            for col_idx in col_start..COL {
                self.clear_columns().await;
                Timer::after_micros(SR_CLEAR_SETTLE_US).await;

                let bitmask = Self::col_bitmask(col_idx);
                self.pulse_latch(&bitmask[..Self::NUM_BYTES]).await;
                Timer::after_micros(SR_COLUMN_SETTLE_US).await;

                let r_start = if col_idx == col_start { row_start } else { 0 };

                for row_idx in r_start..ROW {
                    let pin_high = self.row_pins[row_idx].is_high().ok().unwrap_or(false);

                    let debounce_state = self.debouncer.detect_change_with_debounce(
                        row_idx,
                        col_idx,
                        pin_high,
                        &self.key_states[col_idx][row_idx],
                    );

                    if let DebounceState::Debounced = debounce_state {
                        self.key_states[col_idx][row_idx].toggle_pressed();
                        self.scan_pos = (col_idx, row_idx);
                        #[cfg(feature = "async_matrix")]
                        {
                            self.rescan_needed = true;
                        }
                        self.clear_columns().await;
                        return KeyboardEvent::key(
                            (row_idx + ROW_OFFSET) as u8,
                            (col_idx + COL_OFFSET) as u8,
                            self.key_states[col_idx][row_idx].pressed,
                        );
                    }

                    #[cfg(feature = "async_matrix")]
                    if self.key_states[col_idx][row_idx].pressed {
                        self.rescan_needed = true;
                    }

                    // Keep scanning at full rate while a key is held or bouncing.
                    #[cfg(not(feature = "async_matrix"))]
                    if self.key_states[col_idx][row_idx].pressed || matches!(debounce_state, DebounceState::InProgress)
                    {
                        any_active = true;
                    }
                }
            }

            self.clear_columns().await;

            #[cfg(feature = "async_matrix")]
            {
                if !self.rescan_needed {
                    self.wait_for_key().await;
                }
                self.rescan_needed = false;
                Timer::after_millis(1).await;
            }

            // Polling builds have no interrupt gate: keep the 1ms cadence while any
            // key is active, but idle-scan otherwise so the CPU can sleep between
            // passes.
            #[cfg(not(feature = "async_matrix"))]
            if any_active {
                Timer::after_millis(1).await;
            } else {
                Timer::after_millis(crate::MATRIX_IDLE_SCAN_MS.into()).await;
            }

            self.scan_pos = (0, 0);
        }
    }
}

impl<
    SPI: SpiDevice<u8>,
    LATCH: OutputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> MatrixTrait<ROW, COL> for Hc595Matrix<SPI, LATCH, In, D, ROW, COL, ROW_OFFSET, COL_OFFSET>
{
    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {
        self.clear_columns().await;
        Timer::after_millis(1).await;
    }
}
