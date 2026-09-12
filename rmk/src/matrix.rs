use embassy_time::Timer;
use embedded_hal::digital::{InputPin, OutputPin};
use rmk_macro::input_device;
#[cfg(feature = "async_matrix")]
use {embassy_futures::select::select_array, embedded_hal_async::digital::Wait};

use crate::core_traits::Runnable;
use crate::debounce::{DebounceState, DebouncerTrait};
use crate::event::{KeyboardEvent, publish_event_async};
use crate::input_device::InputDevice;
pub mod bidirectional_matrix;
pub mod direct_pin;
pub mod hc595_matrix;

/// Recording the matrix pressed state
#[cfg(feature = "host_lock")]
pub struct MatrixState {
    // 30 bytes is the limit by Vial and 240 keys is enough for most keyboards
    state: [u8; 30],
    row: usize,
    col: usize,
    row_len: usize,
}

#[cfg(feature = "host_lock")]
impl MatrixState {
    pub fn new(row: usize, col: usize) -> Self {
        let row_len = col.div_ceil(8);
        assert!(row * row_len <= 30, "Matrix too large for MatrixState");
        Self {
            state: [0; 30],
            row,
            col,
            row_len,
        }
    }
    pub fn update(&mut self, event: &KeyboardEvent) {
        use crate::event::{KeyPos, KeyboardEventPos};
        if let KeyboardEventPos::Key(KeyPos { row, col }) = event.pos {
            if row as usize >= self.row || col as usize >= self.col {
                warn!("Matrix read out of bounds");
                return;
            }
            let pressed = event.pressed;
            let index = row as usize * self.row_len * 8 + col as usize;
            let byte_index = index / 8;
            let bit_index = index % 8;
            self.state[byte_index] = self.state[byte_index] & !(1 << bit_index) | ((pressed as u8) << bit_index);
        }
    }
    /// Copy the bitmap out in natural row-major order (bit 0 = col 0,
    /// low byte first within a row) — the Rynk `MatrixState` wire order.
    pub fn read_all(&self, target: &mut [u8]) {
        let n = (self.row * self.row_len).min(target.len());
        target[..n].copy_from_slice(&self.state[..n]);
    }
    pub fn read(&self, row: u8, col: u8) -> bool {
        if row as usize >= self.row || col as usize >= self.col {
            warn!("Matrix read out of bounds");
            return false;
        }
        let index = row as usize * self.row_len * 8 + col as usize;
        let byte_index = index / 8;
        let bit_index = index % 8;
        self.state[byte_index] & (1 << bit_index) != 0
    }
}

/// MatrixTrait is the trait for keyboard matrix.
///
/// The keyboard matrix is a 2D matrix of keys, the matrix does the scanning and saves the result to each key's `KeyState`.
pub trait MatrixTrait<const ROW: usize, const COL: usize>: InputDevice {
    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self);
}

/// KeyState represents the state of a key.
#[derive(Copy, Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeyState {
    // True if the key is pressed
    pub pressed: bool,
    // True if the key's state is just changed
    // pub changed: bool,
}

impl Default for KeyState {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyState {
    pub fn new() -> Self {
        KeyState { pressed: false }
    }

    pub fn toggle_pressed(&mut self) {
        self.pressed = !self.pressed;
    }

    pub fn is_releasing(&self) -> bool {
        !self.pressed
    }

    pub fn is_pressing(&self) -> bool {
        self.pressed
    }
}

pub trait RowPins<const COL2ROW: bool> {
    type RowPinsType;
}
pub trait ColPins<const COL2ROW: bool> {
    type ColPinsType;
}

pub trait MatrixOutputPins<Out: OutputPin> {
    fn get_output_pins(&self) -> &[Out];
    fn get_output_pins_mut(&mut self) -> &mut [Out];
}

pub trait MatrixInputPins<In: InputPin> {
    fn get_input_pins(&self) -> &[In];
    fn get_input_pins_mut(&mut self) -> &mut [In];
    #[cfg(feature = "async_matrix")]
    async fn wait_input_pins(&mut self);
}

/// Matrix is the physical pcb layout of the keyboard matrix.
pub struct Matrix<
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const COL2ROW: bool,
    const ROW_OFFSET: usize = 0,
    const COL_OFFSET: usize = 0,
> where
    Self: RowPins<COL2ROW>,
    Self: ColPins<COL2ROW>,
{
    /// Row pins of the pcb matrix
    row_pins: <Self as RowPins<COL2ROW>>::RowPinsType,
    /// Column pins of the pcb matrix
    col_pins: <Self as ColPins<COL2ROW>>::ColPinsType,
    /// Debouncer
    debouncer: D,
    /// Key state matrix
    key_states: [[KeyState; ROW]; COL],
    /// Current scan pos: (out_idx, in_idx)
    scan_pos: (usize, usize),
    /// Re-scan needed flag
    #[cfg(feature = "async_matrix")]
    rescan_needed: bool,
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> RowPins<true> for Matrix<In, Out, D, ROW, COL, true, ROW_OFFSET, COL_OFFSET>
{
    type RowPinsType = [In; ROW];
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> RowPins<false> for Matrix<In, Out, D, ROW, COL, false, ROW_OFFSET, COL_OFFSET>
{
    type RowPinsType = [Out; ROW];
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> ColPins<false> for Matrix<In, Out, D, ROW, COL, false, ROW_OFFSET, COL_OFFSET>
{
    type ColPinsType = [In; COL];
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> ColPins<true> for Matrix<In, Out, D, ROW, COL, true, ROW_OFFSET, COL_OFFSET>
{
    type ColPinsType = [Out; COL];
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> MatrixOutputPins<Out> for Matrix<In, Out, D, ROW, COL, true, ROW_OFFSET, COL_OFFSET>
{
    fn get_output_pins(&self) -> &[Out] {
        &self.col_pins
    }

    fn get_output_pins_mut(&mut self) -> &mut [Out] {
        &mut self.col_pins
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> MatrixOutputPins<Out> for Matrix<In, Out, D, ROW, COL, false, ROW_OFFSET, COL_OFFSET>
{
    fn get_output_pins(&self) -> &[Out] {
        &self.row_pins
    }

    fn get_output_pins_mut(&mut self) -> &mut [Out] {
        &mut self.row_pins
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> MatrixInputPins<In> for Matrix<In, Out, D, ROW, COL, true, ROW_OFFSET, COL_OFFSET>
{
    fn get_input_pins(&self) -> &[In] {
        &self.row_pins
    }

    fn get_input_pins_mut(&mut self) -> &mut [In] {
        &mut self.row_pins
    }

    #[cfg(feature = "async_matrix")]
    async fn wait_input_pins(&mut self) {
        let futs = self.row_pins.each_mut().map(|input_pin| input_pin.wait_for_high());
        let _ = select_array(futs).await;
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> MatrixInputPins<In> for Matrix<In, Out, D, ROW, COL, false, ROW_OFFSET, COL_OFFSET>
{
    fn get_input_pins(&self) -> &[In] {
        &self.col_pins
    }

    fn get_input_pins_mut(&mut self) -> &mut [In] {
        &mut self.col_pins
    }

    #[cfg(feature = "async_matrix")]
    async fn wait_input_pins(&mut self) {
        let futs = self.col_pins.each_mut().map(|input_pin| input_pin.wait_for_high());
        let _ = select_array(futs).await;
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const COL2ROW: bool,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> Matrix<In, Out, D, ROW, COL, COL2ROW, ROW_OFFSET, COL_OFFSET>
where
    Self: RowPins<COL2ROW>,
    Self: ColPins<COL2ROW>,
{
    const OUTPUT_PIN_NUM: usize = const { if COL2ROW { COL } else { ROW } };
    const INPUT_PIN_NUM: usize = const { if COL2ROW { ROW } else { COL } };

    /// Create a matrix from input and output pins.
    pub fn new(
        row_pins: <Self as RowPins<COL2ROW>>::RowPinsType,
        col_pins: <Self as ColPins<COL2ROW>>::ColPinsType,
        debouncer: D,
    ) -> Self {
        Matrix {
            row_pins,
            col_pins,
            debouncer,
            key_states: [[KeyState::new(); ROW]; COL],
            scan_pos: (0, 0),
            #[cfg(feature = "async_matrix")]
            rescan_needed: false,
        }
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const COL2ROW: bool,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> InputDevice for Matrix<In, Out, D, ROW, COL, COL2ROW, ROW_OFFSET, COL_OFFSET>
where
    Self: RowPins<COL2ROW>,
    Self: ColPins<COL2ROW>,
    Self: MatrixOutputPins<Out>,
    Self: MatrixInputPins<In>,
{
    type Event = KeyboardEvent;

    async fn read_event(&mut self) -> Self::Event {
        loop {
            let (out_idx_start, in_idx_start) = self.scan_pos;
            #[cfg(not(feature = "async_matrix"))]
            let mut any_active = false;

            // Scan matrix and send report
            for out_idx in out_idx_start..Self::OUTPUT_PIN_NUM {
                // Pull up output pin, wait 1us ensuring the change comes into effect
                if let Some(out_pin) = self.get_output_pins_mut().get_mut(out_idx) {
                    out_pin.set_high().ok();
                }
                // This may take >1ms on some platforms if other tasks are running!
                Timer::after_micros(1).await;

                let in_start = if out_idx == out_idx_start { in_idx_start } else { 0 };

                for in_idx in in_start..Self::INPUT_PIN_NUM {
                    let in_pin_state = if let Some(in_pin) = self.get_input_pins_mut().get_mut(in_idx) {
                        in_pin.is_high().ok().unwrap_or_default()
                    } else {
                        false
                    };
                    // Check input pins and debounce
                    // Convert in_idx/out_idx to row_idx/col_idx based on COL2ROW
                    let (row_idx, col_idx) = if COL2ROW { (in_idx, out_idx) } else { (out_idx, in_idx) };
                    let debounce_state = self.debouncer.detect_change_with_debounce(
                        row_idx,
                        col_idx,
                        in_pin_state,
                        &self.key_states[col_idx][row_idx],
                    );

                    if let DebounceState::Debounced = debounce_state {
                        self.key_states[col_idx][row_idx].toggle_pressed();
                        self.scan_pos = (out_idx, in_idx);
                        #[cfg(feature = "async_matrix")]
                        {
                            self.rescan_needed = true;
                        }
                        // Pull it back to low before returning
                        if let Some(out_pin) = self.get_output_pins_mut().get_mut(out_idx) {
                            out_pin.set_low().ok();
                        }
                        return KeyboardEvent::key(
                            (row_idx + ROW_OFFSET) as u8,
                            (col_idx + COL_OFFSET) as u8,
                            self.key_states[col_idx][row_idx].pressed,
                        );
                    }

                    // If there's key still pressed, always refresh the self.scan_start
                    #[cfg(feature = "async_matrix")]
                    if self.key_states[col_idx][row_idx].pressed {
                        self.rescan_needed = true;
                    }

                    // Keep scanning at full rate while a key is held or bouncing,
                    // so releases and chord changes are caught immediately.
                    #[cfg(not(feature = "async_matrix"))]
                    if self.key_states[col_idx][row_idx].pressed || matches!(debounce_state, DebounceState::InProgress)
                    {
                        any_active = true;
                    }
                }

                // Pull it back to low
                if let Some(out_pin) = self.get_output_pins_mut().get_mut(out_idx) {
                    out_pin.set_low().ok();
                }
            }

            #[cfg(feature = "async_matrix")]
            {
                if !self.rescan_needed {
                    self.wait_for_key().await;
                }
                self.rescan_needed = false;
            }
            // Polling builds have no interrupt gate, so slow down when nothing is
            // pressed or debouncing instead of busy-scanning the whole matrix.
            #[cfg(not(feature = "async_matrix"))]
            if !any_active {
                Timer::after_millis(crate::MATRIX_IDLE_SCAN_MS.into()).await;
            }
            self.scan_pos = (0, 0);
        }
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const COL2ROW: bool,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> Runnable for Matrix<In, Out, D, ROW, COL, COL2ROW, ROW_OFFSET, COL_OFFSET>
where
    Self: RowPins<COL2ROW>,
    Self: ColPins<COL2ROW>,
    Self: MatrixOutputPins<Out>,
    Self: MatrixInputPins<In>,
{
    async fn run(&mut self) -> ! {
        loop {
            let event = self.read_event().await;
            publish_event_async(event).await;
        }
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    Out: OutputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const COL2ROW: bool,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> MatrixTrait<ROW, COL> for Matrix<In, Out, D, ROW, COL, COL2ROW, ROW_OFFSET, COL_OFFSET>
where
    Self: RowPins<COL2ROW>,
    Self: ColPins<COL2ROW>,
    Self: MatrixOutputPins<Out>,
    Self: MatrixInputPins<In>,
{
    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {
        // First, set all output pins to high
        for out in self.get_output_pins_mut().iter_mut() {
            out.set_high().ok();
        }

        // Wait for any key press
        self.wait_input_pins().await;

        // Set all output pins back to low
        for out in self.get_output_pins_mut().iter_mut() {
            out.set_low().ok();
        }
    }
}

#[input_device(publish = KeyboardEvent)]
pub struct TestMatrix<const ROW: usize, const COL: usize> {
    last: bool,
}
impl<const ROW: usize, const COL: usize> Default for TestMatrix<ROW, COL> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ROW: usize, const COL: usize> TestMatrix<ROW, COL> {
    pub fn new() -> Self {
        Self { last: false }
    }

    async fn read_keyboard_event(&mut self) -> KeyboardEvent {
        if self.last {
            embassy_time::Timer::after_millis(100).await;
        } else {
            embassy_time::Timer::after_secs(5).await;
        }
        self.last = !self.last;
        KeyboardEvent::key(0, 0, self.last)
    }
}

impl<const ROW: usize, const COL: usize> MatrixTrait<ROW, COL> for TestMatrix<ROW, COL> {
    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {}
}
