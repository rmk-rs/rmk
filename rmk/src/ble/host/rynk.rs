//! Rynk config over BLE GATT — only the active profile's connection may drive
//! the shared [`BleConfig`]. Background connections keep HID/LED/battery, but
//! their config writes are dropped in `handle_write` without an ATT error.

use core::cell::Cell;

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::pipe::Pipe;
use embedded_io_async::{ErrorType, Write};
use heapless::Vec;
use rmk_types::protocol::rynk::{RYNK_BLE_CHUNK_SIZE, RYNK_HID_REPORT_SIZE};
use trouble_host::prelude::*;

use super::HostWriteOutcome;
use crate::ble::ble_server::{RynkGattService, RynkHidService, Server};
use crate::host::rynk::RynkService;
use crate::host::transport::HostTransportError;
use crate::state::{CONNECTION_STATUS, current_profile};

pub(crate) struct BleConfig<'s> {
    service: &'s RynkService<'s>,
    buffer: Mutex<NoopRawMutex, [u8; crate::RYNK_BUFFER_SIZE]>,
    pipe: Pipe<NoopRawMutex, 512>,
}

impl<'s> BleConfig<'s> {
    pub(crate) fn new(service: &'s RynkService<'s>) -> Self {
        Self {
            service,
            buffer: Mutex::new([0; crate::RYNK_BUFFER_SIZE]),
            pipe: Pipe::new(),
        }
    }
}

/// Which BLE transport the host is using, so the session routes replies/topics
/// to the right characteristic. Set only on a config WRITE, never on a CCCD
/// subscribe — the OS HOGP driver auto-subscribes the HID input CCCD on bond,
/// which would mis-bind a native session's replies. Reset per connection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RynkBleSource {
    /// No transport yet — drop topic pushes (no subscriber).
    None,
    /// Custom 128-bit `RynkGattService` (native hosts).
    Custom,
    /// Vendor HID-over-GATT `RynkHidService` (browsers over WebHID).
    Hid,
}

/// Per-connection GATT write dispatcher for the two Rynk BLE transports.
pub(crate) struct HostGattHandler<'a, 's> {
    custom: &'a RynkGattService,
    hid: &'a RynkHidService,
    source: Cell<RynkBleSource>,
    profile: u8,
    config: Option<&'a BleConfig<'s>>,
}

impl<'a, 's> HostGattHandler<'a, 's> {
    pub(crate) fn new(server: &'a Server<'_>, profile: u8, config: Option<&'a BleConfig<'s>>) -> Self {
        Self {
            custom: &server.rynk_service,
            hid: &server.rynk_hid_service,
            source: Cell::new(RynkBleSource::None),
            profile,
            config,
        }
    }

    pub(crate) async fn handle_write(&self, handle: u16, data: &[u8], encrypted: bool) -> HostWriteOutcome {
        if handle == self.custom.output_data.handle {
            if !data.is_empty() {
                if encrypted {
                    debug!("Got Rynk packet ({} bytes)", data.len());
                    self.write_pipe(data, RynkBleSource::Custom).await;
                } else {
                    warn!("Rynk: dropping {}-byte write on unencrypted link", data.len());
                }
            }
            HostWriteOutcome::Handled
        } else if handle == self.custom.input_data.cccd_handle.expect("No CCCD for Rynk input") {
            // A subscription alone must not bind the reply transport. OS HOGP
            // drivers may subscribe automatically when restoring a bond.
            HostWriteOutcome::CccdUpdated
        } else if handle == self.hid.output_data.handle {
            if encrypted {
                if data.len() == RYNK_HID_REPORT_SIZE {
                    self.write_pipe(data, RynkBleSource::Hid).await;
                } else {
                    warn!("Wrong Rynk HID report size: {}", data.len());
                }
            } else {
                warn!("Rynk HID: dropping {}-byte write on unencrypted link", data.len());
            }
            HostWriteOutcome::Handled
        } else if handle == self.hid.input_data.cccd_handle.expect("No CCCD for Rynk HID input") {
            HostWriteOutcome::CccdUpdated
        } else if handle == self.hid.hid_control_point.handle {
            HostWriteOutcome::ControlPoint
        } else {
            HostWriteOutcome::Unhandled
        }
    }

    /// Gate on the active profile, then copy one transport chunk into the
    /// shared pipe. A switch cancels a blocked write instead of stalling the
    /// GATT task; the host retries after the profile becomes active again.
    async fn write_pipe(&self, data: &[u8], source: RynkBleSource) {
        let Some(config) = self.config else {
            return;
        };
        let mut status = CONNECTION_STATUS
            .receiver()
            .expect("connection status subscriber capacity exceeded");
        if let Either::Second(()) = select(
            status.get_and(|s| s.ble.profile != self.profile),
            config.pipe.write_all(data),
        )
        .await
        {
            self.source.set(source);
        }
    }

    /// Run Rynk sessions for one connection. The shared buffer is held only
    /// while this connection is the active profile; switching releases it and
    /// the next active connection continues with its own handshake flag.
    pub(crate) async fn run<P: PacketPool>(&self, conn: &GattConnection<'_, '_, P>) {
        let Some(config) = self.config else {
            return core::future::pending().await;
        };
        let mut status = CONNECTION_STATUS
            .receiver()
            .expect("connection status subscriber capacity exceeded");
        let mut handshaked = false;
        let mut tx = RynkBleTx {
            custom_input: self.custom.input_data.clone(),
            hid_input: self.hid.input_data,
            source: &self.source,
            conn,
        };
        loop {
            status.get_and(|s| s.ble.profile == self.profile).await;
            let mut buffer = config.buffer.lock().await;
            // A stale waiter must not clear bytes the new active connection
            // already wrote into the shared pipe.
            if current_profile() != self.profile {
                continue;
            }
            config.pipe.clear();
            let mut reader = &config.pipe;
            select(
                status.get_and(|s| s.ble.profile != self.profile),
                config
                    .service
                    .run_session_with_buffer(&mut reader, &mut tx, &mut buffer, &mut handshaked),
            )
            .await;
        }
    }
}

/// Write half: routes each reply/topic frame to the active transport's
/// characteristic — MTU-chunked on the custom char, or fixed 32-byte report
/// fragments on the HID char.
struct RynkBleTx<'a, 'b, 'c, P: PacketPool> {
    custom_input: Characteristic<Vec<u8, RYNK_BLE_CHUNK_SIZE>>,
    hid_input: Characteristic<[u8; RYNK_HID_REPORT_SIZE]>,
    source: &'a Cell<RynkBleSource>,
    conn: &'a GattConnection<'b, 'c, P>,
}

impl<P: PacketPool> ErrorType for RynkBleTx<'_, '_, '_, P> {
    type Error = HostTransportError;
}

impl<P: PacketPool> Write for RynkBleTx<'_, '_, '_, P> {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        match self.source.get() {
            RynkBleSource::Hid => {
                for chunk in buf.chunks(RYNK_HID_REPORT_SIZE) {
                    let mut report = [0u8; RYNK_HID_REPORT_SIZE];
                    report[..chunk.len()].copy_from_slice(chunk);
                    if let Err(e) = self.hid_input.notify(self.conn, &report, true).await {
                        error!("Failed to notify Rynk HID reply: {:?}", e);
                        return Err(HostTransportError);
                    }
                }
            }
            RynkBleSource::Custom => {
                // Raw, MTU-chunked — a notify past ATT_MTU − 3 is silently
                // truncated, not split, so a dropped tail would desync the host.
                let max_notify = (self.conn.raw().att_mtu() as usize).saturating_sub(3);
                let chunk_size = RYNK_BLE_CHUNK_SIZE.min(max_notify).max(1);
                for chunk in buf.chunks(chunk_size) {
                    let payload =
                        Vec::<u8, RYNK_BLE_CHUNK_SIZE>::from_slice(chunk).expect("chunk size <= RYNK_BLE_CHUNK_SIZE");
                    if let Err(e) = self.custom_input.notify(self.conn, &payload, true).await {
                        error!("Failed to notify Rynk reply: {:?}", e);
                        return Err(HostTransportError);
                    }
                }
            }
            // No transport established yet — drop (e.g. a topic emitted before
            // the host has written a request or subscribed for notifications).
            RynkBleSource::None => {}
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
        let handler = HostGattHandler::new(&server, 0, None);

        assert_eq!(
            block_on(handler.handle_write(server.rynk_service.output_data.handle, &[], true)),
            HostWriteOutcome::Handled
        );
        assert_eq!(
            block_on(handler.handle_write(server.rynk_hid_service.output_data.handle, &[], true,)),
            HostWriteOutcome::Handled
        );
        assert_eq!(
            block_on(handler.handle_write(server.rynk_service.input_data.cccd_handle.unwrap(), &[], true,)),
            HostWriteOutcome::CccdUpdated
        );
        assert_eq!(
            block_on(handler.handle_write(server.rynk_hid_service.input_data.cccd_handle.unwrap(), &[], true,)),
            HostWriteOutcome::CccdUpdated
        );
        assert_eq!(
            block_on(handler.handle_write(server.rynk_hid_service.hid_control_point.handle, &[0], true,)),
            HostWriteOutcome::ControlPoint
        );
        assert_eq!(
            block_on(handler.handle_write(u16::MAX, &[], true)),
            HostWriteOutcome::Unhandled
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

    /// Read one frame back out of `pipe` via a Deframer (the session's
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
