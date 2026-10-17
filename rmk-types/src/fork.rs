//! Shared fork-related types used by firmware and protocol layers.

use core::ops::{BitAnd, BitOr, Not};

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use crate::action::KeyAction;
use crate::led_indicator::LedIndicator;
use crate::modifier::ModifierCombination;
use crate::mouse_button::MouseButtons;

/// Bitset state used by fork matching logic.
///
/// A zero (default) value means "match nothing" — no modifiers, LEDs, or mouse buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct StateBits {
    /// Active modifier combination to match.
    pub modifiers: ModifierCombination,
    /// LED indicator state to match (Num/Caps/Scroll Lock, etc.).
    pub leds: LedIndicator,
    /// Mouse button state to match.
    pub mouse: MouseButtons,
}

impl BitOr for StateBits {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self {
            modifiers: self.modifiers | rhs.modifiers,
            leds: self.leds | rhs.leds,
            mouse: self.mouse | rhs.mouse,
        }
    }
}

impl BitAnd for StateBits {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self {
            modifiers: self.modifiers & rhs.modifiers,
            leds: self.leds & rhs.leds,
            mouse: self.mouse & rhs.mouse,
        }
    }
}

impl Not for StateBits {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self {
            modifiers: !self.modifiers,
            leds: !self.leds,
            mouse: !self.mouse,
        }
    }
}

impl StateBits {
    pub const fn new_from(modifiers: ModifierCombination, leds: LedIndicator, mouse: MouseButtons) -> Self {
        Self { modifiers, leds, mouse }
    }
}

/// Fork (key override) configuration.
///
/// A fork overrides a key's output based on the current modifier/LED/mouse state.
/// When the trigger key is pressed, the fork checks current state against `match_any`
/// and `match_none` to decide between `positive_output` and `negative_output`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct Fork {
    /// The key action that activates this fork. Should not be `KeyAction::Transparent`.
    pub trigger: KeyAction,
    /// Output when the state condition is NOT met. `Transparent` leaves the key alone.
    pub negative_output: KeyAction,
    /// Output when the state condition IS met.
    pub positive_output: KeyAction,
    /// If any of these state bits are active, the positive branch is taken.
    pub match_any: StateBits,
    /// If any of these state bits are active, the fork is suppressed.
    pub match_none: StateBits,
    /// Modifiers to keep active when the fork fires.
    pub kept_modifiers: ModifierCombination,
    /// Whether this fork's output is checked against other forks that have not
    /// yet triggered, allowing chained replacements.
    pub bindable: bool,
    /// Bitmask of layers the trigger must come from. `u32::MAX` means all layers.
    pub layers: u32,
}

impl Default for Fork {
    fn default() -> Self {
        Self::empty()
    }
}

impl Fork {
    pub fn new(
        trigger: KeyAction,
        negative_output: KeyAction,
        positive_output: KeyAction,
        match_any: StateBits,
        match_none: StateBits,
        kept_modifiers: ModifierCombination,
        bindable: bool,
    ) -> Self {
        Self {
            trigger,
            negative_output,
            positive_output,
            match_any,
            match_none,
            kept_modifiers,
            bindable,
            layers: u32::MAX,
        }
    }

    pub fn empty() -> Self {
        Self::new(
            KeyAction::No,
            KeyAction::No,
            KeyAction::No,
            StateBits::default(),
            StateBits::default(),
            ModifierCombination::default(),
            false,
        )
    }
}
