//! Composite key actions stored in the keymap.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use super::Action;

/// A KeyAction is the action at a keyboard position, stored in keymap.
/// It can be a single action like triggering a key, or a composite keyboard action like tap/hold
#[derive(Debug, Copy, Clone, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum KeyAction {
    /// No action
    No,
    /// Transparent action, next layer will be checked
    Transparent,
    /// A single action, such as triggering a key, or activating a layer. Action is triggered when pressed and cancelled when released.
    Single(Action),
    /// Don't wait the release of the key, auto-release after a time threshold.
    Tap(Action),
    /// Tap hold action. The `u8` indexes the morse profile table; an index
    /// with no table entry (e.g. `u8::MAX`, which the default-profile macros
    /// emit) falls back to the default [`crate::morse::MorseProfile`].
    TapHold(Action, Action, u8),
    /// Morse action, references a morse configuration by index.
    Morse(u8),
    /// Sticky activation of a modifier, layer-on, or key-with-modifier action.
    /// The `u8` indexes the Sticky Key profile table; `u8::MAX` selects the
    /// default profile. An unavailable index also falls back to the default.
    Sticky(Action, u8),
}

impl KeyAction {
    /// Convert `KeyAction` to the internal `Action`.
    /// Only valid for `Single` and `Tap` variant, returns `Action::No` for other variants.
    pub fn to_action(self) -> Action {
        match self {
            KeyAction::Single(a) | KeyAction::Tap(a) => a,
            _ => Action::No,
        }
    }

    /// 'morse' is an alias for the superset of tap dance and tap hold keys,
    /// since their handling have many similarities
    pub fn is_morse(&self) -> bool {
        matches!(self, KeyAction::TapHold(_, _, _) | KeyAction::Morse(_))
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, KeyAction::No)
    }
}

/// Combo and fork trigger matching compares tap/hold actions by their logical
/// actions while ignoring their timing-profile index. Sticky profiles remain
/// part of identity because they define release behavior, not only timing.
///
/// This is intentional: a combo or fork may store a trigger with one profile
/// index, but if the user later rebinds the key's profile, the trigger should
/// still match. The profile is a per-key timing config, not part of the key's
/// logical identity.
impl PartialEq for KeyAction {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (KeyAction::No, KeyAction::No) => true,
            (KeyAction::Transparent, KeyAction::Transparent) => true,
            (KeyAction::Single(a), KeyAction::Single(b)) => a == b,
            (KeyAction::Tap(a), KeyAction::Tap(b)) => a == b,
            (KeyAction::TapHold(a, b, _), KeyAction::TapHold(c, d, _)) => a == c && b == d,
            (KeyAction::Morse(a), KeyAction::Morse(b)) => a == b,
            (KeyAction::Sticky(a, profile_a), KeyAction::Sticky(b, profile_b)) => a == b && profile_a == profile_b,
            _ => false,
        }
    }
}
