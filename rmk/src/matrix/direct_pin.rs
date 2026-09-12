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
            #[cfg(not(feature = "async_matrix"))]
            let mut any_active = false;

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

                        // Keep scanning at full rate while a key is held or bouncing.
                        #[cfg(not(feature = "async_matrix"))]
                        if self.key_states[row_idx][col_idx].pressed
                            || matches!(debounce_state, DebounceState::InProgress)
                        {
                            any_active = true;
                        }
                    }
                }
            }

            self.scan_pos = (0, 0);

            // The interrupt gate in wait_for_key already covers idle in async
            // builds; polling builds slow down when nothing is pressed or
            // debouncing so the CPU can sleep between passes.
            #[cfg(feature = "async_matrix")]
            Timer::after_micros(100).await;

            #[cfg(not(feature = "async_matrix"))]
            if any_active {
                Timer::after_micros(100).await;
            } else {
                Timer::after_millis(crate::MATRIX_IDLE_SCAN_MS.into()).await;
            }
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

#[cfg(all(test, not(feature = "async_matrix")))]
mod tests {
    use embassy_time::{Duration, Instant};
    use embedded_hal_mock::eh1::digital::{Mock as PinMock, State as PinState, Transaction as PinTrans};

    use super::*;
    use crate::debounce::fast_debouncer::FastDebouncer;
    use crate::test_support::test_block_on as block_on;

    /// While nobody is pressing or debouncing, each pass must actually sleep
    /// `MATRIX_IDLE_SCAN_MS` instead of busy-polling, and a fresh press must
    /// still be caught on the very next pass.
    #[test]
    fn idle_pass_sleeps_then_catches_the_next_press() {
        let expectations = [
            PinTrans::get(PinState::Low),
            PinTrans::get(PinState::Low),
            PinTrans::get(PinState::High),
        ];
        let pin = PinMock::new(&expectations);
        let mut matrix: DirectPinMatrix<PinMock, FastDebouncer<1, 1>, 1, 1, 1> =
            DirectPinMatrix::new([[Some(pin)]], FastDebouncer::new(), false);

        let (elapsed, event) = block_on(async {
            let start = Instant::now();
            let event = matrix.read_keyboard_event().await;
            (start.elapsed(), event)
        });

        assert!(event.pressed);
        let idle_period = Duration::from_millis(crate::MATRIX_IDLE_SCAN_MS as u64);
        // Two idle passes must have actually slept, not busy-looped: bounded
        // below by two full idle periods (minus one tick of rounding slack)...
        assert!(
            elapsed >= idle_period * 2 - Duration::from_micros(100),
            "expected ~2 idle sleeps of {idle_period:?}, only {elapsed:?} of virtual time passed"
        );
        // ...and bounded above so a fresh press is still caught promptly,
        // rather than the matrix getting stuck idling forever.
        assert!(
            elapsed < idle_period * 3,
            "press should be caught within the next idle pass, took {elapsed:?}"
        );

        matrix.direct_pins[0][0].as_mut().unwrap().done();
    }

    /// A key held down on one column must keep the whole pass at active rate,
    /// even while a second column sees nothing change pass after pass: the
    /// held key's `pressed` state, not just its own debounce state, has to
    /// hold `any_active` true for the other column too.
    #[test]
    fn held_key_keeps_active_scan_rate_for_the_whole_pass() {
        // Column 0 (key A) is pressed and held throughout. Column 1 (key B)
        // stays low for 5 passes, then goes high on the 6th.
        // +1 for the initial press pass, which reads column A before returning.
        let a_expectations = core::array::from_fn::<_, 7, _>(|_| PinTrans::get(PinState::High));
        let mut b_expectations = vec![PinTrans::get(PinState::Low); 5];
        b_expectations.push(PinTrans::get(PinState::High));

        let pin_a = PinMock::new(&a_expectations);
        let pin_b = PinMock::new(&b_expectations);
        let mut matrix: DirectPinMatrix<PinMock, FastDebouncer<1, 2>, 1, 2, 2> =
            DirectPinMatrix::new([[Some(pin_a), Some(pin_b)]], FastDebouncer::new(), false);

        // Press and hold key A (first pass, returns immediately: not idle-gated).
        let a_press = block_on(matrix.read_keyboard_event());
        assert!(a_press.pressed);

        // Now watch key B: 5 no-change passes with A held, then B's press.
        let (elapsed, b_press) = block_on(async {
            let start = Instant::now();
            let event = matrix.read_keyboard_event().await;
            (start.elapsed(), event)
        });
        assert!(b_press.pressed);
        // 5 passes at the active 100us cadence is ~500us; a single idle sleep
        // would already blow past MATRIX_IDLE_SCAN_MS (10ms default).
        assert!(
            elapsed < Duration::from_millis(crate::MATRIX_IDLE_SCAN_MS as u64) / 2,
            "key B's no-change passes were idle-gated despite key A being held, took {elapsed:?}"
        );

        matrix.direct_pins[0][0].as_mut().unwrap().done();
        matrix.direct_pins[0][1].as_mut().unwrap().done();
    }
}
