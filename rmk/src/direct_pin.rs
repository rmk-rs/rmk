use embassy_time::{Instant, Timer};
use embedded_hal;
use embedded_hal::digital::InputPin;
#[cfg(feature = "async_matrix")]
use {core::future::pending, embassy_futures::select::select_array, embedded_hal_async::digital::Wait};

use crate::MatrixTrait;
use crate::debounce::{DebounceState, DebouncerTrait};
use crate::event::{Event, KeyboardEvent};
use crate::input_device::InputDevice;
use crate::matrix::KeyState;

/// DirectPinMartex only has input pins.
pub struct DirectPinMatrix<
    #[cfg(feature = "async_matrix")] In: Wait + InputPin,
    #[cfg(not(feature = "async_matrix"))] In: InputPin,
    D: DebouncerTrait<ROW, COL>,
    const ROW: usize,
    const COL: usize,
    const SIZE: usize,
> {
    /// Input pins of the pcb matrix
    direct_pins: [[Option<In>; COL]; ROW],
    /// Debouncer
    debouncer: D,
    /// Key state matrix
    key_states: [[KeyState; COL]; ROW],
    /// Start scanning
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
> DirectPinMatrix<In, D, ROW, COL, SIZE>
{
    /// Create a matrix from input and output pins.
    pub fn new(direct_pins: [[Option<In>; COL]; ROW], debouncer: D, low_active: bool) -> Self {
        DirectPinMatrix {
            direct_pins,
            debouncer,
            key_states: [[KeyState::new(); COL]; ROW],
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
> InputDevice for DirectPinMatrix<In, D, ROW, COL, SIZE>
{
    async fn read_event(&mut self) -> crate::event::Event {
        loop {
            let (row_idx_start, col_idx_start) = self.scan_pos;

            #[cfg(feature = "async_matrix")]
            self.wait_for_key().await;

            // Scan matrix and send report
            for row_idx in row_idx_start..self.direct_pins.len() {
                let pins_row = self.direct_pins.get_mut(row_idx).unwrap();
                for col_idx in col_idx_start..pins_row.len() {
                    let direct_pin = pins_row.get_mut(col_idx).unwrap();
                    // for (col_idx, direct_pin) in pins_row.iter_mut().enumerate() {
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
                            return Event::Key(KeyboardEvent::key(row_idx as u8, col_idx as u8, key_state.pressed));
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
> MatrixTrait<ROW, COL> for DirectPinMatrix<In, D, ROW, COL, SIZE>
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
