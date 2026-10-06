//! Rynk protocol ICD — RMK's native host-communication protocol.
//!
//! Carries RMK's canonical types (`KeyAction`, `Combo`, `Morse`, `Fork`,
//! `EncoderAction`, `BatteryStatus`, `BleStatus`) on the wire as a 3-byte
//! fixed header + postcard-encoded payload, COBS-framed.
//!
//! ## Wire format
//!
//! ```text
//! ┌──────────────┬────────────┐
//! │  CMD u16 LE  │   SEQ u8   │  ← 3-byte header
//! ├──────────────┴────────────┤
//! │ postcard-encoded payload  │
//! └───────────────────────────┘
//! ```
//!
//! On the wire the whole frame is COBS-encoded and terminated by a single
//! `0x00` delimiter (see [`message`] and [`Deframer`]), making the byte
//! stream self-synchronizing.
//!
//! - **CMD** — `0x0000..=0x7FFF` request/response, `0x8000..=0xFFFF` topic
//!   (server→host push).
//! - **SEQ** — the sequence number of current request. Topics send SEQ = 0.
//!
//! ## Sizing
//!
//! One parameter drives every derived size: `rynk_buffer_size` in `keyboard.toml`
//! (`constants::RYNK_BUFFER_SIZE`). It's the physical RAM of each frame buffer;
//! COBS framing overhead is deducted internally.
//! - [`RYNK_MAX_PAYLOAD_SIZE`]: the largest payload one frame can carry.
//! - [`MAX_BULK_ITEMS`]/[`MAX_BULK_KEYS`]: how many bulk-page items fit.
//!
//! ## Module layout
//!
//! [`command`] is the only public sub-module: the [`Cmd`] ids, the table
//! binding each command to its payload types, and the
//! [`Endpoint`](command::Endpoint) trait its rows implement. Firmware and host
//! both compile against it, so the two ends can't disagree about a message's
//! types. Everything else — the framing (`message`, [`RynkHeader`],
//! [`encode_frame`], [`RynkMessage`], the COBS [`Deframer`]), [`RynkError`],
//! and the per-domain payload types — is private and re-exported flat at
//! `protocol::rynk::*`.
//!
//! ## Compatibility
//!
//! `Cmd::GetVersion = 0x0001` and its `Result<ProtocolVersion, RynkError>` reply
//! are frozen across all versions.
//! Within a `major`, changes must keep old hosts working, so `minor`
//! is informational: a new `Cmd` or a new/extended topic is a `minor` bump (old
//! peers answer `UnknownCmd` or ignore trailing topic bytes), while reshaping an
//! existing request/response — *including appending a field* — is a `major` bump,
//! since hosts reject trailing response bytes. While `major` is 0 the protocol
//! is unstable: any change bumps `minor`, and hosts require an exact match.
//! The `snapshots/*.snap` golden files (`tests.rs`) fail on any accidental drift.

pub mod command;

mod deframer;
mod error;
mod message;
mod payload;

#[cfg(test)]
pub(crate) mod tests;

pub use self::command::{Cmd, TopicEvent};
pub use self::deframer::Deframer;
pub use self::error::RynkError;
pub use self::message::{
    RYNK_HEADER_SIZE, RYNK_MAX_PAYLOAD_SIZE, RynkHeader, RynkMessage, encode_frame, max_wire_size,
};
pub use self::payload::*;

/// Largest single GATT write/notification on the Rynk BLE characteristics.
pub const RYNK_BLE_CHUNK_SIZE: usize = 244;

/// Fixed size of one Rynk-over-WebHID report (`RynkHidService`)
pub const RYNK_HID_REPORT_SIZE: usize = 32;

/// Rynk GATT service UUID
pub const RYNK_SERVICE_UUID: u128 = 0x10900067_537f_4f0a_9b55_929e271f61ab;
/// Rynk `input_data` characteristic UUID.
pub const RYNK_INPUT_CHAR_UUID: u128 = 0x80f9319b_0c74_43a5_9738_c59d6dda3db9;
/// Rynk `output_data` characteristic UUID.
pub const RYNK_OUTPUT_CHAR_UUID: u128 = 0x19802524_6f90_4346_93c2_63dbc509ab55;

/// Informational marker the `rynk` firmware prepends to its USB serial number.
/// Host discovery keys on the vendor interface triple below, not on this.
pub const RYNK_MAGIC: &str = "rynk:";

/// Class of the Rynk vendor-specific USB bulk interface.
pub const RYNK_USB_INTERFACE_CLASS: u8 = 0xFF;
/// Subclass of the Rynk USB interface: `'R'`.
pub const RYNK_USB_INTERFACE_SUBCLASS: u8 = 0x52;
/// Protocol of the Rynk USB interface: `'R'`.
pub const RYNK_USB_INTERFACE_PROTOCOL: u8 = 0x52;
