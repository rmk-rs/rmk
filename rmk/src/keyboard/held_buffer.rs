use core::num::NonZeroU64;

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
    next_order: u64,
}

impl HeldBuffer {
    /// Create a new held buffer
    pub fn new() -> Self {
        Self {
            keys: heapless::Vec::new(),
            next_order: 0,
        }
    }

    /// Push a new held key into the buffer
    pub fn push(&mut self, key: HeldKey) {
        if let Err(e) = self.keys.push(key) {
            error!("Held buffer overflowed, cannot save: {:?}", e);
        }
    }

    /// Find a held key by the key action
    pub fn find_action(&self, action: &KeyAction) -> Option<&HeldKey> {
        self.keys.iter().find(|x| x.action == *action)
    }

    /// Find an executing held key by source (compatibility alias).
    pub fn find_source(&self, source: KeyboardEventSource) -> Option<&HeldKey> {
        self.find_executing_source(source)
    }

    /// Find an executing held key, excluding queued and consumed generations.
    pub fn find_executing_source(&self, source: KeyboardEventSource) -> Option<&HeldKey> {
        self.keys.iter().find(|x| x.event.source == source && x.is_executing())
    }

    /// Find a mutable executing held key by source (compatibility alias).
    pub fn find_source_mut(&mut self, source: KeyboardEventSource) -> Option<&mut HeldKey> {
        self.find_executing_source_mut(source)
    }

    /// Find a mutable executing key, excluding queued and consumed generations.
    pub fn find_executing_source_mut(&mut self, source: KeyboardEventSource) -> Option<&mut HeldKey> {
        self.keys
            .iter_mut()
            .find(|x| x.event.source == source && x.is_executing())
    }

    /// Remove an executing held key, preserving order (compatibility alias).
    pub fn remove_if<P>(&mut self, predicate: P) -> Option<HeldKey>
    where
        P: FnMut(&HeldKey) -> bool,
    {
        self.remove_executing_if(predicate)
    }

    /// Remove a matching executing key, excluding queued and consumed generations.
    pub fn remove_executing_if<P>(&mut self, mut predicate: P) -> Option<HeldKey>
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

    pub(crate) fn sort_by_press_time(&mut self) {
        for i in 1..self.keys.len() {
            let mut j = i;
            while j > 0 && self.keys[j - 1].press_time > self.keys[j].press_time {
                self.keys.swap(j - 1, j);
                j -= 1;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdmissionError {
    Full,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct EventStamp {
    pub at: Instant,
    pub order: EventOrder,
}

/// Capture order starts at one, leaving zero available for absent edges.
pub(crate) type EventOrder = NonZeroU64;

#[derive(Clone, Copy, Debug)]
pub(crate) struct PendingEvent {
    pub generation: EventOrder,
    pub event: KeyboardEvent,
    pub at: Instant,
    pub order: EventOrder,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum DeliveryPhase {
    Queued,
    PressDelivered,
    ReleaseDelivered,
    Consumed,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct Capture {
    pub down: EventStamp,
    /// Causal delivery prerequisite; generation IDs and timestamps retain capture order.
    pub deliver_before: Option<EventOrder>,
    pub up: Option<EventStamp>,
    pub phase: DeliveryPhase,
    pub admission_action: KeyAction,
    pub last_simple_before: Instant,
    pub binding: Option<KeyAction>,
    pub fork_bound: bool,
    pub policy: Option<OrderedPolicy>,
    pub observed_through: EventOrder,
}

#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) struct OrderedPolicy {
    pub mode: rmk_types::morse::MorseMode,
    pub unilateral: bool,
    pub quick_tap_ms: u16,
}

impl HeldBuffer {
    /// Replace a validated reservation's state/action without changing its captured edges.
    pub(crate) fn replace_admitted(&mut self, generation: EventOrder, mut held: HeldKey) {
        let index = self.generation_index(generation).unwrap();
        held.capture = self.keys[index].capture;
        self.keys[index] = held;
    }

    pub(crate) fn generation_index(&self, generation: EventOrder) -> Option<usize> {
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
    ) -> Result<Option<EventOrder>, AdmissionError> {
        let open = self.keys.iter().position(|key| {
            key.event.source == event.source && key.capture.is_some_and(|capture| capture.up.is_none())
        });
        if event.pressed && open.is_some() || !event.pressed && open.is_none() {
            return Ok(None);
        }
        if event.pressed && self.keys.is_full() {
            return Err(AdmissionError::Full);
        }
        let order = self
            .next_order
            .checked_add(1)
            .and_then(EventOrder::new)
            .expect("input order exhausted");
        self.next_order = order.get();
        let stamp = EventStamp { at, order };
        if let Some(index) = open {
            let capture = self.keys[index].capture.as_mut().unwrap();
            capture.up = Some(stamp);
            Ok(Some(capture.down.order))
        } else {
            let mut key = HeldKey::new(event, action, KeyState::Deferred, at, at);
            key.capture = Some(Capture {
                down: stamp,
                deliver_before: None,
                up: None,
                phase: DeliveryPhase::Queued,
                admission_action: action,
                last_simple_before,
                binding: None,
                fork_bound: false,
                policy: None,
                observed_through: stamp.order,
            });
            self.keys.push(key).map_err(|_| AdmissionError::Full)?;
            Ok(Some(stamp.order))
        }
    }

    pub(crate) fn next_pending_event(&self) -> Option<PendingEvent> {
        let mut next = None;
        let mut best = None;
        for key in &self.keys {
            let Some(capture) = key.capture.as_ref() else { continue };
            if key.state == KeyState::WaitingCombo {
                continue;
            }
            let (stamp, pressed) = match capture.phase {
                DeliveryPhase::Queued => (capture.down, true),
                DeliveryPhase::PressDelivered => {
                    let Some(up) = capture.up else { continue };
                    (up, false)
                }
                DeliveryPhase::ReleaseDelivered | DeliveryPhase::Consumed => continue,
            };
            let delivery_key = (
                if pressed {
                    capture.deliver_before.unwrap_or(stamp.order)
                } else {
                    stamp.order
                },
                if pressed && capture.deliver_before.is_some() {
                    0u8
                } else {
                    1
                },
                stamp.order,
            );
            if best.is_none_or(|current| delivery_key < current) {
                best = Some(delivery_key);
                next = Some(PendingEvent {
                    generation: capture.down.order,
                    event: KeyboardEvent {
                        pressed,
                        source: key.event.source,
                    },
                    at: stamp.at,
                    order: stamp.order,
                });
            }
        }
        next
    }

    pub(crate) fn morse_timeout_eligible(&self, key: &HeldKey) -> bool {
        if !key.action.is_morse()
            || !matches!(
                key.state,
                KeyState::Pressed(_) | KeyState::Released(_) | KeyState::EarlyFired(_)
            )
            || key.capture.is_some_and(|capture| {
                matches!(capture.phase, DeliveryPhase::Queued | DeliveryPhase::Consumed)
                    || (capture.phase == DeliveryPhase::PressDelivered
                        && capture.up.is_some_and(|up| up.at <= key.timeout_time))
            })
        {
            return false;
        }
        let history = matches!(key.state, KeyState::EarlyFired(_))
            || (key.capture.is_none_or(|capture| capture.policy.is_none())
                && matches!(key.state, KeyState::Released(_)));
        // A captured continuation still counts while a combo or ordered owner delays it.
        !history
            || !self.keys.iter().any(|later| {
                later.event.source == key.event.source
                    && later.capture.is_some_and(|capture| {
                        capture.phase == DeliveryPhase::Queued
                            && key.capture.is_none_or(|old| capture.down.order > old.down.order)
                            && capture.down.at <= key.timeout_time
                    })
            })
    }

    pub(crate) fn complete_pending_event(&mut self, event: PendingEvent) {
        if let Some(capture) = self
            .keys
            .iter_mut()
            .filter_map(|key| key.capture.as_mut())
            .find(|capture| capture.down.order == event.generation)
        {
            capture.phase = if event.event.pressed {
                DeliveryPhase::PressDelivered
            } else {
                DeliveryPhase::ReleaseDelivered
            };
        }
    }
}

/// The state of a held key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyState {
    Deferred,
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
    pub(crate) capture: Option<Capture>,
}

impl HeldKey {
    fn is_executing(&self) -> bool {
        !self
            .capture
            .is_some_and(|capture| matches!(capture.phase, DeliveryPhase::Queued | DeliveryPhase::Consumed))
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
            capture: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use embassy_time::Instant;
    use rmk_types::action::KeyAction;

    use super::{AdmissionError, HeldBuffer};
    use crate::event::KeyboardEvent;

    #[test]
    fn held_records_fit_in_112_bytes() {
        assert!(core::mem::size_of::<super::HeldKey>() <= 112);
    }

    #[test]
    fn causal_combo_presses_precede_their_interrupt_but_not_earlier_releases() {
        let mut buffer = HeldBuffer::new();
        let at = Instant::from_millis(100);
        let first = buffer
            .admit(KeyboardEvent::key(0, 0, true), KeyAction::No, at, at)
            .unwrap()
            .unwrap();
        buffer
            .admit(KeyboardEvent::key(0, 0, false), KeyAction::No, at, at)
            .unwrap();
        buffer.keys[0].capture.as_mut().unwrap().phase = super::DeliveryPhase::PressDelivered;
        let interrupt = buffer
            .admit(KeyboardEvent::key(0, 1, true), KeyAction::No, at, at)
            .unwrap()
            .unwrap();
        let combo = buffer
            .admit(KeyboardEvent::combo(0, true), KeyAction::No, at, at)
            .unwrap()
            .unwrap();
        let other_combo = buffer
            .admit(KeyboardEvent::combo(1, true), KeyAction::No, at, at)
            .unwrap()
            .unwrap();
        for key in &mut buffer.keys[2..] {
            key.capture.as_mut().unwrap().deliver_before = Some(interrupt);
        }
        buffer.keys.swap(2, 3);
        for (generation, pressed) in [(first, false), (combo, true), (other_combo, true), (interrupt, true)] {
            let pending = buffer.next_pending_event().unwrap();
            assert_eq!(pending.generation, generation);
            assert_eq!(pending.event.pressed, pressed);
            assert_eq!(pending.at, at);
            buffer.complete_pending_event(pending);
        }
        assert!(buffer.next_pending_event().is_none());
    }

    #[test]
    fn generation_ids_keep_the_full_counter_range() {
        let mut buffer = HeldBuffer::new();
        buffer.next_order = u64::MAX - 2;
        let at = Instant::from_ticks(1);
        let generation = buffer
            .admit(KeyboardEvent::key(0, 0, true), KeyAction::No, at, at)
            .unwrap()
            .unwrap();
        assert_eq!(generation.get(), u64::MAX - 1);
        assert_eq!(
            buffer.admit(KeyboardEvent::key(0, 0, false), KeyAction::No, at, at),
            Ok(Some(generation))
        );
        assert_eq!(buffer.next_order, u64::MAX);
        let press = buffer.next_pending_event().unwrap();
        assert_eq!(press.generation, generation);
        buffer.complete_pending_event(press);
        let release = buffer.next_pending_event().unwrap();
        assert_eq!(release.generation, generation);
        assert!(release.order > press.order);
    }

    #[test]
    #[should_panic(expected = "input order exhausted")]
    fn exhausted_event_ids_do_not_wrap_and_reuse_ownership() {
        let mut buffer = HeldBuffer::new();
        buffer.next_order = u64::MAX;
        let at = Instant::from_ticks(1);
        let _ = buffer.admit(KeyboardEvent::key(0, 0, true), KeyAction::No, at, at);
    }

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
