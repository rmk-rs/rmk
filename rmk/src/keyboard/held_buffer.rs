use embassy_time::Instant;
use rmk_types::action::{Action, KeyAction};
use rmk_types::morse::MorsePattern;

use crate::event::{KeyboardEvent, KeyboardEventPos};

/// Held keys ordered by their current press or Morse transition time.
#[derive(Debug, Default, Clone)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct HeldBuffer {
    // TODO: Make the buffer size configurable
    pub(crate) keys: heapless::Vec<HeldKey, 16>,
}

impl HeldBuffer {
    /// Create a new held buffer
    pub fn new() -> Self {
        Self {
            keys: heapless::Vec::new(),
        }
    }

    /// Insert a held key in time order, retaining the order of equal timestamps.
    pub fn push(&mut self, key: HeldKey) {
        if let Err(e) = self.keys.push(key) {
            error!("Held buffer overflowed, cannot save: {:?}", e);
            return;
        }
        self.reposition(self.keys.len() - 1);
    }

    pub(crate) fn update_press_time(&mut self, pos: KeyboardEventPos, at: Instant) {
        if let Some(index) = self.keys.iter().position(|key| key.event.pos == pos) {
            self.keys[index].press_time = at;
            self.reposition(index);
        }
    }

    /// Only the inserted or updated entry can be out of order.
    fn reposition(&mut self, mut index: usize) {
        let key = self.keys[index];
        while index > 0 && self.keys[index - 1].press_time > key.press_time {
            self.keys[index] = self.keys[index - 1];
            index -= 1;
        }
        while index + 1 < self.keys.len() && self.keys[index + 1].press_time < key.press_time {
            self.keys[index] = self.keys[index + 1];
            index += 1;
        }
        self.keys[index] = key;
    }

    /// Find a held key by the key action
    pub fn find_action(&self, action: &KeyAction) -> Option<&HeldKey> {
        self.keys.iter().find(|x| x.action == *action)
    }

    /// Find a held key by the KeyboardEventPos
    pub fn find_pos(&self, pos: KeyboardEventPos) -> Option<&HeldKey> {
        self.keys.iter().find(|x| x.event.pos == pos)
    }

    /// Find a mutable held key by the KeyboardEventPos
    pub fn find_pos_mut(&mut self, pos: KeyboardEventPos) -> Option<&mut HeldKey> {
        self.keys.iter_mut().find(|x| x.event.pos == pos)
    }

    /// Remove a held key from the buffer, keep the order
    pub fn remove_if<P>(&mut self, predicate: P) -> Option<HeldKey>
    where
        P: FnMut(&HeldKey) -> bool,
    {
        if let Some(i) = self.keys.iter().position(predicate) {
            Some(self.keys.remove(i))
        } else {
            None
        }
    }

    /// Get the key with the earliest timeout among those matching `predicate`.
    pub fn next_timeout<P>(&self, mut predicate: P) -> Option<HeldKey>
    where
        P: FnMut(&HeldKey) -> bool,
    {
        self.keys
            .iter()
            .filter(|k| predicate(k))
            .min_by_key(|k| k.timeout_time)
            .copied()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// The state of a held key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyState {
    /// The current key is a component of a combo, and it's waiting for other combo components
    WaitingCombo,

    /// After a press event is received.
    /// The data represents the previously completed morse pattern
    Pressed(MorsePattern),

    /// After a press event is received and the hold timeout is reached.
    /// The data represents the previously completed morse pattern
    /// including the current hold
    Holding(MorsePattern),

    /// After a release event is received for a key still kept in the HeldBuffer - so morse pattern may continue
    /// The data represents the already completed morse pattern
    Released(MorsePattern),

    /// After a tap has been fired early (early fire optimization), but the key
    /// remains in the buffer to allow hold_after_tap continuation.
    EarlyFired(MorsePattern),

    /// After flow-tap resolved the key as a tap: the tap action's press HID report
    /// is sent and held while the key is physically held. On release the action is
    /// released and the key is kept as `EarlyFired` so hold_after_tap can continue.
    FlowTapped(Action),

    /// The corresponding action is already executed (so the Pressed HID report is sent),
    /// but the release HID report is not sent yet (will be sent only when the corresponding
    /// key is really released).
    ProcessedButReleaseNotReportedYet(Action),
    // The Idle state is represented by the removal from the HeldBuffer
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct HeldKey {
    pub event: KeyboardEvent,
    pub action: KeyAction,
    /// Current state of the held key
    pub state: KeyState,
    /// Press time, or release time while waiting for another Morse tap.
    press_time: Instant,
    /// The timeout time for the key
    pub timeout_time: Instant,
}

impl HeldKey {
    pub fn press_time(&self) -> Instant {
        self.press_time
    }

    pub fn new(
        event: KeyboardEvent,
        action: KeyAction,
        state: KeyState,
        press_time: Instant,
        timeout_time: Instant,
    ) -> Self {
        Self {
            event,
            action,
            state,
            press_time,
            timeout_time,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_matches_reference(buffer: &HeldBuffer, expected: &[HeldKey]) {
        assert_eq!(buffer.keys.len(), expected.len());
        for (actual, expected) in buffer.keys.iter().zip(expected) {
            assert_eq!(
                (
                    actual.event,
                    actual.action,
                    actual.state,
                    actual.press_time,
                    actual.timeout_time
                ),
                (
                    expected.event,
                    expected.action,
                    expected.state,
                    expected.press_time,
                    expected.timeout_time
                ),
            );
        }
        assert_eq!(
            buffer.next_timeout(|_| true).map(|key| key.event),
            expected.iter().min_by_key(|key| key.timeout_time).map(|key| key.event),
        );
    }

    #[test]
    fn ordered_buffer_keeps_order_through_insert_update_remove() {
        for seed in 0..256 {
            let mut buffer = HeldBuffer::new();
            let mut expected: heapless::Vec<HeldKey, 16> = heapless::Vec::new();
            for id in 0..16 {
                let key = HeldKey::new(
                    KeyboardEvent::key(0, id, true),
                    KeyAction::Single(Action::LayerOn(id)),
                    KeyState::WaitingCombo,
                    Instant::from_ticks(((seed >> (id % 8)) & 3) as u64),
                    Instant::from_ticks(100 - id as u64),
                );
                buffer.push(key);
                expected.push(key).unwrap();
                expected.sort_by_key(|key| key.press_time);
                assert_matches_reference(&buffer, &expected);
            }
            let overflow = HeldKey::new(
                KeyboardEvent::key(0, 255, true),
                KeyAction::No,
                KeyState::WaitingCombo,
                Instant::MIN,
                Instant::MIN,
            );
            buffer.push(overflow);
            buffer.update_press_time(overflow.event.pos, Instant::MAX);
            assert_matches_reference(&buffer, &expected);
            for id in 0..16 {
                let pos = KeyboardEvent::key(0, id, true).pos;
                for ticks in [0, 1, 3, 17, u64::MAX] {
                    let at = Instant::from_ticks(ticks);
                    buffer.update_press_time(pos, at);
                    expected.iter_mut().find(|key| key.event.pos == pos).unwrap().press_time = at;
                    expected.sort_by_key(|key| key.press_time);
                    assert_matches_reference(&buffer, &expected);
                }
            }
            for id in 0..16 {
                let pos = KeyboardEvent::key(0, id, true).pos;
                assert!(buffer.remove_if(|key| key.event.pos == pos).is_some());
                expected.retain(|key| key.event.pos != pos);
                assert_matches_reference(&buffer, &expected);
            }
        }
    }
}
