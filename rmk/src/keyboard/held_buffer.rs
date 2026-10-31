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
    #[cfg(feature = "buffered_tap_hold")]
    next_order: u64,
    #[cfg(feature = "buffered_tap_hold")]
    pub(crate) admission: Option<u64>,
}

impl HeldBuffer {
    /// Create a new held buffer
    pub fn new() -> Self {
        Self {
            keys: heapless::Vec::new(),
            #[cfg(feature = "buffered_tap_hold")]
            next_order: 0,
            #[cfg(feature = "buffered_tap_hold")]
            admission: None,
        }
    }

    /// Push a new held key into the buffer
    pub fn push(&mut self, key: HeldKey) {
        #[cfg(feature = "buffered_tap_hold")]
        if key.capture.is_none()
            && let Some(index) = self.keys.iter().position(|entry| {
                entry.event.source == key.event.source
                    && entry
                        .capture
                        .is_some_and(|capture| Some(capture.down.order) == self.admission)
            })
        {
            let mut capture = self.keys[index].capture;
            if key.state == KeyState::WaitingCombo {
                capture.as_mut().unwrap().eligible = false;
            }
            self.keys[index] = key;
            self.keys[index].capture = capture;
            return;
        }
        if let Err(e) = self.keys.push(key) {
            error!("Held buffer overflowed, cannot save: {:?}", e);
        }
    }

    /// Find a held key by the key action
    pub fn find_action(&self, action: &KeyAction) -> Option<&HeldKey> {
        self.keys.iter().find(|x| x.action == *action)
    }

    /// Find a held key by the KeyboardEventSource
    pub fn find_source(&self, source: KeyboardEventSource) -> Option<&HeldKey> {
        self.keys.iter().find(|x| x.event.source == source && x.is_executing())
    }

    /// Find a mutable held key by the KeyboardEventSource
    pub fn find_source_mut(&mut self, source: KeyboardEventSource) -> Option<&mut HeldKey> {
        self.keys
            .iter_mut()
            .find(|x| x.event.source == source && x.is_executing())
    }

    /// Remove a held key from the buffer, keep the order
    pub fn remove_if<P>(&mut self, mut predicate: P) -> Option<HeldKey>
    where
        P: FnMut(&HeldKey) -> bool,
    {
        if let Some(i) = self.keys.iter().position(|key| key.is_executing() && predicate(key)) {
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

#[cfg(feature = "buffered_tap_hold")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionError {
    Full,
}

#[cfg(feature = "buffered_tap_hold")]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct EventStamp {
    pub at: Instant,
    pub order: u64,
}

#[cfg(feature = "buffered_tap_hold")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingEvent {
    pub generation: u64,
    pub event: KeyboardEvent,
    pub at: Instant,
    pub order: u64,
}

#[cfg(feature = "buffered_tap_hold")]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct Capture {
    pub down: EventStamp,
    pub up: Option<EventStamp>,
    pub press_pending: bool,
    pub release_pending: bool,
    pub admission_action: KeyAction,
    pub last_simple_before: Instant,
    pub binding: Option<KeyAction>,
    pub fork_bound: bool,
    pub eligible: bool,
    pub consumed: bool,
    pub policy: Option<OrderedPolicy>,
    pub observed_through: u64,
}

#[cfg(feature = "buffered_tap_hold")]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct OrderedPolicy {
    pub mode: rmk_types::morse::MorseMode,
    pub unilateral: bool,
    pub quick_tap_ms: u16,
}

#[cfg(feature = "buffered_tap_hold")]
impl HeldBuffer {
    pub(crate) fn generation_index(&self, generation: u64) -> Option<usize> {
        self.keys
            .iter()
            .position(|key| key.capture.is_some_and(|capture| capture.down.order == generation))
    }

    /// Reserve ownership before any action or combo side effect. Releases use
    /// the reservation made by their press, including when the store is full.
    pub(crate) fn admit(
        &mut self,
        event: KeyboardEvent,
        action: KeyAction,
        at: Instant,
        last_simple_before: Instant,
    ) -> Result<Option<u64>, AdmissionError> {
        let open = self.keys.iter().position(|key| {
            key.event.source == event.source && key.capture.is_some_and(|capture| capture.up.is_none())
        });
        if event.pressed && open.is_some() || !event.pressed && open.is_none() {
            return Ok(None);
        }
        if event.pressed && self.keys.is_full() {
            return Err(AdmissionError::Full);
        }
        self.next_order = self.next_order.checked_add(1).expect("input order exhausted");
        let stamp = EventStamp {
            at,
            order: self.next_order,
        };
        if let Some(index) = open {
            let capture = self.keys[index].capture.as_mut().unwrap();
            capture.up = Some(stamp);
            capture.release_pending = true;
            Ok(Some(capture.down.order))
        } else {
            let mut key = HeldKey::new(event, action, KeyState::Deferred, at, at);
            key.capture = Some(Capture {
                down: stamp,
                up: None,
                press_pending: true,
                release_pending: false,
                admission_action: action,
                last_simple_before,
                binding: None,
                fork_bound: false,
                eligible: true,
                consumed: false,
                policy: None,
                observed_through: stamp.order,
            });
            self.keys.push(key).map_err(|_| AdmissionError::Full)?;
            Ok(Some(stamp.order))
        }
    }

    pub(crate) fn next_pending_event(&self) -> Option<PendingEvent> {
        self.keys
            .iter()
            .filter_map(|key| {
                let capture = key.capture?;
                if !capture.eligible || capture.consumed {
                    return None;
                }
                let (stamp, pressed) = if capture.press_pending {
                    (capture.down, true)
                } else if capture.release_pending {
                    (capture.up?, false)
                } else {
                    return None;
                };
                Some(PendingEvent {
                    generation: capture.down.order,
                    event: KeyboardEvent {
                        pressed,
                        source: key.event.source,
                    },
                    at: stamp.at,
                    order: stamp.order,
                })
            })
            .min_by_key(|pending| pending.order)
    }

    pub(crate) fn complete_pending_event(&mut self, event: PendingEvent) {
        if let Some(capture) = self
            .keys
            .iter_mut()
            .filter_map(|key| key.capture.as_mut())
            .find(|capture| capture.down.order == event.generation)
        {
            if event.event.pressed {
                capture.press_pending = false;
            } else {
                capture.release_pending = false;
            }
        }
    }
}

/// The state of a held key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyState {
    #[cfg(feature = "buffered_tap_hold")]
    Deferred,
    #[cfg(feature = "buffered_tap_hold")]
    OrderedPending,
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
    #[cfg(feature = "buffered_tap_hold")]
    pub(crate) capture: Option<Capture>,
}

impl HeldKey {
    fn is_executing(&self) -> bool {
        #[cfg(feature = "buffered_tap_hold")]
        if self
            .capture
            .is_some_and(|capture| capture.press_pending || capture.consumed)
        {
            return false;
        }
        true
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
            #[cfg(feature = "buffered_tap_hold")]
            capture: None,
        }
    }
}

#[cfg(all(test, feature = "buffered_tap_hold"))]
mod tests {
    use embassy_time::Instant;
    use rmk_types::action::KeyAction;

    use super::{AdmissionError, HeldBuffer};
    use crate::event::KeyboardEvent;

    #[test]
    fn a_repress_keeps_the_previous_generations_pending_release() {
        let mut buffer = HeldBuffer::new();
        let at = Instant::from_millis(100);
        let down = KeyboardEvent::key(0, 0, true);
        let up = KeyboardEvent::key(0, 0, false);
        let first = buffer.admit(down, KeyAction::No, at, at).unwrap().unwrap();
        assert_eq!(buffer.admit(down, KeyAction::No, at, at), Ok(None));
        assert_eq!(buffer.admit(up, KeyAction::No, at, at), Ok(Some(first)));
        let second = buffer.admit(down, KeyAction::No, at, at).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(buffer.admit(up, KeyAction::No, at, at), Ok(Some(second)));
        assert_eq!(buffer.admit(up, KeyAction::No, at, at), Ok(None));

        for (generation, event) in [(first, down), (first, up), (second, down), (second, up)] {
            let pending = buffer.next_pending_event().unwrap();
            assert_eq!(pending.generation, generation);
            assert_eq!(pending.event, event);
            assert_eq!(pending.at, at);
            buffer.complete_pending_event(pending);
        }
        assert!(buffer.next_pending_event().is_none());
    }

    #[test]
    fn a_full_store_can_capture_every_admitted_release() {
        let mut buffer = HeldBuffer::new();
        let at = Instant::from_millis(100);
        for col in 0..16 {
            buffer
                .admit(KeyboardEvent::key(0, col, true), KeyAction::No, at, at)
                .unwrap();
        }
        assert_eq!(
            buffer.admit(KeyboardEvent::key(0, 16, true), KeyAction::No, at, at),
            Err(AdmissionError::Full)
        );
        assert_eq!(
            buffer.admit(KeyboardEvent::key(0, 16, false), KeyAction::No, at, at),
            Ok(None)
        );
        for col in 0..16 {
            assert!(
                buffer
                    .admit(KeyboardEvent::key(0, col, false), KeyAction::No, at, at)
                    .unwrap()
                    .is_some()
            );
        }
        let mut events = 0;
        while let Some(pending) = buffer.next_pending_event() {
            buffer.complete_pending_event(pending);
            events += 1;
        }
        assert_eq!(events, 32);
    }
}
