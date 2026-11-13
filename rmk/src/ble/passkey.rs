use core::sync::atomic::{AtomicBool, Ordering};

#[cfg(feature = "passkey_entry")]
use embassy_futures::select::{Either, select};
use embassy_sync::signal::Signal;
#[cfg(feature = "passkey_entry")]
use embassy_time::{Duration, Instant, with_deadline};
use rmk_types::keycode::HidKeyCode;
#[cfg(feature = "passkey_entry")]
use trouble_host::prelude::{DefaultPacketPool, GattConnection, GattConnectionEvent};

#[doc(hidden)]
pub fn passkey_entry_enabled() -> bool {
    #[cfg(feature = "passkey_entry")]
    {
        crate::PASSKEY_ENTRY_ENABLED
    }
    #[cfg(not(feature = "passkey_entry"))]
    {
        false
    }
}

#[cfg(feature = "passkey_entry")]
pub(crate) struct PasskeyInputState {
    pub(crate) deadline: Option<Instant>,
    cleanup: Option<PasskeyCleanupGuard>,
}

#[cfg(feature = "passkey_entry")]
impl PasskeyInputState {
    pub(crate) const fn new() -> Self {
        Self {
            deadline: None,
            cleanup: None,
        }
    }

    pub(crate) fn clear(&mut self) {
        self.deadline = None;
        drop(self.cleanup.take());
    }

    pub(crate) fn begin(&mut self) {
        self.clear();
        begin_passkey_entry_session();
        self.cleanup = Some(PasskeyCleanupGuard::new());
        self.deadline = Some(Instant::now() + Duration::from_secs(crate::PASSKEY_ENTRY_TIMEOUT_SECS as u64));
    }
}

#[cfg(feature = "passkey_entry")]
pub(crate) async fn next_gatt_event<'a, 'b>(
    conn: &GattConnection<'a, 'b, DefaultPacketPool>,
    passkey_state: &mut PasskeyInputState,
) -> Option<GattConnectionEvent<'a, 'b, DefaultPacketPool>> {
    if crate::PASSKEY_ENTRY_ENABLED
        && let Some(deadline) = passkey_state.deadline
    {
        return match select(conn.next(), with_deadline(deadline, PASSKEY_RESPONSE.wait())).await {
            Either::First(event) => Some(event),
            Either::Second(Ok(Some(passkey))) => {
                passkey_state.clear();

                info!("[gatt] Passkey entered: submitting");
                if let Err(e) = conn.raw().pass_key_input(passkey) {
                    error!("[gatt] pass_key_input error: {:?}", e);
                }
                None
            }
            Either::Second(Ok(None)) => {
                passkey_state.clear();

                info!("[gatt] Passkey entry cancelled");
                if let Err(e) = conn.raw().pass_key_cancel() {
                    error!("[gatt] pass_key_cancel error: {:?}", e);
                }
                None
            }
            Either::Second(Err(_)) => {
                passkey_state.clear();

                warn!("[gatt] Passkey entry timeout");
                let _ = conn.raw().pass_key_cancel();
                None
            }
        };
    }

    Some(conn.next().await)
}

/// Maximum number of digits in a BLE passkey.
pub const PASSKEY_LENGTH: usize = 6;

/// Global flag indicating passkey entry mode is active.
/// Set when `PassKeyInput` event arrives, cleared on submit/cancel/timeout.
pub static PASSKEY_ENTRY_MODE: AtomicBool = AtomicBool::new(false);

/// Signal to carry the passkey result from the keyboard task back to the GATT task.
/// `Some(passkey)` = submit, `None` = cancel.
pub static PASSKEY_RESPONSE: Signal<crate::RawMutex, Option<u32>> = Signal::new();

/// Start a new passkey entry session.
///
/// IMPORTANT: reset the response signal before enabling passkey mode, so an
/// immediate keyboard response cannot be dropped by a late reset.
pub fn begin_passkey_entry_session() {
    PASSKEY_RESPONSE.reset();
    PASSKEY_ENTRY_MODE.store(true, Ordering::Release);
}

/// End the current passkey entry session.
pub fn end_passkey_entry_session() {
    PASSKEY_ENTRY_MODE.store(false, Ordering::Release);
}

/// Drop guard that clears passkey mode whenever the surrounding task exits.
pub struct PasskeyCleanupGuard;

impl Default for PasskeyCleanupGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl PasskeyCleanupGuard {
    pub const fn new() -> Self {
        Self
    }
}

impl Drop for PasskeyCleanupGuard {
    fn drop(&mut self) {
        end_passkey_entry_session();
    }
}

/// Result of processing a key press in passkey entry mode.
#[derive(Debug, PartialEq, Eq)]
pub enum PasskeyAction {
    /// A digit was successfully added. Contains the digit (0-9).
    DigitAdded(u8),
    /// All digits entered and submitted. Contains the assembled passkey.
    Submitted(u32),
    /// The user cancelled passkey entry (Escape).
    Cancelled,
    /// A digit was removed via Backspace.
    Backspaced,
    /// Digit rejected because the buffer is already full.
    BufferFull,
    /// Enter pressed but fewer than PASSKEY_LENGTH digits entered.
    Incomplete,
    /// Key was not a passkey-relevant key (silently consumed).
    Ignored,
}

/// State for passkey digit entry (up to PASSKEY_LENGTH digits).
pub struct PasskeyEntryState {
    digits: [u8; PASSKEY_LENGTH],
    count: usize,
    active: bool,
}

impl Default for PasskeyEntryState {
    fn default() -> Self {
        Self::new()
    }
}

impl PasskeyEntryState {
    pub const fn new() -> Self {
        Self {
            digits: [0; PASSKEY_LENGTH],
            count: 0,
            active: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn activate(&mut self) {
        self.active = true;
        self.reset();
    }

    pub fn deactivate(&mut self) {
        self.active = false;
    }

    pub fn check_mode_transition(&mut self) {
        use core::sync::atomic::Ordering;
        let passkey_active = crate::ble::passkey::PASSKEY_ENTRY_MODE.load(Ordering::Acquire);
        if passkey_active && !self.is_active() {
            self.activate();
        } else if !passkey_active && self.is_active() {
            self.deactivate();
        }
    }

    /// Reset the state for a new passkey entry session.
    pub fn reset(&mut self) {
        self.digits = [0; PASSKEY_LENGTH];
        self.count = 0;
    }

    /// Add a digit (0-9). Returns false if already at PASSKEY_LENGTH digits.
    pub fn add_digit(&mut self, digit: u8) -> bool {
        if self.count < PASSKEY_LENGTH {
            self.digits[self.count] = digit;
            self.count += 1;
            true
        } else {
            false
        }
    }

    /// Remove the last digit. Returns false if empty.
    pub fn remove_digit(&mut self) -> bool {
        if self.count > 0 {
            self.count -= 1;
            self.digits[self.count] = 0;
            true
        } else {
            false
        }
    }

    /// Whether we have all PASSKEY_LENGTH digits.
    pub fn is_complete(&self) -> bool {
        self.count == PASSKEY_LENGTH
    }

    /// Convert the entered digits to a u32 passkey.
    pub fn to_passkey(&self) -> u32 {
        let mut result: u32 = 0;
        for digit in self.digits.iter().take(self.count) {
            result = result * 10 + *digit as u32;
        }
        result
    }

    /// Number of digits entered so far.
    pub fn digit_count(&self) -> usize {
        self.count
    }

    /// Process a key press and return the resulting action.
    ///
    /// Encapsulates all passkey entry logic: digit entry, Enter/submit,
    /// Escape/cancel, Backspace/delete, and ignoring irrelevant keys.
    pub fn handle_key(&mut self, key: HidKeyCode) -> PasskeyAction {
        if let Some(digit) = hid_keycode_to_digit(key) {
            if self.add_digit(digit) {
                PasskeyAction::DigitAdded(digit)
            } else {
                PasskeyAction::BufferFull
            }
        } else if matches!(key, HidKeyCode::Enter | HidKeyCode::KpEnter) {
            if self.is_complete() {
                let passkey = self.to_passkey();
                self.reset();
                PasskeyAction::Submitted(passkey)
            } else {
                PasskeyAction::Incomplete
            }
        } else if matches!(key, HidKeyCode::Escape) {
            self.reset();
            PasskeyAction::Cancelled
        } else if matches!(key, HidKeyCode::Backspace) {
            if self.remove_digit() {
                PasskeyAction::Backspaced
            } else {
                PasskeyAction::Ignored
            }
        } else {
            PasskeyAction::Ignored
        }
    }
}

/// Convert a HID keycode to a digit (0-9), if applicable.
/// Supports both number row keys (Kc0-Kc9) and keypad keys (Kp0-Kp9).
pub fn hid_keycode_to_digit(k: HidKeyCode) -> Option<u8> {
    match k {
        HidKeyCode::Kc1 => Some(1),
        HidKeyCode::Kc2 => Some(2),
        HidKeyCode::Kc3 => Some(3),
        HidKeyCode::Kc4 => Some(4),
        HidKeyCode::Kc5 => Some(5),
        HidKeyCode::Kc6 => Some(6),
        HidKeyCode::Kc7 => Some(7),
        HidKeyCode::Kc8 => Some(8),
        HidKeyCode::Kc9 => Some(9),
        HidKeyCode::Kc0 => Some(0),
        HidKeyCode::Kp1 => Some(1),
        HidKeyCode::Kp2 => Some(2),
        HidKeyCode::Kp3 => Some(3),
        HidKeyCode::Kp4 => Some(4),
        HidKeyCode::Kp5 => Some(5),
        HidKeyCode::Kp6 => Some(6),
        HidKeyCode::Kp7 => Some(7),
        HidKeyCode::Kp8 => Some(8),
        HidKeyCode::Kp9 => Some(9),
        HidKeyCode::Kp0 => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_block_on as block_on;

    #[test]
    fn test_passkey_entry_state_basic() {
        let mut state = PasskeyEntryState::new();
        assert_eq!(state.digit_count(), 0);
        assert!(!state.is_complete());

        // Add digits 1-6
        for d in 1..=6 {
            assert!(state.add_digit(d));
        }
        assert!(state.is_complete());
        assert_eq!(state.to_passkey(), 123456);

        // Can't add 7th digit
        assert!(!state.add_digit(7));
        assert_eq!(state.to_passkey(), 123456);
    }

    #[test]
    fn test_passkey_entry_state_remove() {
        let mut state = PasskeyEntryState::new();
        assert!(!state.remove_digit()); // empty

        state.add_digit(1);
        state.add_digit(2);
        state.add_digit(3);
        assert_eq!(state.to_passkey(), 123);

        assert!(state.remove_digit());
        assert_eq!(state.to_passkey(), 12);
        assert_eq!(state.digit_count(), 2);
    }

    #[test]
    fn test_passkey_entry_state_reset() {
        let mut state = PasskeyEntryState::new();
        state.add_digit(9);
        state.add_digit(8);
        state.reset();
        assert_eq!(state.digit_count(), 0);
        assert_eq!(state.to_passkey(), 0);
    }

    #[test]
    fn test_passkey_with_zeros() {
        let mut state = PasskeyEntryState::new();
        // 007890
        state.add_digit(0);
        state.add_digit(0);
        state.add_digit(7);
        state.add_digit(8);
        state.add_digit(9);
        state.add_digit(0);
        assert_eq!(state.to_passkey(), 7890);
    }

    #[test]
    fn test_handle_key_digit() {
        let mut state = PasskeyEntryState::new();
        assert_eq!(state.handle_key(HidKeyCode::Kc1), PasskeyAction::DigitAdded(1));
        assert_eq!(state.handle_key(HidKeyCode::Kp5), PasskeyAction::DigitAdded(5));
        assert_eq!(state.digit_count(), 2);
    }

    #[test]
    fn test_handle_key_submit() {
        let mut state = PasskeyEntryState::new();
        for d in [
            HidKeyCode::Kc1,
            HidKeyCode::Kc2,
            HidKeyCode::Kc3,
            HidKeyCode::Kc4,
            HidKeyCode::Kc5,
            HidKeyCode::Kc6,
        ] {
            state.handle_key(d);
        }
        assert_eq!(state.handle_key(HidKeyCode::Enter), PasskeyAction::Submitted(123456));
        assert_eq!(state.digit_count(), 0); // reset after submit
    }

    #[test]
    fn test_handle_key_incomplete() {
        let mut state = PasskeyEntryState::new();
        state.handle_key(HidKeyCode::Kc1);
        assert_eq!(state.handle_key(HidKeyCode::Enter), PasskeyAction::Incomplete);
    }

    #[test]
    fn test_handle_key_cancel() {
        let mut state = PasskeyEntryState::new();
        state.handle_key(HidKeyCode::Kc1);
        state.handle_key(HidKeyCode::Kc2);
        assert_eq!(state.handle_key(HidKeyCode::Escape), PasskeyAction::Cancelled);
        assert_eq!(state.digit_count(), 0);
    }

    #[test]
    fn test_handle_key_backspace() {
        let mut state = PasskeyEntryState::new();
        state.handle_key(HidKeyCode::Kc1);
        state.handle_key(HidKeyCode::Kc2);
        assert_eq!(state.handle_key(HidKeyCode::Backspace), PasskeyAction::Backspaced);
        assert_eq!(state.digit_count(), 1);
        // Backspace on empty
        state.handle_key(HidKeyCode::Backspace);
        assert_eq!(state.handle_key(HidKeyCode::Backspace), PasskeyAction::Ignored);
    }

    #[test]
    fn test_handle_key_buffer_full() {
        let mut state = PasskeyEntryState::new();
        for d in [
            HidKeyCode::Kc1,
            HidKeyCode::Kc2,
            HidKeyCode::Kc3,
            HidKeyCode::Kc4,
            HidKeyCode::Kc5,
            HidKeyCode::Kc6,
        ] {
            state.handle_key(d);
        }
        assert_eq!(state.handle_key(HidKeyCode::Kc7), PasskeyAction::BufferFull);
    }

    #[test]
    fn test_handle_key_ignored() {
        let mut state = PasskeyEntryState::new();
        assert_eq!(state.handle_key(HidKeyCode::A), PasskeyAction::Ignored);
    }

    #[test]
    fn test_handle_key_kp_enter() {
        let mut state = PasskeyEntryState::new();
        for d in [
            HidKeyCode::Kp1,
            HidKeyCode::Kp2,
            HidKeyCode::Kp3,
            HidKeyCode::Kp4,
            HidKeyCode::Kp5,
            HidKeyCode::Kp6,
        ] {
            state.handle_key(d);
        }
        assert_eq!(state.handle_key(HidKeyCode::KpEnter), PasskeyAction::Submitted(123456));
    }

    #[test]
    fn test_hid_keycode_to_digit_all() {
        let keycodes = [
            (HidKeyCode::Kc0, 0),
            (HidKeyCode::Kc1, 1),
            (HidKeyCode::Kc2, 2),
            (HidKeyCode::Kc3, 3),
            (HidKeyCode::Kc4, 4),
            (HidKeyCode::Kc5, 5),
            (HidKeyCode::Kc6, 6),
            (HidKeyCode::Kc7, 7),
            (HidKeyCode::Kc8, 8),
            (HidKeyCode::Kc9, 9),
            (HidKeyCode::Kp0, 0),
            (HidKeyCode::Kp1, 1),
            (HidKeyCode::Kp2, 2),
            (HidKeyCode::Kp3, 3),
            (HidKeyCode::Kp4, 4),
            (HidKeyCode::Kp5, 5),
            (HidKeyCode::Kp6, 6),
            (HidKeyCode::Kp7, 7),
            (HidKeyCode::Kp8, 8),
            (HidKeyCode::Kp9, 9),
        ];
        for (kc, expected) in keycodes {
            assert_eq!(hid_keycode_to_digit(kc), Some(expected), "Failed for {:?}", kc);
        }
        assert_eq!(hid_keycode_to_digit(HidKeyCode::A), None);
    }

    #[test]
    fn test_passkey_session_begin_order_allows_immediate_response() {
        // Simulate stale data from a previous session.
        PASSKEY_RESPONSE.signal(Some(111111));

        begin_passkey_entry_session();
        assert!(PASSKEY_ENTRY_MODE.load(Ordering::Acquire));

        // Simulate immediate keyboard response right after session begin.
        PASSKEY_RESPONSE.signal(Some(222222));
        let got = block_on(async { PASSKEY_RESPONSE.wait().await });
        assert_eq!(got, Some(222222));

        end_passkey_entry_session();
        assert!(!PASSKEY_ENTRY_MODE.load(Ordering::Acquire));
    }

    #[test]
    fn test_passkey_cleanup_guard_clears_active_mode_on_drop() {
        begin_passkey_entry_session();
        assert!(PASSKEY_ENTRY_MODE.load(Ordering::Acquire));

        {
            let _guard = PasskeyCleanupGuard::new();
            assert!(PASSKEY_ENTRY_MODE.load(Ordering::Acquire));
        }

        assert!(!PASSKEY_ENTRY_MODE.load(Ordering::Acquire));
    }
}
