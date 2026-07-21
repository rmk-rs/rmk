//! Bounded application-message hook for the split protocol.
//!
//! A small, opaque, bounded payload that an application can send between the
//! split central and its peripheral alongside — but never in front of — the
//! normal split traffic. RMK itself attaches no meaning to the bytes; a
//! firmware built on RMK can use it for application-level side-band state
//! (for example, propagating lighting or UI state across the split link).
//!
//! Flow:
//!
//! - Central: the application queues [`SplitAppData`] into [`SPLIT_APP_TX`]
//!   (bounded, `try_send` only — the queue must never be awaited full).
//!   `PeripheralManager` drains it as one more (lowest-priority) arm of its
//!   outgoing-message select and wraps each payload in
//!   `SplitMessage::Application`. Peripheral→central key events always win:
//!   they arrive on the read arm, which is polled first.
//! - Peripheral: `SplitPeripheral` forwards received `Application` messages
//!   into [`SPLIT_APP_RX`] with `try_send` (drop-on-full keeps the split read
//!   loop responsive; the application is expected to tolerate loss, e.g. by
//!   resyncing on reconnect).
//! - Peripheral → central: the peripheral application queues
//!   [`SplitAppData`] into [`SPLIT_APP_PERIPH_TX`] (bounded, `try_send`
//!   only); `SplitPeripheral` drains it as one more outgoing arm of its
//!   select, behind key events. The central's `PeripheralManager` forwards
//!   received `Application` messages into [`SPLIT_APP_RX`] the same way the
//!   peripheral does — the inbox is symmetric ("this side's received
//!   application messages"), only the senders differ.
//! - Both sides: [`SPLIT_APP_LINK`] carries the split-link state (central:
//!   "peripheral link up"; peripheral: "central link up"), set by the split
//!   driver. The central raises it at session start; the peripheral raises
//!   it on the FIRST message received from the central — for BLE the bare
//!   connection is not enough, since notifications to a central that has
//!   not yet subscribed are silently dropped (see `split/peripheral.rs`).
//!   Both lower it at session end. Applications use the `false → true` edge
//!   to trigger an idempotent resync.
//!
//! Note: the queues assume a single split peripheral. Extending this hook to
//! multiple peripherals would key them by peripheral id.

use embassy_sync::channel::Channel;
use embassy_sync::watch::Watch;
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::RawMutex;

/// Maximum payload of one application split message. Deliberately small:
/// every split BLE transfer is `SPLIT_MESSAGE_MAX_SIZE` bytes on the wire,
/// so this bound also taxes key-event messages. Hard cap: the trouble
/// `gatt_service` macro initializes its characteristic arrays via
/// `Default`, which arrays only implement up to 32 elements — so
/// `SPLIT_MESSAGE_MAX_SIZE` (this + 1-byte length prefix + 1-byte enum
/// discriminant + 4 bytes margin) must stay ≤ 32.
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

/// Central → peripheral queue, drained by `PeripheralManager` while the link
/// is up. Producers MUST use `try_send` (bounded, never block); capacity is
/// sized so one full application resync burst fits with headroom. Forty-eight
/// entries admit a complete 40-cell semantic lighting snapshot (begin,
/// context, one worst-case animated cell per packet, and commit). This is
/// mutation/reconnect traffic, not rendered-frame streaming.
pub static SPLIT_APP_TX: Channel<RawMutex, SplitAppData, 48> = Channel::new();

/// This side's inbox of received application messages (peripheral: from the
/// central's `SPLIT_APP_TX`; central: from the peripheral's
/// `SPLIT_APP_PERIPH_TX`). Filled with `try_send` (drop-on-full) by the
/// split read loops.
pub static SPLIT_APP_RX: Channel<RawMutex, SplitAppData, 8> = Channel::new();

/// Peripheral → central queue, drained by `SplitPeripheral` while the link
/// is up. Producers MUST use `try_send`. Small: the application announces
/// tiny, rare state (e.g. its build identity once per link-up).
pub static SPLIT_APP_PERIPH_TX: Channel<RawMutex, SplitAppData, 2> = Channel::new();

/// Split-link state for the application: on the central, "peripheral link
/// up"; on the peripheral, "central link up". Written by the split driver at
/// session start/end; state-based (a late receiver still observes the latest
/// value), so edges cannot be lost the way pub/sub events can.
pub static SPLIT_APP_LINK: Watch<RawMutex, bool, 2> = Watch::new();
