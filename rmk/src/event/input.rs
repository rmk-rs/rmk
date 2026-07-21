//! Input events for RMK
//!
//! This module contains all input-related events:
//! - Keyboard events (key press/release, rotary encoder)
//! - Modifier events
//! - Pointing device events (mouse, trackball, etc.)

use postcard::experimental::max_size::MaxSize;
use rmk_macro::event;
use rmk_types::modifier::ModifierCombination;
use serde::{Deserialize, Serialize};

use crate::input_device::pointing::PointingMode;
use crate::input_device::rotary_encoder::Direction;
// ============================================================================
// Keyboard Events
// ============================================================================

/// `KeyboardEvent` is the event whose `KeyAction` is stored in the keymap.
///
/// `KeyboardEvent` is different from events from pointing devices,
/// events from pointing devices are processed directly by the corresponding processors,
/// while `KeyboardEvent` is processed by the keyboard with the keymap.
#[event(
    channel_size = crate::KEYBOARD_EVENT_CHANNEL_SIZE,
    pubs = crate::KEYBOARD_EVENT_PUB_SIZE,
    subs = crate::KEYBOARD_EVENT_SUB_SIZE
)]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeyboardEvent {
    pub pressed: bool,
    pub pos: KeyboardEventPos,
}

impl KeyboardEvent {
    pub fn key(row: u8, col: u8, pressed: bool) -> Self {
        Self {
            pressed,
            pos: KeyboardEventPos::Key(KeyPos { row, col }),
        }
    }

    pub fn rotary_encoder(id: u8, direction: Direction, pressed: bool) -> Self {
        Self {
            pressed,
            pos: KeyboardEventPos::RotaryEncoder(RotaryEncoderPos { id, direction }),
        }
    }
}

/// The position of the keyboard event.
///
/// The position can be either a key (row, col), or a rotary encoder (id, direction)
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyboardEventPos {
    Key(KeyPos),
    RotaryEncoder(RotaryEncoderPos),
}

impl KeyboardEventPos {
    pub(crate) fn key_pos(col: u8, row: u8) -> Self {
        Self::Key(KeyPos { row, col })
    }

    pub(crate) fn rotary_encoder_pos(id: u8, direction: Direction) -> Self {
        Self::RotaryEncoder(RotaryEncoderPos { id, direction })
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct KeyPos {
    pub row: u8,
    pub col: u8,
}

/// Event for rotary encoder
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct RotaryEncoderPos {
    /// The id of the rotary encoder
    pub id: u8,
    /// The direction of the rotary encoder
    pub direction: Direction,
}

// ============================================================================
// Modifier Events
// ============================================================================

/// Final resolved HID modifier combination changed event.
///
/// This carries the modifier byte used by the keyboard report, including
/// one-shot, macro, with-modifier, and fork effects. It is not merely the set
/// of physically held modifier keys.
#[event(channel_size = crate::MODIFIER_EVENT_CHANNEL_SIZE, pubs = crate::MODIFIER_EVENT_PUB_SIZE, subs = crate::MODIFIER_EVENT_SUB_SIZE)]
#[derive(Clone, Copy, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ModifierEvent {
    pub modifier: ModifierCombination,
}

// ============================================================================
// Pointing Device Events
// ============================================================================

#[event(
    channel_size = crate::POINTING_EVENT_CHANNEL_SIZE,
    pubs = crate::POINTING_EVENT_PUB_SIZE,
    subs = crate::POINTING_EVENT_SUB_SIZE
)]
#[derive(Serialize, Deserialize, Clone, Debug, Copy, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PointingEvent {
    /// The id of the pointing device that produced this event.
    pub device_id: u8,
    /// Raw axis values (X, Y, Z).
    pub axes: [AxisEvent; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct AxisEvent {
    /// The axis event value type, relative or absolute
    pub typ: AxisValType,
    /// The axis name
    pub axis: Axis,
    /// Value of the axis event
    pub value: i16,
}

#[derive(Serialize, Deserialize, Clone, Debug, Copy, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum AxisValType {
    /// The axis value is relative
    Rel,
    /// The axis value is absolute
    Abs,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
pub enum Axis {
    X,
    Y,
    Z,
    H,
    V,
    // .. More is allowed
}

/// Set the CPI (Resolution) of the pointing device
/// TODO: Make the channel size configurable
#[event(channel_size = 8, pubs = 2, subs = 2)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PointingSetCpiEvent {
    pub device_id: u8,
    pub cpi: u16,
}

/// Pointing processor event
/// TODO: Make the channel size configurable
#[event(channel_size = 8, pubs = 2, subs = 2)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct PointingProcessorEvent {
    pub device_id: u8,
    pub mode: PointingMode,
}
