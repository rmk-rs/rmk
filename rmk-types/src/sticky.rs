//! Sticky key configuration.
//!
//! A sticky key postpones the release of the action it wraps until the next
//! input, so a tap of `SK(LShift)` shifts the key that follows it. This module
//! holds the per-key configuration; the state machine lives in the `rmk` crate.

use bitfield_struct::bitfield;
use heapless::Vec;

use crate::constants::STICKY_IGNORE_MAX;
use crate::keycode::HidKeyCode;

/// When the host gets to see the effect, and which layer transitions release it.
#[bitfield(u8, order = Lsb)]
#[derive(PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StickyFlags {
    /// Send the effect to the host as soon as the sticky key is pressed, rather
    /// than holding it back until the next input.
    #[bits(1)]
    pub activate_on_press: bool,
    /// Release on the next input's press edge instead of letting the effect
    /// disappear with that key's own release report.
    #[bits(1)]
    pub release_on_next_press: bool,
    /// Release when a layer is activated.
    #[bits(1)]
    pub release_on_layer_enter: bool,
    /// Release when a layer is deactivated.
    #[bits(1)]
    pub release_on_layer_exit: bool,
    #[bits(4)]
    __: u8,
}

/// One sticky key profile, referenced by index from [`crate::action::KeyAction::Sticky`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct StickyProfile {
    /// How long the effect survives after the key comes up. 0 means no timeout.
    pub timeout_ms: u16,
    /// Keycodes that don't count as "the next input". Hitting one of these also
    /// refills the timeout, which is what makes Alt+Tab style cycling work.
    pub ignore: Vec<HidKeyCode, STICKY_IGNORE_MAX>,
    pub flags: StickyFlags,
}

impl Default for StickyProfile {
    fn default() -> Self {
        Self {
            timeout_ms: 1000,
            ignore: Vec::new(),
            flags: StickyFlags::new(),
        }
    }
}

/// Profile index reserved for `OSL`. It resolves to the default profile with
/// `release_on_next_press` forced on: a layer has to be restored on the next
/// key's press, otherwise the key after that one may still resolve on it.
/// ZMK splits `sk` and `sl` the same way.
pub const STICKY_PROFILE_LAYER: u8 = u8::MAX - 1;
