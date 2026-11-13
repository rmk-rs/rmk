//! Rynk over BLE GATT. Each connection owns its incoming bytes and reply route.

use core::cell::Cell;

use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::pipe::Pipe;
use embedded_io_async::{ErrorType, Write};
use heapless::Vec;
use rmk_types::protocol::rynk::{RYNK_BLE_CHUNK_SIZE, RYNK_HID_REPORT_SIZE};
use trouble_host::prelude::*;

use super::GattWriteResult;
use crate::ble::ble_server::{RynkGattService, RynkHidService, Server};
use crate::host::rynk::RynkService;
use crate::host::transport::HostTransportError;

/// Per-connection GATT write dispatcher for the two Rynk BLE transports.
pub(crate) struct HostConnection<'a> {
    custom: &'a RynkGattService,
    hid: &'a RynkHidService,
    // Configuration writes select the reply route because the OS can subscribe to HID on its own.
    reply_over_hid: Cell<bool>,
    pipe: Pipe<NoopRawMutex, 512>,
}

impl<'a> HostConnection<'a> {
    pub(crate) fn new(server: &'a Server<'_>) -> Self {
        Self {
            custom: &server.rynk_service,
            hid: &server.rynk_hid_service,
            reply_over_hid: Cell::new(false),
            pipe: Pipe::new(),
        }
    }

    pub(crate) async fn handle_write(&self, handle: u16, data: &[u8], encrypted: bool) -> GattWriteResult {
        let hid = handle == self.hid.output_data.handle;
        if hid || handle == self.custom.output_data.handle {
            if !encrypted {
                warn!("Rynk: dropping {}-byte write on unencrypted link", data.len());
                return GattWriteResult::Handled;
            }
            if hid && data.len() != RYNK_HID_REPORT_SIZE {
                warn!("Wrong Rynk HID report size: {}", data.len());
                return GattWriteResult::Handled;
            }
            if !data.is_empty() {
                self.reply_over_hid.set(hid);
                self.pipe.write_all(data).await;
            }
            GattWriteResult::Handled
        } else if self.custom.input_data.cccd_handle == Some(handle) || self.hid.input_data.cccd_handle == Some(handle)
        {
            GattWriteResult::CccdWrite
        } else if handle == self.hid.hid_control_point.handle {
            GattWriteResult::HidControlPoint
        } else {
            GattWriteResult::Unhandled
        }
    }

    pub(crate) async fn run<P: PacketPool>(&self, conn: &GattConnection<'_, '_, P>, service: &RynkService<'_>) {
        let mut reader = &self.pipe;
        let mut tx = RynkBleTx { host: self, conn };
        service.run(&mut reader, &mut tx).await;
    }
}

/// Write half: routes each reply/topic frame to the active transport's
/// characteristic — MTU-chunked on the custom char, or fixed 32-byte report
/// fragments on the HID char.
struct RynkBleTx<'a, 'b, 'c, P: PacketPool> {
    host: &'a HostConnection<'a>,
    conn: &'a GattConnection<'b, 'c, P>,
}

impl<P: PacketPool> ErrorType for RynkBleTx<'_, '_, '_, P> {
    type Error = HostTransportError;
}

impl<P: PacketPool> Write for RynkBleTx<'_, '_, '_, P> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if self.host.reply_over_hid.get() {
            for chunk in buf.chunks(RYNK_HID_REPORT_SIZE) {
                let mut report = [0u8; RYNK_HID_REPORT_SIZE];
                report[..chunk.len()].copy_from_slice(chunk);
                if let Err(e) = self.host.hid.input_data.notify(self.conn, &report, true).await {
                    error!("Failed to notify Rynk HID reply: {:?}", e);
                    return Err(HostTransportError);
                }
            }
        } else {
            // ATT notifications have a three-byte header. Larger payloads would be truncated.
            let chunk_size = RYNK_BLE_CHUNK_SIZE.min(usize::from(self.conn.raw().att_mtu()) - 3);
            for chunk in buf.chunks(chunk_size) {
                let payload =
                    Vec::<u8, RYNK_BLE_CHUNK_SIZE>::from_slice(chunk).expect("chunk size <= RYNK_BLE_CHUNK_SIZE");
                if let Err(e) = self.host.custom.input_data.notify(self.conn, &payload, true).await {
                    error!("Failed to notify Rynk reply: {:?}", e);
                    return Err(HostTransportError);
                }
            }
        }
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use embassy_sync::pipe::Pipe;
    use rmk_types::protocol::rynk::{Cmd, Deframer, RYNK_HEADER_SIZE, RynkHeader, encode_frame};

    use super::*;

    #[test]
    fn classifies_rynk_gatt_handles() {
        use crate::test_support::test_block_on as block_on;

        let server = Server::new_default("rmk").unwrap();
        let handler = HostConnection::new(&server);

        assert_eq!(
            block_on(handler.handle_write(
                server.rynk_hid_service.output_data.handle,
                &[0; RYNK_HID_REPORT_SIZE],
                true
            )),
            GattWriteResult::Handled
        );
        assert!(handler.reply_over_hid.get());
        assert_eq!(
            block_on(handler.handle_write(server.rynk_service.output_data.handle, &[1], true)),
            GattWriteResult::Handled
        );
        assert!(!handler.reply_over_hid.get());
        assert_eq!(
            block_on(handler.handle_write(server.rynk_service.input_data.cccd_handle.unwrap(), &[], true,)),
            GattWriteResult::CccdWrite
        );
        assert_eq!(
            block_on(handler.handle_write(server.rynk_hid_service.input_data.cccd_handle.unwrap(), &[], true,)),
            GattWriteResult::CccdWrite
        );
        assert_eq!(
            block_on(handler.handle_write(server.rynk_hid_service.hid_control_point.handle, &[0], true,)),
            GattWriteResult::HidControlPoint
        );
        assert_eq!(
            block_on(handler.handle_write(u16::MAX, &[], true)),
            GattWriteResult::Unhandled
        );
    }

    /// Build a COBS frame carrying `payload` into `buf`; returns its length.
    fn cobs_frame(buf: &mut [u8], seq: u8, payload: &[u8]) -> usize {
        encode_frame(
            buf,
            RynkHeader {
                cmd: Cmd::GetMacro,
                seq,
            },
            &payload,
        )
        .unwrap()
    }

    /// Read one frame back out of `pipe` via a Deframer (the connection's
    /// reassembler), returning its seq and decoded payload.
    fn read_one_frame(pipe: &Pipe<NoopRawMutex, 512>) -> (u8, Vec<u8, 64>) {
        use crate::test_support::test_block_on as block_on;

        let mut buf = [0u8; 256];
        let mut df = Deframer::new();
        loop {
            let n = block_on(pipe.read(df.tail(&mut buf)));
            df.commit(n);
            if let Some(frame_len) = df.next(&mut buf) {
                let frame = &buf[..frame_len];
                return (frame[2], postcard::from_bytes(&frame[RYNK_HEADER_SIZE..]).unwrap());
            }
        }
    }

    /// Fragment a frame into zero-padded 32-byte reports fed to the shared pipe
    /// as `handle_write`'s WebHID arm does; the padding drops out as empty frames.
    #[test]
    fn fragments_reassemble_through_pipe_for_session() {
        let pipe = Pipe::<NoopRawMutex, 512>::new();
        let payload: [u8; 60] = core::array::from_fn(|i| i as u8);
        let mut buf = [0u8; 128];
        let n = cobs_frame(&mut buf, 9, &payload);
        assert!(n > RYNK_HID_REPORT_SIZE, "frame must span multiple reports");

        for chunk in buf[..n].chunks(RYNK_HID_REPORT_SIZE) {
            let mut report = [0u8; RYNK_HID_REPORT_SIZE];
            report[..chunk.len()].copy_from_slice(chunk);
            assert_eq!(pipe.try_write(&report).unwrap(), report.len());
        }

        let (seq, decoded) = read_one_frame(&pipe);
        assert_eq!(seq, 9);
        assert_eq!(&decoded[..], &payload[..]);
    }

    /// A frame smaller than one report: the report's trailing zero-padding is
    /// skipped by the Deframer, leaving just the frame.
    #[test]
    fn small_frame_drops_padding() {
        let pipe = Pipe::<NoopRawMutex, 512>::new();
        let mut buf = [0u8; 64];
        let n = cobs_frame(&mut buf, 3, &[0xAA, 0xBB]);
        let mut report = [0u8; RYNK_HID_REPORT_SIZE];
        report[..n].copy_from_slice(&buf[..n]);
        assert_eq!(pipe.try_write(&report).unwrap(), report.len());

        let (seq, decoded) = read_one_frame(&pipe);
        assert_eq!(seq, 3);
        assert_eq!(&decoded[..], &[0xAA, 0xBB]);
    }
}
