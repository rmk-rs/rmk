use postcard::experimental::max_size::MaxSize;
use rmk_types::connection::ConnectionStatus;
use serde::{Deserialize, Serialize};
#[cfg(feature = "dfu_split")]
use serde::{Deserializer, Serializer};

#[cfg(feature = "_ble")]
use crate::event::BatteryStatusEvent;
use crate::event::{KeyboardEvent, PointingEvent};

#[cfg(feature = "_ble")]
pub mod ble;
pub mod central;
/// Common abstraction layer of split driver
pub(crate) mod driver;
pub mod peripheral;
#[cfg(feature = "rp2040")]
pub mod rp;
#[cfg(not(feature = "_ble"))]
pub mod serial;

/// Maximum size of a split message
pub const SPLIT_MESSAGE_MAX_SIZE: usize = SplitMessage::POSTCARD_MAX_SIZE + 4;

/// The rectangular region of the central's keymap covered by one split
/// peripheral: its matrix size, and where it sits.
#[derive(Debug, Clone, Copy)]
pub struct PeripheralMatrixConfig {
    pub rows: u8,
    pub cols: u8,
    pub row_offset: u8,
    pub col_offset: u8,
}

/// Message used from central & peripheral communication
#[repr(u8)]
#[derive(Serialize, Deserialize, Debug, Clone, Copy, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum SplitMessage {
    /// Keyboard event, from peripheral to central
    Key(KeyboardEvent),
    /// Pointing device event, from peripheral to central
    Pointing(PointingEvent),
    /// Led state, on/off, from central to peripheral
    LedState(bool),
    /// `ConnectionStatus` snapshot of the central.
    /// Synced central → peripheral on every change.
    ConnectionStatus(ConnectionStatus),
    /// BLE Address, used in syncing address between central and peripheral
    Address([u8; 6]),
    /// Clear the saved peer info
    ClearPeer,
    /// Lock state led indicator from central to peripheral
    KeyboardIndicator(u8),
    /// Layer number from central to peripheral
    Layer(u8),
    /// WPM from central to peripheral
    #[cfg(feature = "display")]
    Wpm(u16),
    /// Modifier state from central to peripheral
    #[cfg(feature = "display")]
    Modifier(u8),
    /// Sleep state from central to peripheral
    #[cfg(feature = "_render_state")]
    SleepState(bool),
    /// Battery status, from peripheral to central
    #[cfg(feature = "_ble")]
    BatteryStatus(BatteryStatusEvent),

    // -----------------------------------------------------------------------
    // dfu_split — firmware update over split link
    // -----------------------------------------------------------------------
    /// Central → Peripheral: query the hash of the ACTIVE slot firmware.
    #[cfg(feature = "dfu_split")]
    FirmwareHashQuery,
    /// Peripheral → Central: respond with the CRC32 of the ACTIVE slot firmware.
    #[cfg(feature = "dfu_split")]
    FirmwareHashResponse(u32),
    /// Central → Peripheral: a chunk of the new firmware at a given offset.
    #[cfg(feature = "dfu_split")]
    FirmwareChunk {
        offset: u32,
        len: u16,
        data: FirmwareChunkData,
    },
    /// Peripheral → Central: acknowledge that `offset` bytes have been written,
    /// together with the CRC-32 of **this single chunk**.
    #[cfg(feature = "dfu_split")]
    FirmwareChunkAck { offset: u32, crc: u32 },
    /// Central → Peripheral: all chunks sent, peripheral should compute DFU CRC.
    #[cfg(feature = "dfu_split")]
    FirmwareUpdateComplete,
    /// Peripheral → Central: CRC-32 of the full DFU partition.
    #[cfg(feature = "dfu_split")]
    FirmwareCrcReport(u32),
    /// Central → Peripheral: end-to-end CRC matches, safe to reset.
    #[cfg(feature = "dfu_split")]
    FirmwareCrcOk,
    /// Central → Peripheral: end-to-end CRC mismatch, do NOT reset.
    #[cfg(feature = "dfu_split")]
    FirmwareCrcFail,
    /// Peripheral → Central: confirm mark_updated succeeded, about to reset.
    #[cfg(feature = "dfu_split")]
    FirmwareUpdateConfirm,

    /// opaque bounded application payload, central →
    /// peripheral (see `crate::split_app`). Kept as the LAST variant so
    /// the postcard discriminants of all existing messages stay stable across
    /// halves flashed at different revisions.
    Application(crate::split_app::SplitAppData),
}

// -----------------------------------------------------------------------
// FirmwareChunkData — buffer for dfu_split firmware transfer
// -----------------------------------------------------------------------

/// Fixed-size buffer for firmware chunk transfer over the split link.
///
/// Postcard's COBS encoding stores this as `&[u8]` (varint length prefix
/// + bytes) rather than a fixed `[u8; 256]` — necessary because serde
/// does not implement `Deserialize` for arrays larger than 32 elements.
#[cfg(feature = "dfu_split")]
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct FirmwareChunkData(pub [u8; 256]);

#[cfg(feature = "dfu_split")]
impl Serialize for FirmwareChunkData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_slice().serialize(serializer)
    }
}

#[cfg(feature = "dfu_split")]
impl<'de> Deserialize<'de> for FirmwareChunkData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let buf: &[u8] = Deserialize::deserialize(deserializer)?;
        if buf.len() > 256 {
            return Err(D::Error::custom("firmware chunk exceeds 256 bytes"));
        }
        let mut data = [0u8; 256];
        data[..buf.len()].copy_from_slice(buf);
        Ok(FirmwareChunkData(data))
    }
}

#[cfg(feature = "dfu_split")]
impl MaxSize for FirmwareChunkData {
    const POSTCARD_MAX_SIZE: usize = 258;
}
