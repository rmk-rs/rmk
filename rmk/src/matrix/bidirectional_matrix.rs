use embassy_time::Timer;
use rmk_macro::input_device;

use crate::debounce::{DebounceState, DebouncerTrait};
use crate::driver::flex_pin::FlexPin;
use crate::event::KeyboardEvent;
use crate::matrix::{KeyState, MatrixTrait};

pub enum ScanLocation {
    Pins(usize, usize),
    Ignore,
}

/// Matrix is the physical pcb layout of the keyboard matrix.
#[input_device(publish = KeyboardEvent)]
pub struct BidirectionalMatrix<
    Pin: FlexPin,
    D: DebouncerTrait<ROW, COL>,
    const PIN_NUM: usize,
    const ROW: usize,
    const COL: usize,
> {
    /// Input pins of the pcb matrix
    pins: [Pin; PIN_NUM],
    /// Debouncer
    debouncer: D,
    /// Key state matrix
    key_state: [[KeyState; COL]; ROW],
    /// Current scan pos: (row_idx, col_idx)
    scan_pos: (usize, usize),
    /// Scan map
    scan_map: [[ScanLocation; COL]; ROW],
}

impl<Pin: FlexPin, D: DebouncerTrait<ROW, COL>, const PIN_NUM: usize, const ROW: usize, const COL: usize>
    BidirectionalMatrix<Pin, D, PIN_NUM, ROW, COL>
{
    /// Create a matrix from input and output pins.
    pub fn new(pins: [Pin; PIN_NUM], debouncer: D, scan_map: [[ScanLocation; COL]; ROW]) -> Self {
        BidirectionalMatrix {
            pins,
            debouncer,
            key_state: [[KeyState::new(); COL]; ROW],
            scan_pos: (0, 0),
            scan_map,
        }
    }
}

impl<Pin: FlexPin, D: DebouncerTrait<ROW, COL>, const PIN_NUM: usize, const ROW: usize, const COL: usize>
    BidirectionalMatrix<Pin, D, PIN_NUM, ROW, COL>
{
    async fn read_keyboard_event(&mut self) -> KeyboardEvent {
        loop {
            let (scan_x_start, scan_y_start) = self.scan_pos;

            // Scan following the scan map and send report
            // Loop through rows.
            for scan_y_idx in scan_y_start..self.scan_map.len() {
                // Loop trough cols.
                let scan_x_start_current = if scan_y_idx == scan_y_start { scan_x_start } else { 0 };
                for scan_x_idx in scan_x_start_current..self.scan_map[scan_y_idx].len() {
                    if let ScanLocation::Pins(in_idx, out_idx) = self.scan_map[scan_y_idx][scan_x_idx] {
                        let [in_pin, out_pin] = self.pins.get_disjoint_mut([in_idx, out_idx]).unwrap();
                        // Set output pin to high.
                        out_pin.set_as_output();
                        out_pin.set_high().ok();
                        Timer::after_micros(1).await;

                        // Check input pin and debounce
                        let debounce_state = self.debouncer.detect_change_with_debounce(
                            scan_y_idx,
                            scan_x_idx,
                            in_pin.is_high().ok().unwrap_or_default(),
                            &self.key_state[scan_y_idx][scan_x_idx],
                        );
                        if let DebounceState::Debounced = debounce_state {
                            self.key_state[scan_y_idx][scan_x_idx].toggle_pressed();
                            self.scan_pos = (scan_y_idx, scan_x_idx);
                            // Pull output pin back to low before returning
                            out_pin.set_low().ok();
                            out_pin.set_as_input();
                            return KeyboardEvent::key(
                                scan_y_idx as u8,
                                scan_x_idx as u8,
                                self.key_state[scan_y_idx][scan_x_idx].pressed,
                            );
                        }

                        // Pull output pin back to low
                        out_pin.set_low().ok();
                        out_pin.set_as_input();
                        Timer::after_micros(1).await;
                    }
                }
            }
            self.scan_pos = (0, 0);
        }
    }
}

impl<Pin: FlexPin, D: DebouncerTrait<ROW, COL>, const PIN_NUM: usize, const ROW: usize, const COL: usize>
    MatrixTrait<ROW, COL> for BidirectionalMatrix<Pin, D, PIN_NUM, ROW, COL>
{
    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {}
}

#[cfg(test)]
mod tests {
    use core::cell::RefCell;
    use core::convert::Infallible;
    use std::rc::Rc;

    use embedded_hal::digital::{ErrorType, InputPin, OutputPin};

    use super::*;
    use crate::test_support::test_block_on;

    const ROW: usize = 2;
    const COL: usize = 3;

    /// Row `r` is read on pin `r`; column `c` is driven on pin `ROW + c`.
    #[derive(Default)]
    struct Board {
        driven: Option<usize>,
        pressed: [[bool; COL]; ROW],
    }

    struct FakePin {
        idx: usize,
        board: Rc<RefCell<Board>>,
    }

    impl ErrorType for FakePin {
        type Error = Infallible;
    }

    impl InputPin for FakePin {
        fn is_high(&mut self) -> Result<bool, Infallible> {
            let board = self.board.borrow();
            Ok(self.idx < ROW && board.driven.is_some_and(|out| board.pressed[self.idx][out - ROW]))
        }

        fn is_low(&mut self) -> Result<bool, Infallible> {
            self.is_high().map(|high| !high)
        }
    }

    impl OutputPin for FakePin {
        fn set_high(&mut self) -> Result<(), Infallible> {
            self.board.borrow_mut().driven = Some(self.idx);
            Ok(())
        }

        fn set_low(&mut self) -> Result<(), Infallible> {
            self.board.borrow_mut().driven = None;
            Ok(())
        }
    }

    impl FlexPin for FakePin {
        fn set_as_input(&mut self) {}
        fn set_as_output(&mut self) {}
    }

    struct ImmediateDebouncer;

    impl DebouncerTrait<ROW, COL> for ImmediateDebouncer {
        fn detect_change_with_debounce(&mut self, _: usize, _: usize, active: bool, state: &KeyState) -> DebounceState {
            if active != state.pressed {
                DebounceState::Debounced
            } else {
                DebounceState::Ignored
            }
        }
    }

    #[test]
    fn simultaneous_presses_on_rectangular_matrix_are_reported_in_scan_order() {
        let board = Rc::new(RefCell::new(Board::default()));
        let pins = core::array::from_fn(|idx| FakePin {
            idx,
            board: board.clone(),
        });
        let scan_map = core::array::from_fn(|row| core::array::from_fn(|col| ScanLocation::Pins(row, ROW + col)));
        let mut matrix: BidirectionalMatrix<_, _, { ROW + COL }, ROW, COL> =
            BidirectionalMatrix::new(pins, ImmediateDebouncer, scan_map);

        let keys = [(0, 1), (0, 2), (1, 0), (1, 2)];
        for &(row, col) in &keys {
            board.borrow_mut().pressed[row][col] = true;
        }
        let events = test_block_on(async {
            let mut events = Vec::new();
            for _ in 0..keys.len() {
                events.push(matrix.read_keyboard_event().await);
            }
            events
        });
        let expected: Vec<_> = keys
            .iter()
            .map(|&(row, col)| KeyboardEvent::key(row as u8, col as u8, true))
            .collect();
        assert_eq!(events, expected);
    }
}
