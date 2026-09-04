//! Compact runtime configuration types for Sticky Keys.

use bitfield_struct::bitfield;
use postcard::experimental::max_size::MaxSize;

/// Events that release a Sticky Key latch.
#[bitfield(u8, order = Lsb, defmt = cfg(feature = "defmt"))]
#[derive(MaxSize, Eq, PartialEq)]
pub struct StickyKeyReleaseMode {
    pub other_key_press: bool,
    pub other_key_release: bool,
    pub layer_enter: bool,
    pub layer_exit: bool,
    pub double_tap: bool,
    #[bits(3)]
    __: u8,
}

impl StickyKeyReleaseMode {
    pub const OTHER_KEY_PRESS: Self = Self::new().with_other_key_press(true);
    pub const OTHER_KEY_RELEASE: Self = Self::new().with_other_key_release(true);
    pub const LAYER_ENTER: Self = Self::new().with_layer_enter(true);
    pub const LAYER_EXIT: Self = Self::new().with_layer_exit(true);
    pub const DOUBLE_TAP: Self = Self::new().with_double_tap(true);

    pub const fn intersects(self, other: Self) -> bool {
        self.into_bits() & other.into_bits() != 0
    }
}
