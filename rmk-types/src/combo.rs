//! Combo configuration types shared between firmware and protocol layers.

use heapless::Vec;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::action::KeyAction;
use crate::constants::COMBO_SIZE;

/// A physical key in the keyboard's unified matrix.
///
/// Split peripherals apply their configured row/column offsets before events
/// reach combo processing, so these coordinates address the complete keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct MatrixPosition {
    pub row: u8,
    pub col: u8,
}

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
    pub layer: Option<u8>,
}

impl MaxSize for Combo {
    const POSTCARD_MAX_SIZE: usize = crate::heapless_vec_max_size::<KeyAction, COMBO_SIZE>()
        + KeyAction::POSTCARD_MAX_SIZE
        + Option::<u8>::POSTCARD_MAX_SIZE;
}

/// Configuration data for a combo triggered by physical matrix positions.
///
/// Unlike [`Combo`], matching is independent of the actions currently resolved
/// at those positions. This makes the trigger stable across keymap and layer
/// changes and lets two positions carrying the same action participate in one
/// combo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct PositionCombo {
    #[cfg_attr(feature = "wasm", tsify(type = "MatrixPosition[]"))]
    pub positions: Vec<MatrixPosition, COMBO_SIZE>,
    pub output: KeyAction,
    pub layer: Option<u8>,
}

impl MaxSize for PositionCombo {
    const POSTCARD_MAX_SIZE: usize = crate::heapless_vec_max_size::<MatrixPosition, COMBO_SIZE>()
        + KeyAction::POSTCARD_MAX_SIZE
        + Option::<u8>::POSTCARD_MAX_SIZE;
}

impl PositionCombo {
    /// Create a combo from an iterator of physical positions.
    ///
    /// If more than `COMBO_SIZE` positions are supplied, excess positions are
    /// dropped. Callers accepting untrusted configuration should reject
    /// duplicate or out-of-matrix positions before constructing the runtime
    /// combo.
    pub fn new<I: IntoIterator<Item = MatrixPosition>>(positions: I, output: KeyAction, layer: Option<u8>) -> Self {
        let mut combo_positions = Vec::new();
        for position in positions {
            if combo_positions.push(position).is_err() {
                break;
            }
        }
        Self {
            positions: combo_positions,
            output,
            layer,
        }
    }

    pub fn empty() -> Self {
        Self {
            positions: Vec::new(),
            output: KeyAction::No,
            layer: None,
        }
    }

    pub fn size(&self) -> usize {
        self.positions.len()
    }

    pub fn find_position_index(&self, position: &MatrixPosition) -> Option<usize> {
        self.positions.iter().position(|p| p == position)
    }

    pub fn contains(&self, position: &MatrixPosition) -> bool {
        self.positions.contains(position)
    }
}

/// Versioned combo representation used by additive Rynk endpoints.
///
/// The original [`Combo`] wire representation and commands remain unchanged;
/// this enum is carried only by the newer definition endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum ComboDefinition {
    Actions(Combo),
    Positions(PositionCombo),
}

impl ComboDefinition {
    pub fn empty() -> Self {
        Self::Actions(Combo::empty())
    }

    pub fn size(&self) -> usize {
        match self {
            Self::Actions(combo) => combo.size(),
            Self::Positions(combo) => combo.size(),
        }
    }

    pub fn output(&self) -> KeyAction {
        match self {
            Self::Actions(combo) => combo.output,
            Self::Positions(combo) => combo.output,
        }
    }

    pub fn layer(&self) -> Option<u8> {
        match self {
            Self::Actions(combo) => combo.layer,
            Self::Positions(combo) => combo.layer,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.size() == 0 && self.output() == KeyAction::No
    }
}

impl Combo {
    /// Create a new combo from an iterator of key actions.
    ///
    /// Actions equal to `KeyAction::No` are filtered out. If there are more
    /// non-No actions than `COMBO_SIZE`, excess actions are silently dropped.
    pub fn new<I: IntoIterator<Item = KeyAction>>(actions: I, output: KeyAction, layer: Option<u8>) -> Self {
        let mut combo_actions = Vec::new();
        for action in actions {
            if action != KeyAction::No && combo_actions.push(action).is_err() {
                break;
            }
        }
        Self {
            actions: combo_actions,
            output,
            layer,
        }
    }

    /// Get an empty combo.
    pub fn empty() -> Self {
        Self {
            actions: Vec::new(),
            output: KeyAction::No,
            layer: None,
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
