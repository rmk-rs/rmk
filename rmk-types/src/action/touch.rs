//! Touchpad gesture actions.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use super::KeyAction;

/// A gesture a touchpad recognizes. Each has its own action per layer in the touch
/// map.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, MaxSize, strum::EnumCount)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum TouchGesture {
    /// One finger touching and lifting without moving.
    Tap,
    TwoFingerTap,
    ThreeFingerTap,
}

impl TouchGesture {
    /// How many gestures there are.
    pub const COUNT: usize = <Self as strum::EnumCount>::COUNT;
}

/// The actions of a touchpad's gestures, stored in the touch map.
///
/// Every gesture defaults to `KeyAction::No` (no action).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TouchAction {
    /// Each gesture's action, indexed by `TouchGesture as usize`.
    pub actions: [KeyAction; TouchGesture::COUNT],
}

impl Default for TouchAction {
    fn default() -> Self {
        Self::new()
    }
}

impl TouchAction {
    /// No action for any gesture.
    pub const fn new() -> Self {
        Self {
            actions: [KeyAction::No; TouchGesture::COUNT],
        }
    }

    /// Every gesture transparent: each takes its action from the layer below.
    pub const fn transparent() -> Self {
        Self {
            actions: [KeyAction::Transparent; TouchGesture::COUNT],
        }
    }

    /// `self` with `gesture` triggering `action`.
    pub const fn with(mut self, gesture: TouchGesture, action: KeyAction) -> Self {
        self.actions[gesture as usize] = action;
        self
    }

    pub const fn get(&self, gesture: TouchGesture) -> KeyAction {
        self.actions[gesture as usize]
    }
}
