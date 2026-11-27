use rmk_types::action::KeyAction;
use rmk_types::combo::{ComboDefinition, MatrixPosition};
use rmk_types::constants::MAX_COMBO_KEYS;

/// Combo config instantiated with firmware's combo Vec capacity.
pub type ComboConfig = rmk_types::combo::Combo;
/// Position-combo config instantiated with firmware's combo Vec capacity.
pub type PositionComboConfig = rmk_types::combo::PositionCombo;

use crate::event::{KeyboardEvent, KeyboardEventSource};

// Combo.state is a u16 bitmask, so combos are limited to 16 keys.
// Use core::assert! explicitly — the crate-level `assert!` macro dispatches to
// defmt::assert! which is not const-compatible.
const _: () = core::assert!(
    MAX_COMBO_KEYS <= 16,
    "MAX_COMBO_KEYS exceeds 16 — Combo.state is u16 and cannot track more than 16 keys"
);

/// Runtime combo instance (config + runtime state)
#[derive(Clone, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct Combo {
    pub(crate) definition: ComboDefinition,
    /// The state records the pressed keys of the combo
    state: u16,
    /// The flag indicates whether the combo is triggered
    is_triggered: bool,
}

impl Default for Combo {
    fn default() -> Self {
        Self::empty()
    }
}

impl Combo {
    pub fn new(config: ComboConfig) -> Self {
        Self::from_definition(ComboDefinition::Actions(config))
    }

    pub fn new_positions(config: PositionComboConfig) -> Self {
        Self::from_definition(ComboDefinition::Positions(config))
    }

    pub(crate) fn from_definition(definition: ComboDefinition) -> Self {
        Self {
            definition,
            state: 0,
            is_triggered: false,
        }
    }

    pub fn empty() -> Self {
        Self::new(ComboConfig::empty())
    }

    pub(crate) fn definition(&self) -> ComboDefinition {
        self.definition.clone()
    }

    pub(crate) fn legacy_config(&self) -> Option<&ComboConfig> {
        match &self.definition {
            ComboDefinition::Actions(config) => Some(config),
            ComboDefinition::Positions(_) => None,
        }
    }

    fn layer(&self) -> Option<u8> {
        self.definition.layer()
    }

    pub(crate) fn output(&self) -> KeyAction {
        self.definition.output()
    }

    fn definition_input_index(
        definition: &ComboDefinition,
        key_action: &KeyAction,
        position: KeyboardEventSource,
    ) -> Option<usize> {
        match definition {
            ComboDefinition::Actions(config) => config.find_key_action_index(key_action),
            ComboDefinition::Positions(config) => match position {
                KeyboardEventSource::Key(position) => config.find_position_index(&MatrixPosition {
                    row: position.row,
                    col: position.col,
                }),
                KeyboardEventSource::RotaryEncoder(_)
                | KeyboardEventSource::Combo(_)
                | KeyboardEventSource::Macro
                | KeyboardEventSource::Virtual(_)
                | KeyboardEventSource::Touch(_)
                | KeyboardEventSource::KeyCode(_) => None,
            },
        }
    }

    fn input_index(&self, key_action: &KeyAction, position: KeyboardEventSource) -> Option<usize> {
        Self::definition_input_index(&self.definition, key_action, position)
    }

    pub(crate) fn definition_contains_input(
        definition: &ComboDefinition,
        key_action: &KeyAction,
        position: KeyboardEventSource,
    ) -> bool {
        Self::definition_input_index(definition, key_action, position).is_some()
    }

    pub(crate) fn contains_input(&self, key_action: &KeyAction, position: KeyboardEventSource) -> bool {
        self.input_index(key_action, position).is_some()
    }

    /// Update the combo's state when a key is pressed.
    /// Returns true if the combo is updated.
    pub(crate) fn update(&mut self, key_action: &KeyAction, key_event: KeyboardEvent, active_layer: u8) -> bool {
        if !key_event.pressed || self.size() == 0 || self.is_triggered {
            // Ignore combo that without actions
            return false;
        }

        if let Some(layer) = self.layer()
            && layer != active_layer
        {
            return false;
        }

        let action_idx = self.input_index(key_action, key_event.source);
        if let Some(i) = action_idx {
            self.state |= 1 << i;
        } else if !self.is_all_pressed() {
            self.reset();
        }
        action_idx.is_some()
    }

    /// Re-assert a combo key's bit in the state of an already-triggered combo.
    ///
    /// Covers the case where the user releases one key of a held chord and presses
    /// it again while the other combo key is still down. The re-press must not
    /// leak to HID (it would overwrite the combo output's slot), and the eventual
    /// release must still complete the combo — so we re-set the bit here.
    ///
    /// Returns true iff this combo is triggered and the action/position pair
    /// matches one of its inputs, i.e. the caller should swallow the press.
    pub(crate) fn reassert_if_triggered(&mut self, key_action: &KeyAction, position: KeyboardEventSource) -> bool {
        if !self.is_triggered {
            return false;
        }
        if let Some(i) = self.input_index(key_action, position) {
            self.state |= 1 << i;
            return true;
        }
        false
    }

    /// Update the combo's state when a key is released
    /// When the combo is fully released from triggered state, this function returns true
    pub(crate) fn update_released(&mut self, key_action: &KeyAction, position: KeyboardEventSource) -> bool {
        if let Some(i) = self.input_index(key_action, position) {
            self.state &= !(1 << i);
        }

        // Reset the combo if all keys are released
        if self.state == 0 {
            if self.is_triggered {
                self.reset();
                return true;
            }
            self.reset();
        }
        false
    }

    /// Mark the combo as done, if all actions are satisfied
    pub(crate) fn trigger(&mut self) -> KeyAction {
        if self.is_triggered() {
            return self.output();
        }

        if self.is_all_pressed() {
            self.is_triggered = true;
        }
        self.output()
    }

    /// Fully pressed but not fired yet.
    pub(crate) fn is_pending(&self) -> bool {
        self.is_all_pressed() && !self.is_triggered()
    }

    /// Check if the combo is dispatched into key event
    pub(crate) fn is_triggered(&self) -> bool {
        self.is_triggered
    }

    /// Check if all keys of this combo are pressed, but it does not mean the combo key event is sent
    pub(crate) fn is_all_pressed(&self) -> bool {
        let cnt = self.size();
        cnt > 0 && self.keys_pressed() == cnt as u32
    }

    /// The size of the current combo
    pub(crate) fn size(&self) -> usize {
        self.definition.size()
    }

    pub(crate) fn keys_pressed(&self) -> u32 {
        self.state.count_ones()
    }

    pub(crate) fn reset(&mut self) {
        self.state = 0;
        self.is_triggered = false;
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::action::Action;
    use rmk_types::keycode::{HidKeyCode, KeyCode};

    use super::*;

    fn hid(k: HidKeyCode) -> KeyAction {
        KeyAction::Single(Action::Key(KeyCode::Hid(k)))
    }

    // A combo whose output is empty (`KC_NO`) must still mark itself triggered
    // once all its keys are pressed. `is_triggered` is what makes the release
    // path consume the combo keys' releases (see `process_combo`). Without it a
    // combo swallows the presses but forwards the releases, leaving a stateful
    // key such as a mouse wheel with an unpaired release that repeats forever.
    #[test]
    fn empty_output_combo_still_triggers_so_releases_are_consumed() {
        let mut combo = Combo::new(ComboConfig::new(
            [hid(HidKeyCode::A), hid(HidKeyCode::B)],
            KeyAction::No,
            None,
        ));
        let a = hid(HidKeyCode::A);
        let b = hid(HidKeyCode::B);

        assert!(combo.update(&a, KeyboardEvent::key(0, 0, true), 0));
        assert!(combo.update(&b, KeyboardEvent::key(0, 1, true), 0));
        assert!(combo.is_all_pressed());

        let output = combo.trigger();
        assert_eq!(output, KeyAction::No, "empty-output combo still emits nothing");
        assert!(
            combo.is_triggered(),
            "a KC_NO combo must trigger so the release path consumes the releases"
        );
    }

    #[test]
    fn position_combo_tracks_duplicate_actions_by_coordinate() {
        let action = hid(HidKeyCode::A);
        let mut combo = Combo::new_positions(PositionComboConfig::new(
            [MatrixPosition { row: 0, col: 0 }, MatrixPosition { row: 0, col: 1 }],
            hid(HidKeyCode::B),
            None,
        ));

        assert!(combo.update(&action, KeyboardEvent::key(0, 0, true), 0));
        assert!(!combo.is_all_pressed());
        assert!(combo.update(&action, KeyboardEvent::key(0, 1, true), 1));
        assert!(
            combo.is_all_pressed(),
            "unscoped position matching ignores layer changes"
        );
    }

    #[test]
    fn position_combo_layer_scope_gates_recording() {
        let action = hid(HidKeyCode::A);
        let mut combo = Combo::new_positions(PositionComboConfig::new(
            [MatrixPosition { row: 0, col: 0 }],
            hid(HidKeyCode::B),
            Some(0),
        ));

        assert!(!combo.update(&action, KeyboardEvent::key(0, 0, true), 1));
        assert!(!combo.is_all_pressed());
        assert!(combo.update(&action, KeyboardEvent::key(0, 0, true), 0));
        assert!(combo.is_all_pressed());
    }
}
