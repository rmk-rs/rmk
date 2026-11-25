//! Combo configuration types shared between firmware and protocol layers.

use heapless::Vec;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::action::KeyAction;
use crate::constants::COMBO_SIZE;

/// Configuration data for a combo.
///
/// A combo triggers an output action when a set of keys are pressed simultaneously.
/// The maximum number of trigger keys is determined by `COMBO_SIZE` (from `constants.rs`,
/// generated at build time from `keyboard.toml` on firmware or fixed upper bound on host).
/// Actions are stored in a Vec — only meaningful keys are present (no `KeyAction::No` padding).
///
/// Note: `COMBO_SIZE` is a **wire-format** capacity — on firmware it equals
/// `MAX_COMBO_KEYS` (from `keyboard.toml`), on host it's a fixed upper bound.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct Combo {
    #[cfg_attr(feature = "wasm", tsify(type = "KeyAction[]"))]
    pub actions: Vec<KeyAction, COMBO_SIZE>,
    pub output: KeyAction,
    /// Bitmask of layers the combo is valid on. `u32::MAX` means all layers.
    pub layers: u32,
}

impl MaxSize for Combo {
    const POSTCARD_MAX_SIZE: usize =
        crate::heapless_vec_max_size::<KeyAction, COMBO_SIZE>() + KeyAction::POSTCARD_MAX_SIZE + u32::POSTCARD_MAX_SIZE;
}

impl Combo {
    /// Create a new combo from an iterator of key actions.
    ///
    /// Actions equal to `KeyAction::No` are filtered out. If there are more
    /// non-No actions than `COMBO_SIZE`, excess actions are silently dropped.
    pub fn new<I: IntoIterator<Item = KeyAction>>(actions: I, output: KeyAction, layers: u32) -> Self {
        let mut combo_actions = Vec::new();
        for action in actions {
            if action != KeyAction::No && combo_actions.push(action).is_err() {
                break;
            }
        }
        Self {
            actions: combo_actions,
            output,
            layers,
        }
    }

    /// Get an empty combo.
    pub fn empty() -> Self {
        Self {
            actions: Vec::new(),
            output: KeyAction::No,
            layers: u32::MAX,
        }
    }

    /// Returns the number of key actions in the combo.
    pub fn size(&self) -> usize {
        self.actions.len()
    }

    /// Find the index of a key action in the combo.
    pub fn find_key_action_index(&self, key_action: &KeyAction) -> Option<usize> {
        self.actions.iter().position(|a| a == key_action)
    }

    /// Check whether the combo contains the given key action.
    pub fn contains(&self, key_action: &KeyAction) -> bool {
        self.actions.contains(key_action)
    }
}
