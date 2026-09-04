use embassy_time::Instant;
use rmk_types::action::{Action, KeyAction};
use rmk_types::morse::MorsePattern;

use crate::event::{KeyboardEvent, KeyboardEventPos};

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

    /// Push a new held key into the buffer and then sort by the timeout
    pub fn push(&mut self, key: HeldKey) {
        if let Err(e) = self.keys.push(key) {
            error!("Held buffer overflowed, cannot save: {:?}", e);
        }

        // Sort the buffer after push
        self.keys.sort_unstable_by_key(|k| k.timeout_time);
    }

    /// Push a new held key into the buffer
    pub fn push_without_sort(&mut self, key: HeldKey) {
        if let Err(e) = self.keys.push(key) {
            error!("Held buffer overflowed, cannot save: {:?}", e);
        }
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

    /// Remove a held key from the buffer and then resort the buffer
    pub fn remove(&mut self, pos: KeyboardEventPos) -> Option<HeldKey> {
        let k = self.remove_if(|k| k.event.pos == pos);
        // Re-sort the buffer after remove
        self.keys.sort_unstable_by_key(|k| k.timeout_time);
        k
    }

    /// Get the next timeout key in the buffer
    pub fn next_timeout<P>(&self, mut predicate: P) -> Option<HeldKey>
    where
        P: FnMut(&HeldKey) -> bool,
    {
        // Support that the held buffer is already sorted by the timeout time
        self.keys.iter().find(|&x| predicate(x)).copied()
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
    /// Stable combo-table slot when this is a buffered Sticky combo output.
    pub sticky_combo_index: Option<u16>,
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
            sticky_combo_index: None,
            state,
            press_time,
            timeout_time,
        }
    }

    pub fn with_sticky_combo_index(mut self, combo_index: u16) -> Self {
        if matches!(self.action, KeyAction::Sticky(_, _)) {
            self.sticky_combo_index = Some(combo_index);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::action::Action;
    use rmk_types::keycode::{HidKeyCode, KeyCode};
    use rmk_types::modifier::ModifierCombination;

    use super::*;

    fn buffered(action: KeyAction) -> HeldKey {
        let now = Instant::now();
        HeldKey::new(
            KeyboardEvent::key(0, 0, true),
            action,
            KeyState::Pressed(MorsePattern::default()),
            now,
            now,
        )
        .with_sticky_combo_index(7)
    }

    #[test]
    fn sticky_combo_outputs_retain_combo_identity() {
        let sticky_actions = [
            KeyAction::Sticky(Action::Modifier(ModifierCombination::LCTRL), 0),
            KeyAction::Sticky(Action::LayerOn(1), 0),
            KeyAction::Sticky(Action::KeyWithModifier(HidKeyCode::Tab, ModifierCombination::LALT), 0),
        ];
        for action in sticky_actions {
            assert_eq!(buffered(action).sticky_combo_index, Some(7));
        }
    }

    #[test]
    fn non_sticky_combo_outputs_remain_layer_sensitive() {
        let ordinary = KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A)));
        // No stored identity means buffered dispatch still reloads this
        // position from the active layer, as it did before Sticky combo
        // provenance was generalized.
        assert_eq!(buffered(ordinary).sticky_combo_index, None);
    }
}
