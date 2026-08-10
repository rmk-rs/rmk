//! Rynk wire-format message.
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
//! One frame is the 3-byte header plus a postcard-encoded payload.
//! The frame is COBS-encoded on the wire.

use postcard::ser_flavors::{Cobs, Flavor, Slice};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::RynkError;
use super::command::Cmd;

/// Size in bytes of the fixed Rynk header.
pub const RYNK_HEADER_SIZE: usize = 3;

/// The fixed header of a [`RynkMessage`].
#[derive(Debug, Clone, Copy)]
pub struct RynkHeader {
    pub cmd: Cmd,
    pub seq: u8,
}

impl RynkHeader {
    /// Decode the 3 header bytes.
    pub const fn parse(bytes: &[u8; RYNK_HEADER_SIZE]) -> Self {
        Self {
            cmd: Cmd::from_le_bytes([bytes[0], bytes[1]]),
            seq: bytes[2],
        }
    }

    /// Decode the Rynk header out of a COBS-encoded frame, leaving the frame bytes untouched.
    ///
    /// This is mainly used in the dongle to check the command.
    pub fn peek(encoded: &[u8]) -> Option<Self> {
        let mut out = [0u8; RYNK_HEADER_SIZE];
        let mut n = 0;
        let mut state = cobs::DecoderState::Idle;
        for &byte in encoded {
            match state.feed(byte).ok()? {
                cobs::DecodeResult::NoData => {}
                // The frame's delimiter arrived before a full header was decoded.
                cobs::DecodeResult::DataComplete => return None,
                cobs::DecodeResult::DataContinue(b) => {
                    out[n] = b;
                    n += 1;
                    if n == RYNK_HEADER_SIZE {
                        return Some(Self::parse(&out));
                    }
                }
            }
        }
        None
    }

    pub const fn to_bytes(&self) -> [u8; RYNK_HEADER_SIZE] {
        let cmd_bytes = self.cmd.to_le_bytes();
        [cmd_bytes[0], cmd_bytes[1], self.seq]
    }
}

/// Calculate worst-case wire size of a frame: streaming-COBS code bytes
/// plus the `0x00` delimiter. Inverse of `max_frame_size`.
pub const fn max_wire_size(frame_size: usize) -> usize {
    frame_size + frame_size / 254 + 2
}

/// Calculate largest logical frame size that can be COBS-encoded into a
/// physical buffer of the given size, delimiter included.
pub(crate) const fn max_frame_size(max_encoded_size: usize) -> usize {
    let mut len = 0;
    while max_wire_size(len + 1) <= max_encoded_size {
        len += 1;
    }
    len
}

/// Largest request/response payload either peer can carry in one frame.
pub const RYNK_MAX_PAYLOAD_SIZE: usize = {
    let frame_size = max_frame_size(crate::constants::RYNK_BUFFER_SIZE);
    // Assert before subtracting so a too-small buffer fails with this message,
    // not a const-eval underflow.
    assert!(
        frame_size >= RYNK_HEADER_SIZE,
        "rynk_buffer_size is too small for a COBS-framed header; increase it"
    );
    frame_size - RYNK_HEADER_SIZE
};

/// COBS-encode the frame `header ++ postcard(value)` into `buf`, returning the
/// total framed length. The header goes through the COBS encoder too, so a
/// `0x00` in it never aliases the delimiter.
pub fn encode_frame<T: Serialize>(buf: &mut [u8], header: RynkHeader, value: &T) -> Result<usize, RynkError> {
    let mut ser = postcard::Serializer {
        output: Cobs::try_new(Slice::new(buf)).map_err(|_| RynkError::Internal)?,
    };
    ser.output
        .try_extend(&header.to_bytes())
        .map_err(|_| RynkError::Internal)?;
    value.serialize(&mut ser).map_err(|_| RynkError::Internal)?;
    Ok(ser.output.finalize().map_err(|_| RynkError::Internal)?.len())
}

/// One page of bulk-response items. Serializes as a postcard seq — the wire
/// shape of a `Vec` of the items — streaming from the iterator instead of
/// materializing one.
struct BulkItems<I>(I);

impl<I> Serialize for BulkItems<I>
where
    I: ExactSizeIterator + Clone,
    I::Item: Serialize,
{
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = ser.serialize_seq(Some(self.0.len()))?;
        for item in self.0.clone() {
            seq.serialize_element(&item)?;
        }
        seq.end()
    }
}

/// One received frame, answered in place: wrap the decoded logical frame
/// (`[cmd, seq, payload]`), decode the request out of it, then `encode_*` the
/// reply over it. The header is cached because the reply overwrites the buffer.
pub struct RynkMessage<'a> {
    buf: &'a mut [u8],
    header: RynkHeader,
    /// Valid prefix of `buf`: the logical frame on arrival, the COBS-framed
    /// reply after `encode_*`.
    len: usize,
}

impl<'a> RynkMessage<'a> {
    /// Wrap a decoded logical frame occupying `buf[..len]`; the rest of `buf` is
    /// scratch the `encode_*` replies may grow into. The caller guarantees
    /// `len >= RYNK_HEADER_SIZE`, as [`Deframer::next`](super::Deframer::next) does.
    pub fn from_decoded(buf: &'a mut [u8], len: usize) -> Self {
        debug_assert!(len >= RYNK_HEADER_SIZE && len <= buf.len());
        let header = RynkHeader::parse(buf.first_chunk().unwrap());
        Self { buf, header, len }
    }

    /// The decoded header.
    pub const fn header(&self) -> RynkHeader {
        self.header
    }

    /// Physical bytes available to the reply — the whole wrapped buffer, which
    /// shrinks when a pipelined tail is parked behind it.
    pub fn capacity(&self) -> usize {
        self.buf.len()
    }

    /// The COBS-encoded reply frame, ready to transmit. Valid after `encode_*`.
    pub fn frame(&self) -> &[u8] {
        &self.buf[..self.len]
    }

    /// The decoded payload bytes. Valid on a received (decoded) message.
    pub fn payload(&self) -> &[u8] {
        &self.buf[RYNK_HEADER_SIZE..self.len]
    }

    /// Decode the request payload.
    pub fn decode_request<T: DeserializeOwned>(&self) -> Result<T, RynkError> {
        postcard::from_bytes(self.payload()).map_err(|_| RynkError::Malformed)
    }

    /// Encode `Ok(value)` as the response frame, replacing the buffer contents.
    pub fn encode_response<T: Serialize>(&mut self, value: &T) -> Result<(), RynkError> {
        self.len = encode_frame(self.buf, self.header, &Ok::<&T, RynkError>(value))?;
        Ok(())
    }

    /// Encode `Ok(items)` as a bulk response frame, streaming the page through
    /// the COBS encoder — no `Vec` needed.
    pub fn encode_bulk<I>(&mut self, items: I) -> Result<(), RynkError>
    where
        I: ExactSizeIterator + Clone,
        I::Item: Serialize,
    {
        self.len = encode_frame(self.buf, self.header, &Ok::<_, RynkError>(BulkItems(items)))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_frame_round_trips_through_cobs() {
        // GetVersion = 0x0001, so cmd_hi is 0x00 — a good test that COBS carries
        // an interior zero without it aliasing the delimiter.
        let mut buf = [0u8; 64];
        let framed_len = encode_frame(
            &mut buf,
            RynkHeader {
                cmd: Cmd::GetVersion,
                seq: 0x42,
            },
            &[1u8, 2, 3, 4],
        )
        .unwrap();
        // The wire frame is delimiter-terminated with no interior 0x00.
        assert_eq!(buf[framed_len - 1], 0);
        assert!(buf[..framed_len - 1].iter().all(|&b| b != 0));

        // Decode it back to the logical [cmd, seq, payload].
        let n = cobs::decode_in_place(&mut buf[..framed_len - 1]).unwrap();
        let header = RynkHeader::parse(buf[..RYNK_HEADER_SIZE].try_into().unwrap());
        assert_eq!(header.cmd, Cmd::GetVersion);
        assert_eq!(header.seq, 0x42);
        // postcard encodes [u8; 4] as 4 bare bytes.
        assert_eq!(&buf[RYNK_HEADER_SIZE..n], &[1, 2, 3, 4]);
    }

    #[test]
    fn encode_rejects_short_buffer() {
        // Too small to hold the COBS-framed 3-byte header.
        let mut buf = [0u8; 2];
        assert_eq!(
            encode_frame(
                &mut buf,
                RynkHeader {
                    cmd: Cmd::GetVersion,
                    seq: 0
                },
                &()
            )
            .err(),
            Some(RynkError::Internal),
        );
    }

    #[test]
    fn encode_bulk_streams_the_ok_sequence_shape() {
        // The streamed bulk reply must be byte-identical to a plain
        // `Ok(sequence)` value, or hosts can't decode it.
        let header = RynkHeader {
            cmd: Cmd::GetKeymapBulk,
            seq: 7,
        };
        let mut buf = [0u8; 64];
        buf[..RYNK_HEADER_SIZE].copy_from_slice(&header.to_bytes());
        let mut msg = RynkMessage::from_decoded(&mut buf, RYNK_HEADER_SIZE);
        msg.encode_bulk([1u8, 2, 3].into_iter().map(|b| b * 2)).unwrap();

        let mut expected = [0u8; 64];
        let n = encode_frame(&mut expected, header, &Ok::<&[u8], RynkError>(&[2, 4, 6])).unwrap();
        assert_eq!(msg.frame(), &expected[..n]);
    }

    #[test]
    fn decoded_payload_spans_header_to_len() {
        let mut buf = [0u8; 8];
        buf[0..2].copy_from_slice(&Cmd::SetDefaultLayer.to_le_bytes());
        buf[2] = 0x34;
        buf[3..].copy_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);

        let msg = RynkMessage::from_decoded(&mut buf, 8);
        assert_eq!(msg.header().cmd, Cmd::SetDefaultLayer);
        assert_eq!(msg.header().seq, 0x34);
        assert_eq!(msg.payload(), &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
    }

    #[test]
    fn max_frame_size_is_exact_for_streaming_cobs() {
        // COBS worst case is an all-nonzero frame. For each physical size,
        // max_frame_size bytes must stream-encode into it and one byte more
        // must not — the inverse of the encoder's worst case is exact, including
        // at the 254-block boundaries where streaming COBS costs one byte over
        // one-shot encoding.
        let nonzero = [0x41u8; 600];
        let encodes_into = |logical: usize, physical: usize| {
            let mut store = [0u8; 1024];
            let mut ser = postcard::Serializer {
                output: Cobs::try_new(Slice::new(&mut store[..physical])).unwrap(),
            };
            ser.output.try_extend(&nonzero[..logical]).is_ok() && ser.output.finalize().is_ok()
        };
        for physical in [
            2usize, 3, 4, 255, 256, 257, 258, 259, 480, 488, 509, 510, 511, 512, 513, 514,
        ] {
            let len = max_frame_size(physical);
            assert!(
                encodes_into(len, physical),
                "max_frame_size({physical}) = {len} must fit"
            );
            assert!(
                !encodes_into(len + 1, physical),
                "{} must not fit in {physical}",
                len + 1
            );
        }
        // Below one code byte + delimiter, nothing fits.
        assert_eq!(max_frame_size(0), 0);
        assert_eq!(max_frame_size(1), 0);
        assert_eq!(
            RYNK_MAX_PAYLOAD_SIZE,
            max_frame_size(crate::constants::RYNK_BUFFER_SIZE) - RYNK_HEADER_SIZE
        );
    }

    #[test]
    fn decode_header_reads_the_prefix_without_touching_the_frame() {
        // Headers with and without interior zeros, with and without payload —
        // the header must decode from the COBS prefix and the frame bytes stay put.
        for (cmd, seq, payload) in [
            (Cmd::GetVersion, 0x42u8, &[1u8, 2, 3, 4][..]), // cmd_hi = 0x00
            (Cmd::from_raw(0x0901), 0x00, &[][..]),         // seq = 0x00, empty payload
            (Cmd::from_raw(0x7FFF), 0xFF, &[0u8, 0][..]),   // no zeros in the header
        ] {
            let mut buf = [0u8; 64];
            let n = encode_frame(&mut buf, RynkHeader { cmd, seq }, &payload).unwrap();
            let copy = buf;
            let header = RynkHeader::peek(&buf[..n]).expect("header decodes");
            assert_eq!(header.cmd, cmd);
            assert_eq!(header.seq, seq);
            assert_eq!(buf, copy, "input frame must not be modified");
            // Also without the trailing delimiter, as a raw splitter may hand it over.
            let header = RynkHeader::peek(&buf[..n - 1]).unwrap();
            assert_eq!(header.cmd, cmd);
        }
    }

    #[test]
    fn decode_header_survives_a_254_byte_first_group() {
        // An all-nonzero frame longer than 254 bytes makes the first COBS group
        // 0xFF-coded; the header must not gain a phantom zero at the group edge.
        let payload = [0x41u8; 300];
        let mut buf = [0u8; 400];
        let header = RynkHeader {
            cmd: Cmd::from_raw(0x0101),
            seq: 7,
        };
        let n = encode_frame(&mut buf, header, &payload.as_slice()).unwrap();
        let decoded = RynkHeader::peek(&buf[..n]).unwrap();
        assert_eq!(decoded.cmd.raw(), 0x0101);
        assert_eq!(decoded.seq, 7);
    }

    #[test]
    fn decode_header_rejects_truncated_or_empty_input() {
        assert!(RynkHeader::peek(&[]).is_none());
        // A bare delimiter or a frame cut before 3 decoded bytes.
        assert!(RynkHeader::peek(&[0x00]).is_none());
        assert!(RynkHeader::peek(&[0x02, 0xAA]).is_none());
        assert!(
            RynkHeader::peek(&[0x03, 0xAA, 0xBB]).is_none(),
            "only 2 decoded bytes + EOF"
        );
    }

    #[test]
    fn frame_never_contains_the_delimiter() {
        // cmd 0x0004 has a zero byte; the encoded frame must not carry a bare 0x00
        // except the trailing delimiter.
        let mut buf = [0u8; 64];
        let n = encode_frame(
            &mut buf,
            RynkHeader {
                cmd: Cmd::from_raw(0x0004),
                seq: 0,
            },
            &[0u8, 0, 0],
        )
        .unwrap();
        assert_eq!(buf[n - 1], 0, "frame is delimiter-terminated");
        assert!(buf[..n - 1].iter().all(|&b| b != 0), "no interior 0x00");
    }
}
