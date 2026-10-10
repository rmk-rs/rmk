use core::num::NonZeroU32;

#[allow(unused_imports)]
use super::*;
use crate::lighting::Rgb8;
use crate::lighting::effect::BuiltinEffect;
use crate::lighting::source::OverlayError;
use crate::lighting::topology::LedSlot;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OverlayCell {
    pub slot: LedSlot,
    pub effect: BuiltinEffect,
    /// Relative lifetime from command application. `None` persists until an
    /// explicit unset/clear or reboot.
    pub ttl_ms: Option<NonZeroU32>,
}

const EMPTY_OVERLAY_CELL: OverlayCell = OverlayCell {
    slot: LedSlot(0),
    effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
    ttl_ms: None,
};

/// Fixed-capacity, owned batch suitable for a bounded async mailbox.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OverlayBatch<const CAP: usize> {
    cells: [OverlayCell; CAP],
    len: usize,
}

impl<const CAP: usize> OverlayBatch<CAP> {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_OVERLAY_CELL; CAP],
            len: 0,
        }
    }

    pub fn push(&mut self, cell: OverlayCell) -> Result<(), OverlayError> {
        if self.len == CAP {
            return Err(OverlayError::TooManyEntries {
                supplied: self.len + 1,
                capacity: CAP,
            });
        }
        self.cells[self.len] = cell;
        self.len += 1;
        Ok(())
    }

    pub fn as_slice(&self) -> &[OverlayCell] {
        &self.cells[..self.len]
    }
}

impl<const CAP: usize> Default for OverlayBatch<CAP> {
    fn default() -> Self {
        Self::new()
    }
}
