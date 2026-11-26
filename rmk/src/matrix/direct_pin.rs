use embassy_time::Timer;
use embedded_hal;
use embedded_hal::digital::InputPin;
use rmk_macro::input_device;
#[cfg(feature = "async_matrix")]
use {
    core::future::pending, embassy_futures::select::select_array, embassy_time::Instant,
    embedded_hal_async::digital::Wait,
};

use super::{KeyState, MatrixTrait};
use crate::debounce::{DebounceState, DebouncerTrait};
use crate::event::KeyboardEvent;

/// DirectPinMartex only has input pins.
#[input_device(publish = KeyboardEvent)]
pub struct DirectPinMatrix<
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const SIZE: usize,
    const ROW_OFFSET: usize = 0,
    const COL_OFFSET: usize = 0,
> {
    /// Input pins of the pcb matrix
    direct_pins: [[Option<In>; COL]; ROW],
    /// Debouncer
    debouncer: D,
    /// Key state matrix
    key_states: [[KeyState; COL]; ROW],
    /// Start scanning — used by async-matrix wait gating only.
    #[cfg(feature = "async_matrix")]
    scan_start: Option<Instant>,
    /// Pin active level
    low_active: bool,
    /// Current scan pos: (out_idx, in_idx)
    scan_pos: (usize, usize),
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const SIZE: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> DirectPinMatrix<In, D, ROW, COL, SIZE, ROW_OFFSET, COL_OFFSET>
{
    /// Create a matrix from input and output pins.
    pub fn new(direct_pins: [[Option<In>; COL]; ROW], debouncer: D, low_active: bool) -> Self {
        DirectPinMatrix {
            direct_pins,
            debouncer,
            key_states: [[KeyState::new(); COL]; ROW],
            #[cfg(feature = "async_matrix")]
            scan_start: None,
            low_active,
            scan_pos: (0, 0),
        }
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const SIZE: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> DirectPinMatrix<In, D, ROW, COL, SIZE, ROW_OFFSET, COL_OFFSET>
{
    /// Read a keyboard event from the direct pin matrix.
    /// This method is called by the generated InputDevice implementation.
    async fn read_keyboard_event(&mut self) -> KeyboardEvent {
        loop {
            let (row_idx_start, col_idx_start) = self.scan_pos;

            #[cfg(feature = "async_matrix")]
            self.wait_for_key().await;

            // Scan matrix and send report
            for row_idx in row_idx_start..self.direct_pins.len() {
                let pins_row = self.direct_pins.get_mut(row_idx).unwrap();
                let col_start = if row_idx == row_idx_start { col_idx_start } else { 0 };
                for col_idx in col_start..pins_row.len() {
                    let direct_pin = pins_row.get_mut(col_idx).unwrap();
                    if let Some(direct_pin) = direct_pin {
                        let pin_state = if self.low_active {
                            direct_pin.is_low().ok().unwrap_or_default()
                        } else {
                            direct_pin.is_high().ok().unwrap_or_default()
                        };

                        let debounce_state = self.debouncer.detect_change_with_debounce(
                            row_idx,
                            col_idx,
                            pin_state,
                            &self.key_states[row_idx][col_idx],
                        );

                        if let DebounceState::Debounced = debounce_state {
                            self.key_states[row_idx][col_idx].toggle_pressed();
                            let key_state = self.key_states[row_idx][col_idx];

                            self.scan_pos = (row_idx, col_idx);
                            return KeyboardEvent::key(
                                (row_idx + ROW_OFFSET) as u8,
                                (col_idx + COL_OFFSET) as u8,
                                key_state.pressed,
                            );
                        }

                        // If there's key still pressed, always refresh the self.scan_start
                        #[cfg(feature = "async_matrix")]
                        if self.key_states[row_idx][col_idx].pressed {
                            self.scan_start = Some(Instant::now());
                        }
                    }
                }
            }

            self.scan_pos = (0, 0);

            Timer::after_micros(100).await;
        }
    }
}

impl<
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const SIZE: usize,
    const ROW_OFFSET: usize,
    const COL_OFFSET: usize,
> MatrixTrait<ROW, COL> for DirectPinMatrix<In, D, ROW, COL, SIZE, ROW_OFFSET, COL_OFFSET>
{
    #[cfg(feature = "async_matrix")]
    async fn wait_for_key(&mut self) {
        if let Some(start_time) = self.scan_start {
            // If no key press over 1ms, stop scanning and wait for interupt
            if start_time.elapsed().as_millis() <= 1 {
                return;
            } else {
                self.scan_start = None;
            }
        }
        Timer::after_micros(1).await;
        info!("Waiting for active level");

        let low_active = self.low_active;
        let futs = self.direct_pins.each_mut().map(|direct_pins_row| {
            let row_futs = direct_pins_row.each_mut().map(|direct_pin| async move {
                match (direct_pin, low_active) {
                    (Some(direct_pin), true) => {
                        let _ = direct_pin.wait_for_low().await;
                    }
                    (Some(direct_pin), false) => {
                        let _ = direct_pin.wait_for_high().await;
                    }
                    (None, _) => pending().await,
                }
            });
            select_array(row_futs)
        });
        let _ = select_array(futs).await;
        self.scan_start = Some(Instant::now());
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;
    use core::convert::Infallible;
    use std::rc::Rc;

    use embedded_hal::digital::ErrorType;

    use super::*;
    use crate::test_support::test_block_on;

    #[derive(Clone, Default)]
    struct FakePin(Rc<Cell<bool>>);

    impl ErrorType for FakePin {
        type Error = Infallible;
    }

    impl InputPin for FakePin {
        fn is_high(&mut self) -> Result<bool, Infallible> {
            Ok(self.0.get())
        }

        fn is_low(&mut self) -> Result<bool, Infallible> {
            Ok(!self.0.get())
        }
    }

    #[cfg(feature = "async_matrix")]
    impl Wait for FakePin {
        async fn wait_for_high(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
        async fn wait_for_low(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
        async fn wait_for_rising_edge(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
        async fn wait_for_falling_edge(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
        async fn wait_for_any_edge(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
    }

    struct ImmediateDebouncer;

    impl<const ROW: usize, const COL: usize> DebouncerTrait<ROW, COL> for ImmediateDebouncer {
        fn detect_change_with_debounce(&mut self, _: usize, _: usize, active: bool, state: &KeyState) -> DebounceState {
            if active != state.pressed {
                DebounceState::Debounced
            } else {
                DebounceState::Ignored
            }
        }
    }

    #[test]
    fn simultaneous_presses_on_later_rows_are_reported_in_scan_order() {
        let levels: [[FakePin; 3]; 2] = Default::default();
        let pins = levels.clone().map(|row| row.map(Some));
        let mut matrix: DirectPinMatrix<_, _, 2, 3, 6> = DirectPinMatrix::new(pins, ImmediateDebouncer, false);

        let keys = [(0, 1), (0, 2), (1, 0), (1, 2)];
        for &(row, col) in &keys {
            levels[row][col].0.set(true);
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
