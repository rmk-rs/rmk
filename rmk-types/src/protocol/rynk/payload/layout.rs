//! Layout wire payload.
//!
//! `GetLayout` streams an opaque, compressed blob the firmware never decodes —
//! it just copies `blob[offset..offset+N]` into [`LayoutChunk`]. `rmk-config`
//! produces the bytes at build time; the host inflates the assembled bytes and
//! decodes them with `rmk_config::layout::LayoutInfo`, re-exported by
//! `rynk::layout`. This `#![no_std]` crate stays alloc-free; only [`LayoutChunk`]
//! needs `MaxSize`.
//!
//! The blob carries no version byte of its own: the shared `LayoutInfo`
//! postcard schema is part of the wire contract, so
//! reshaping it — including appending a field — is a protocol **major** bump,
//! exactly like reshaping any response payload.

use heapless::Vec;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

use super::super::RYNK_BLE_CHUNK_SIZE;

/// One page of the opaque, compressed layout blob, served by `GetLayout`.
///
/// `total_len` is the whole compressed blob length, so the host knows when it
/// has collected every page; `bytes` is one page (≤ one BLE frame).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct LayoutChunk {
    /// Total length of the whole compressed blob.
    pub total_len: u32,
    /// This page's bytes.
    pub bytes: Vec<u8, RYNK_BLE_CHUNK_SIZE>,
}

impl MaxSize for LayoutChunk {
    const POSTCARD_MAX_SIZE: usize =
        <u32 as MaxSize>::POSTCARD_MAX_SIZE + crate::heapless_vec_max_size::<u8, RYNK_BLE_CHUNK_SIZE>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    #[test]
    fn round_trip_layout_chunk() {
        round_trip(&LayoutChunk {
            total_len: 0,
            bytes: Vec::new(),
        });

        let mut bytes: Vec<u8, RYNK_BLE_CHUNK_SIZE> = Vec::new();
        for i in 0..RYNK_BLE_CHUNK_SIZE {
            bytes.push(i as u8).unwrap();
        }
        let full = LayoutChunk {
            total_len: u32::MAX,
            bytes,
        };
        round_trip(&full);
        assert_max_size_bound(&full);
    }
}
