use core::sync::atomic::Ordering;

use embassy_time::Timer;
use embedded_hal::digital::{InputPin, OutputPin};
#[cfg(feature = "async_matrix")]
use {embassy_futures::select::select_array, embedded_hal_async::digital::Wait};

use crate::CONNECTION_STATE;
use crate::debounce::{DebounceState, DebouncerTrait};
use crate::event::{Event, KeyPos, KeyboardEvent, KeyboardEventPos};
use crate::input_device::InputDevice;
use crate::state::ConnectionState;

pub mod bidirectional_matrix;

/// Recording the matrix pressed state
#[cfg(feature = "vial_lock")]
pub struct MatrixState<const ROW: usize, const COL: usize> {
    // 30 bytes is the limited by Vial and 240 keys is enough for
    // most keyborad
    state: [u8; 30],
}

#[cfg(feature = "vial_lock")]
impl<const ROW: usize, const COL: usize> Default for MatrixState<ROW, COL> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "vial_lock")]
impl<const ROW: usize, const COL: usize> MatrixState<ROW, COL> {
    const ROW_LEN: usize = (COL + 8) / 8;
    const OUT_OF_BOUNDARY: () = if ROW * Self::ROW_LEN > 30 {
        panic!(
            "Cannot use matrix tester because your keyboard has too many keys. \
            Consider disable the `matrix_tester` feature"
        )
    };
    pub fn new() -> Self {
        Self { state: [0; 30] }
    }
    pub fn update(&mut self, event: &crate::event::KeyboardEvent) {
        use crate::event::KeyboardEventPos;
        if let KeyboardEventPos::Key(crate::event::KeyPos { row, col }) = event.pos {
            if row as usize >= ROW || col as usize >= COL {
                warn!("Matrix read out of bounds");
                return;
            }
            let pressed = event.pressed;
            let index = row as usize * Self::ROW_LEN * 8 + col as usize;
            let byte_index = index / 8;
            let bit_index = index % 8;
            self.state[byte_index] = self.state[byte_index] & !(1 << bit_index) | ((pressed as u8) << bit_index);
        }
    }
    pub fn read_all(&self, target: &mut [u8]) {
        let slice = &self.state[..(ROW * Self::ROW_LEN)];
        let mut target_iter = target.iter_mut();
        for row_bytes in slice.chunks(Self::ROW_LEN) {
            for byte in row_bytes.iter().rev() {
                if let Some(target_byte) = target_iter.next() {
                    *target_byte = *byte;
                } else {
                    break;
                }
            }
        }
    }
    pub fn read(&self, row: u8, col: u8) -> bool {
        if row as usize >= ROW || col as usize >= COL {
            warn!("Matrix read out of bounds");
            return false;
        }
        let index = row as usize * Self::ROW_LEN * 8 + col as usize;
        let byte_index = index / 8;
        let bit_index = index % 8;
        self.state[byte_index] & (1 << bit_index) != 0
    }
}

/// MatrixTrait is the trait for keyboard matrix.
///
/// The keyboard matrix is a 2D matrix of keys, the matrix does the scanning and saves the result to each key's `KeyState`.
pub trait MatrixTrait<const ROW: usize, const COL: usize>: InputDevice {
    // Wait for USB or BLE really connected
    async fn wait_for_connected(&self) {
        while CONNECTION_STATE.load(Ordering::Acquire) == Into::<bool>::into(ConnectionState::Disconnected) {
            embassy_time::Timer::after_millis(100).await;
        }
        info!("Connected, start scanning matrix");
    }

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
> RowPins<true> for Matrix<In, Out, D, ROW, COL, true>
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
> RowPins<false> for Matrix<In, Out, D, ROW, COL, false>
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
> ColPins<false> for Matrix<In, Out, D, ROW, COL, false>
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
> ColPins<true> for Matrix<In, Out, D, ROW, COL, true>
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
> MatrixOutputPins<Out> for Matrix<In, Out, D, ROW, COL, true>
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
> MatrixOutputPins<Out> for Matrix<In, Out, D, ROW, COL, false>
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
> MatrixInputPins<In> for Matrix<In, Out, D, ROW, COL, true>
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
> MatrixInputPins<In> for Matrix<In, Out, D, ROW, COL, false>
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
> Matrix<In, Out, D, ROW, COL, COL2ROW>
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
> InputDevice for Matrix<In, Out, D, ROW, COL, COL2ROW>
where
    Self: RowPins<COL2ROW>,
    Self: ColPins<COL2ROW>,
    Self: MatrixOutputPins<Out>,
    Self: MatrixInputPins<In>,
{
    async fn read_event(&mut self) -> crate::event::Event {
        loop {
            let (out_idx_start, in_idx_start) = self.scan_pos;

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
                        return Event::Key(KeyboardEvent::key(
                            row_idx as u8,
                            col_idx as u8,
                            self.key_states[col_idx][row_idx].pressed,
                        ));
                    }

                    // If there's key still pressed, always refresh the self.scan_start
                    #[cfg(feature = "async_matrix")]
                    if self.key_states[col_idx][row_idx].pressed {
                        self.rescan_needed = true;
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
> MatrixTrait<ROW, COL> for Matrix<In, Out, D, ROW, COL, COL2ROW>
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

pub struct OffsetMatrixWrapper<
    const ROW: usize,
    const COL: usize,
    M: MatrixTrait<ROW, COL>,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
>(pub M);

impl<const ROW: usize, const COL: usize, M: MatrixTrait<ROW, COL>, const ROW_OFFSET: usize, const COL_OFFSET: usize>
    InputDevice for OffsetMatrixWrapper<ROW, COL, M, ROW_OFFSET, COL_OFFSET>
{
    async fn read_event(&mut self) -> Event {
        match self.0.read_event().await {
            Event::Key(KeyboardEvent {
                pressed,
                pos: KeyboardEventPos::Key(KeyPos { row, col }),
            }) => Event::Key(KeyboardEvent::key(
                row + ROW_OFFSET as u8,
                col + COL_OFFSET as u8,
                pressed,
            )),
            event => event,
        }
    }
}

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
}

impl<const ROW: usize, const COL: usize> MatrixTrait<ROW, COL> for TestMatrix<ROW, COL> {
    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {}
}

impl<const ROW: usize, const COL: usize> InputDevice for TestMatrix<ROW, COL> {
    async fn read_event(&mut self) -> Event {
        if self.last {
            embassy_time::Timer::after_millis(100).await;
        } else {
            embassy_time::Timer::after_secs(5).await;
        }
        self.last = !self.last;
        // info!("Read event: {:?}", self.last);
        Event::Key(KeyboardEvent::key(0, 0, self.last))
    }
}
