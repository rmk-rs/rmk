//! DFU (Device Firmware Update) protocol types.

#[cfg(feature = "host")]
extern crate alloc;
#[cfg(feature = "host")]
use alloc::vec::Vec;

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Maximum firmware data bytes per `DfuWriteRequest`.
///
/// This is derived from [`RYNK_MAX_PAYLOAD_SIZE`](super::super::message::RYNK_MAX_PAYLOAD_SIZE)
/// minus the postcard overhead for the `offset` field (u32 varint, 5 bytes)
/// and the data length varint (2 bytes for sizes up to 32 767).  The host
/// queries `DeviceCapabilities.max_payload_size` at runtime and chunks the
/// firmware accordingly; this bound ensures the firmware can always accept
/// what the host is allowed to send.
pub const DFU_DATA_MAX: usize = {
    let max_payload = super::super::message::RYNK_MAX_PAYLOAD_SIZE;
    // Overhead: offset u32 varint (5) + data length varint (2 for ≤32767)
    let overhead = 7;
    if max_payload > overhead {
        max_payload - overhead
    } else {
        0
    }
};

/// Firmware data chunk for DFU download.
///
/// The host sends firmware data in chunks whose size it derives from
/// `DeviceCapabilities.max_payload_size`.  On the firmware side the data
/// field is bounded to [`DFU_DATA_MAX`] so every valid host payload fits;
/// on the host side it is unbounded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct DfuWriteRequest {
    /// Byte offset within the DFU partition.
    pub offset: u32,
    /// Firmware data bytes.
    #[cfg(not(feature = "host"))]
    pub data: heapless::Vec<u8, { DFU_DATA_MAX }>,
    #[cfg(feature = "host")]
    pub data: Vec<u8>,
}

/// Manual `MaxSize` impl because `heapless::Vec<u8, N>::POSTCARD_MAX_SIZE`
/// depends on the const generic, and the derive macro cannot reference
/// `DFU_DATA_MAX` directly.
#[cfg(not(feature = "host"))]
impl MaxSize for DfuWriteRequest {
    const POSTCARD_MAX_SIZE: usize = u32::POSTCARD_MAX_SIZE + crate::heapless_vec_max_size::<u8, { DFU_DATA_MAX }>();
}

/// CRC synchronization request: the host sends its running CRC-32 and the
/// firmware compares it against its own accumulated value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct DfuCrcSyncRequest {
    /// CRC-32 the host computed over all data sent since the last checkpoint.
    pub expected_crc: u32,
}

/// Roll the firmware's DFU state back to a previous checkpoint.
///
/// After a CRC mismatch the host re-sends data starting from the last known
/// good offset. This command resets the firmware's running CRC and write
/// offset so the retransmitted data accumulates correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct DfuCrcRewindRequest {
    /// Byte offset to rewind to (the checkpoint offset).
    pub offset: u32,
    /// CRC-32 value at the checkpoint (the finalized value to restore).
    pub crc: u32,
}

/// End-of-transfer verification: the host sends the CRC-32 of the entire
/// firmware image. The firmware compares against its accumulated value and
/// performs the vector-table sanity check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct DfuVerifyRequest {
    /// CRC-32 of the complete firmware image.
    pub crc32: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    #[test]
    fn round_trip_dfu_write_request() {
        let req = DfuWriteRequest {
            offset: 0x1000,
            data: heapless::Vec::from_slice(&[1, 2, 3, 4]).unwrap(),
        };
        round_trip(&req);
    }

    #[test]
    fn round_trip_dfu_crc_sync_request() {
        round_trip(&DfuCrcSyncRequest {
            expected_crc: 0xDEADBEEF,
        });
    }

    #[test]
    fn round_trip_dfu_crc_rewind_request() {
        round_trip(&DfuCrcRewindRequest {
            offset: 0x2000,
            crc: 0x12345678,
        });
    }

    #[test]
    fn round_trip_dfu_verify_request() {
        round_trip(&DfuVerifyRequest { crc32: 0xABCD1234 });
    }

    #[test]
    fn max_size_bounds() {
        assert_max_size_bound(&DfuCrcSyncRequest { expected_crc: 0 });
        assert_max_size_bound(&DfuCrcRewindRequest { offset: 0, crc: 0 });
        assert_max_size_bound(&DfuVerifyRequest { crc32: 0 });
    }

    #[test]
    fn dfu_data_max_fits_payload() {
        // DfuWriteRequest::POSTCARD_MAX_SIZE must not exceed RYNK_MAX_PAYLOAD_SIZE.
        assert!(
            DfuWriteRequest::POSTCARD_MAX_SIZE <= crate::protocol::rynk::message::RYNK_MAX_PAYLOAD_SIZE,
            "DfuWriteRequest ({} bytes) exceeds RYNK_MAX_PAYLOAD_SIZE ({} bytes)",
            DfuWriteRequest::POSTCARD_MAX_SIZE,
            crate::protocol::rynk::message::RYNK_MAX_PAYLOAD_SIZE,
        );
    }
}
