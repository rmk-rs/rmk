use embassy_time::Instant;
use rmk_types::action::{Action, KeyAction};
use rmk_types::morse::MorsePattern;

use crate::event::{KeyboardEvent, KeyboardEventSource};

/// The buffer of held keys.
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

    /// Push a new held key into the buffer
    pub fn push(&mut self, key: HeldKey) {
        // The event names the key; a whole `HeldKey` would link its formatter just for this error.
        if let Err(e) = self.keys.push(key) {
            error!("Held buffer overflowed, cannot save: {:?}", e.event);
        }
    }

    /// Find a held key by the key action
    pub fn find_action(&self, action: &KeyAction) -> Option<&HeldKey> {
        self.keys.iter().find(|x| x.action == *action)
    }

    /// Find a held key by the KeyboardEventSource
    pub fn find_source(&self, source: KeyboardEventSource) -> Option<&HeldKey> {
        self.keys.iter().find(|x| x.event.source == source)
    }

    /// Find a mutable held key by the KeyboardEventSource
    pub fn find_source_mut(&mut self, source: KeyboardEventSource) -> Option<&mut HeldKey> {
        self.keys.iter_mut().find(|x| x.event.source == source)
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

    /// Sort keys by press time, keeping the order of keys pressed at the same instant.
    ///
    /// `sort_unstable_by_key` runs this same insertion sort on slices this short, but
    /// still links a full quicksort for every call site.
    pub fn sort_by_press_time(&mut self) {
        for i in 1..self.keys.len() {
            let mut j = i;
            while j > 0 && self.keys[j - 1].press_time > self.keys[j].press_time {
                self.keys.swap(j - 1, j);
                j -= 1;
            }
        }
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
    /// The press time for the key
    pub press_time: Instant,
    /// The timeout time for the key
    pub timeout_time: Instant,
}

impl HeldKey {
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
