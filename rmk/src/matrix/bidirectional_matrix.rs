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
            let mut any_active = false;

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

                        // Keep scanning at full rate while a key is held or bouncing.
                        if self.key_state[scan_y_idx][scan_x_idx].pressed
                            || matches!(debounce_state, DebounceState::InProgress)
                        {
                            any_active = true;
                        }

                        // Pull output pin back to low
                        out_pin.set_low().ok();
                        out_pin.set_as_input();
                        Timer::after_micros(1).await;
                    }
                }
            }
            // Nothing is pressed or debouncing: idle-scan instead of busy-scanning
            // the whole matrix so the CPU can sleep between passes.
            if !any_active {
                Timer::after_millis(crate::MATRIX_IDLE_SCAN_MS.into()).await;
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
