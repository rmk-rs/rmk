//! Input events for RMK
//!
//! This module contains all input-related events:
//! - Keyboard events (key press/release, rotary encoder)
//! - Modifier events
//! - Pointing device events (mouse, trackball, etc.)

use postcard::experimental::max_size::MaxSize;
use rmk_macro::{Event, event};
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

    pub(crate) fn combo(idx: u8, pressed: bool) -> Self {
        Self {
            pressed,
            pos: KeyboardEventPos::Combo(idx),
        }
    }
}

/// The position of the keyboard event.
///
/// A physical position is a key (row, col) or a rotary encoder (id, direction).
/// A synthesized position names the behavior that produced the event.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyboardEventPos {
    Key(KeyPos),
    RotaryEncoder(RotaryEncoderPos),
    /// Output of the combo at this index.
    Combo(u8),
    /// A macro op. A macro's keys belong to no macro in particular.
    Macro,
    /// A software source of events rather than a key, indexed by a u8.
    Virtual(u8),
}

impl KeyboardEventPos {
    /// A key or encoder the user pressed, as opposed to a behavior's synthesized output.
    pub(crate) fn is_physical(self) -> bool {
        matches!(self, Self::Key(_) | Self::RotaryEncoder(_))
    }

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
#[derive(Serialize, Deserialize, Clone, Copy, Debug, MaxSize)]
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

/// Contacts a [`TouchEvent`] carries.
pub const TOUCH_MAX_CONTACTS: usize = 5;

/// One scan of a multi-touch device such as a trackpad: where each contact is.
///
/// Raw data, local to the board the device is on: a touch processor there turns it
/// into pointing events and gestures, and only those cross the split link. A device
/// publishes one per scan while something touches, and one without contacts when
/// the last lifts. Contacts are kept by slot, as in Linux's multi-touch protocol B.
#[event(
    channel_size = crate::TOUCH_EVENT_CHANNEL_SIZE,
    pubs = crate::TOUCH_EVENT_PUB_SIZE,
    subs = crate::TOUCH_EVENT_SUB_SIZE
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TouchEvent {
    /// The id of the pointing device that produced this scan.
    pub device_id: u8,
    /// The largest x and y the device reports.
    pub max: (u16, u16),
    /// Contacts by slot. A contact keeps its slot until it lifts.
    pub contacts: [Option<TouchContact>; TOUCH_MAX_CONTACTS],
}

/// Where a contact is on a touch device, from its top-left corner.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct TouchContact {
    pub x: u16,
    pub y: u16,
}

/// What a touchpad publishes: cursor motion it computes itself, or raw
/// [`TouchEvent`]s for a touch processor to turn into motion and gestures.
#[derive(Event, Clone, Debug)]
pub enum TouchpadEvent {
    Pointing(PointingEvent),
    Touch(TouchEvent),
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
