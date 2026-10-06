use core::num::NonZeroU16;

use embassy_time::Instant;

use super::{DebounceState, DebouncerTrait};
use crate::DEBOUNCE_THRESHOLD;
use crate::matrix::KeyState;

/// Fast per-key debouncer.
pub struct FastDebouncer<const ROW: usize, const COL: usize> {
    started_at: [[Option<NonZeroU16>; ROW]; COL],
}

impl<const ROW: usize, const COL: usize> Default for FastDebouncer<ROW, COL> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const ROW: usize, const COL: usize> FastDebouncer<ROW, COL> {
    /// Create a fast debouncer
    pub fn new() -> Self {
        FastDebouncer {
            started_at: [[None; ROW]; COL],
        }
    }
}

impl<const ROW: usize, const COL: usize> DebouncerTrait<ROW, COL> for FastDebouncer<ROW, COL> {
    /// Per-key fast debounce
    fn detect_change_with_debounce(
        &mut self,
        row_idx: usize,
        col_idx: usize,
        key_active: bool,
        key_state: &KeyState,
    ) -> DebounceState {
        if let Some(started_at) = self.started_at[col_idx][row_idx] {
            // Current key is in debouncing state
            let elapsed = (Instant::now().as_millis() as u16).wrapping_sub(started_at.get());
            if elapsed > DEBOUNCE_THRESHOLD {
                // If the elapsed time > DEBOUNCE_THRESHOLD, reset
                self.started_at[col_idx][row_idx] = None;
                DebounceState::Ignored
            } else {
                // Still in a debouncing progress
                DebounceState::InProgress
            }
        } else if key_state.pressed != key_active {
            // If current key isn't in debouncing state, and a key change is detected
            // Trigger the key immediately and record current tick
            let now = Instant::now().as_millis() as u16;
            self.started_at[col_idx][row_idx] = Some(NonZeroU16::new(now).unwrap_or(NonZeroU16::MIN));
            DebounceState::Debounced
        } else {
            DebounceState::Ignored
        }
    }
}

#[cfg(test)]
mod tests {
    use embassy_time::{Duration, MockDriver};

    use super::*;

    #[test]
    fn debounce_windows_are_per_key() {
        MockDriver::get().reset();
        MockDriver::get().advance(Duration::from_millis(1));

        let mut debouncer = FastDebouncer::<1, 2>::new();
        let released = KeyState::new();
        assert!(matches!(
            debouncer.detect_change_with_debounce(0, 0, true, &released),
            DebounceState::Debounced
        ));

        MockDriver::get().advance(Duration::from_millis((DEBOUNCE_THRESHOLD - 1).into()));
        assert!(matches!(
            debouncer.detect_change_with_debounce(0, 1, true, &released),
            DebounceState::Debounced
        ));

        MockDriver::get().advance(Duration::from_millis(2));
        let pressed = KeyState { pressed: true };
        assert!(matches!(
            debouncer.detect_change_with_debounce(0, 0, false, &pressed),
            DebounceState::Ignored
        ));
    }
}
