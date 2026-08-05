//! Bounded application-message hook for the split protocol.
//!
//! An application built on RMK can exchange small opaque payloads
//! ([`SplitAppData`]) between the split halves alongside — but never in front
//! of — normal split traffic. RMK attaches no meaning to the bytes.
//!
//! Each side queues outgoing payloads with `try_send` ([`SPLIT_APP_TX`] on
//! the central, [`SPLIT_APP_PERIPH_TX`] on the peripheral). The split loops
//! drain them as their lowest-priority outgoing arm and deliver received
//! payloads into [`SPLIT_APP_RX`], dropping on full so application traffic
//! can never delay key events. [`SPLIT_APP_LINK`] carries the link state; its
//! `false → true` edge is the application's cue to resync, which also covers
//! messages lost to a full queue.
//!
//! The queues assume a single split peripheral.

use embassy_sync::channel::Channel;
use embassy_sync::watch::Watch;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::RawMutex;

/// Maximum payload of one application split message. Every split transfer is
/// `SPLIT_MESSAGE_MAX_SIZE` bytes on the wire, and trouble's `gatt_service`
/// macro caps that at 32 (it initializes characteristic arrays via `Default`,
/// which arrays only implement up to 32 elements), so this plus postcard
/// overhead must keep `SPLIT_MESSAGE_MAX_SIZE` ≤ 32.
pub const SPLIT_APP_MSG_MAX: usize = 26;

/// One opaque application payload. Only `data[..len]` is meaningful.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct SplitAppData {
    pub len: u8,
    pub data: [u8; SPLIT_APP_MSG_MAX],
}

impl SplitAppData {
    /// Wrap `payload`; `None` if it exceeds [`SPLIT_APP_MSG_MAX`].
    pub fn new(payload: &[u8]) -> Option<Self> {
        if payload.len() > SPLIT_APP_MSG_MAX {
            return None;
        }
        let mut data = [0u8; SPLIT_APP_MSG_MAX];
        data[..payload.len()].copy_from_slice(payload);
        Some(Self {
            len: payload.len() as u8,
            data,
        })
    }

    pub fn payload(&self) -> &[u8] {
        &self.data[..(self.len as usize).min(SPLIT_APP_MSG_MAX)]
    }
}

// Postcard stores the payload as `&[u8]` (varint length prefix + bytes), so
// only the used bytes travel inside the (fixed-size) split transfer buffer.
impl Serialize for SplitAppData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.payload().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for SplitAppData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let buf: &[u8] = Deserialize::deserialize(deserializer)?;
        SplitAppData::new(buf).ok_or_else(|| D::Error::custom("split app message too long"))
    }
}

impl MaxSize for SplitAppData {
    // 1-byte varint length prefix + payload.
    const POSTCARD_MAX_SIZE: usize = SPLIT_APP_MSG_MAX + 1;
}

/// Central → peripheral queue. Producers must use `try_send` (never await a
/// full queue); capacity fits one full application resync burst.
pub static SPLIT_APP_TX: Channel<RawMutex, SplitAppData, 26> = Channel::new();

/// This side's inbox of received application messages. Filled with `try_send`
/// (drop-on-full) by the split read loops.
pub static SPLIT_APP_RX: Channel<RawMutex, SplitAppData, 8> = Channel::new();

/// Peripheral → central queue. Producers must use `try_send`. Small: meant
/// for tiny, rare state such as a build identity announced once per link-up.
pub static SPLIT_APP_PERIPH_TX: Channel<RawMutex, SplitAppData, 2> = Channel::new();

/// Deliver a received payload into [`SPLIT_APP_RX`]. Drop-on-full: a slow
/// application consumer must never stall the split read loops, and the
/// application resyncs on reconnect anyway.
pub(crate) fn deliver_received(data: SplitAppData) {
    if SPLIT_APP_RX.try_send(data).is_err() {
        warn!("split app message dropped (inbox full)");
    }
}

/// Split-link state for the application, written by the split loops via
/// [`LinkGuard`]. State-based, so a late receiver still observes the current
/// value.
pub static SPLIT_APP_LINK: Watch<RawMutex, bool, 2> = Watch::new();

/// Owns one split session's [`SPLIT_APP_LINK`] state: `mark_up` sends the
/// link-up edge once, and `Drop` sends the link-down edge. The down edge must
/// come from `Drop` because the session futures holding a guard are cancelled
/// from outside on connection loss.
pub(crate) struct LinkGuard {
    up: bool,
}

impl LinkGuard {
    pub(crate) fn new() -> Self {
        Self { up: false }
    }

    /// Send the link-up edge; idempotent.
    pub(crate) fn mark_up(&mut self) {
        if !self.up {
            SPLIT_APP_LINK.sender().send(true);
            self.up = true;
        }
    }
}

impl Drop for LinkGuard {
    fn drop(&mut self) {
        SPLIT_APP_LINK.sender().send(false);
    }
}
