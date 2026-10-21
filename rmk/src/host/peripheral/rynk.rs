//! Update-only Rynk service for a peripheral's own USB connection.

use embassy_time::Duration;
use embedded_io_async::{Read, Write};
use postcard::experimental::max_size::MaxSize;
use rmk_types::constants::RYNK_BUFFER_SIZE;
use rmk_types::protocol::rynk::{
    Cmd, Deframer, DeviceCapabilities, RYNK_HEADER_SIZE, RYNK_MAX_PAYLOAD_SIZE, RynkError, RynkMessage, command,
    encode_frame, max_wire_size,
};

use super::PeripheralService;
use crate::host::lock::HostLock;
use crate::host::rynk::handlers::{Handle, serve};

impl PeripheralService {
    async fn dispatch(&self, locker: &HostLock<'_>, msg: &mut RynkMessage<'_>) -> Result<(), RynkError> {
        let cmd = msg.header().cmd;
        if cmd == Cmd::BootloaderJump && !locker.take_unlock() {
            return Err(RynkError::Locked);
        }
        match cmd {
            Cmd::GetVersion => serve::<command::GetVersion, _>(&self.device, msg).await,
            Cmd::GetDeviceInfo => serve::<command::GetDeviceInfo, _>(&self.device, msg).await,
            Cmd::GetCapabilities => serve::<command::GetCapabilities, _>(self, msg).await,
            Cmd::GetLayout => serve::<command::GetLayout, _>(&self.layout_blob, msg).await,
            Cmd::GetLockStatus => serve::<command::GetLockStatus, _>(locker, msg).await,
            Cmd::UnlockPoll => serve::<command::UnlockPoll, _>(locker, msg).await,
            Cmd::Lock => serve::<command::Lock, _>(locker, msg).await,
            Cmd::Reboot => serve::<command::Reboot, _>(&self.device, msg).await,
            Cmd::BootloaderJump => serve::<command::BootloaderJump, _>(&self.device, msg).await,
            _ => Err(RynkError::UnknownCmd),
        }
    }

    /// Serve framed Rynk requests without subscribing to the central's topics.
    pub async fn run_session<R: Read, W: Write>(&self, rx: &mut R, tx: &mut W) {
        let locker = HostLock::new(
            self.config.unlock_keys,
            self,
            self.config.insecure,
            Duration::from_millis(500),
        );
        let mut buf = [0u8; RYNK_BUFFER_SIZE];
        let mut df = Deframer::new();
        loop {
            while let Some(req_len) = df.next(&mut buf) {
                let parked = df.park_pending(&mut buf);
                let capacity = buf.len() - parked;
                let mut msg = RynkMessage::from_decoded(&mut buf[..capacity], req_len);
                if !msg.header().cmd.is_topic() {
                    let written = match self.dispatch(&locker, &mut msg).await {
                        Ok(()) => tx.write_all(msg.frame()).await,
                        Err(error) => {
                            let error = if parked > 0 && error == RynkError::Internal {
                                RynkError::Busy
                            } else {
                                error
                            };
                            let mut reply = [0u8; max_wire_size(
                                RYNK_HEADER_SIZE + <Result<(), RynkError> as MaxSize>::POSTCARD_MAX_SIZE,
                            )];
                            let len = encode_frame(&mut reply, msg.header(), &Err::<(), RynkError>(error)).unwrap_or(0);
                            tx.write_all(&reply[..len]).await
                        }
                    };
                    if written.is_err() {
                        return;
                    }
                }
                df.unpark_pending(&mut buf, parked);
            }
            match rx.read(df.tail(&mut buf)).await {
                Ok(0) | Err(_) => return,
                Ok(n) => df.commit(n),
            }
            embassy_futures::yield_now().await;
        }
    }
}

impl Handle<command::GetCapabilities> for PeripheralService {
    async fn handle(&self, _: ()) -> Result<DeviceCapabilities, RynkError> {
        // No editable keymap or bulk configuration endpoints on a peripheral.
        Ok(DeviceCapabilities {
            is_split: true,
            max_payload_size: RYNK_MAX_PAYLOAD_SIZE as u16,
            ..Default::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use core::convert::Infallible;
    use core::sync::atomic::Ordering;

    use embassy_futures::block_on;
    use embedded_io_async::ErrorType;
    use rmk_types::protocol::rynk::{DeviceInfo, LayoutChunk, LockStatus, ProtocolVersion, RynkHeader};

    use super::*;
    use crate::config::{DeviceConfig, LockConfig, RmkConfig};
    use crate::event::KeyboardEvent;
    use crate::split::PeripheralMatrixConfig;

    struct Chunks<'a> {
        bytes: &'a [u8],
        size: usize,
    }

    impl ErrorType for Chunks<'_> {
        type Error = Infallible;
    }

    impl Read for Chunks<'_> {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Infallible> {
            let n = buf.len().min(self.size).min(self.bytes.len());
            buf[..n].copy_from_slice(&self.bytes[..n]);
            self.bytes = &self.bytes[n..];
            Ok(n)
        }
    }

    #[test]
    fn peripheral_rynk_frames_identity_and_one_use_unlock() {
        let config = RmkConfig {
            device_config: DeviceConfig {
                product_name: "Right",
                ..Default::default()
            },
            lock_config: LockConfig {
                unlock_keys: &[(2, 3), (3, 4)],
                ..Default::default()
            },
            layout_blob: b"shared layout",
            ..Default::default()
        };
        let commands = [
            Cmd::GetVersion,
            Cmd::BootloaderJump,
            Cmd::UnlockPoll,
            Cmd::GetDeviceInfo,
            Cmd::GetCapabilities,
            Cmd::GetLockStatus,
            Cmd::GetLayout,
            Cmd::SetDefaultLayer,
            Cmd::BootloaderJump,
            Cmd::BootloaderJump,
            Cmd::GetLockStatus,
        ];
        let mut input = std::vec::Vec::new();
        for (seq, cmd) in commands.into_iter().enumerate() {
            let mut frame = [0u8; 32];
            let header = RynkHeader { cmd, seq: seq as u8 };
            let n = if cmd == Cmd::GetLayout {
                encode_frame(&mut frame, header, &0u32)
            } else {
                encode_frame(&mut frame, header, &())
            }
            .unwrap();
            input.extend_from_slice(&frame[..n]);
        }
        for size in [1, usize::MAX] {
            for held in [true, false] {
                let service = PeripheralService::new(
                    &config,
                    PeripheralMatrixConfig {
                        rows: 2,
                        cols: 2,
                        row_offset: 2,
                        col_offset: 3,
                    },
                );
                if held {
                    service.matrix.borrow_mut().update(&KeyboardEvent::key(0, 0, true));
                    service.matrix.borrow_mut().update(&KeyboardEvent::key(1, 1, true));
                }
                let before = crate::boot::BOOTLOADER_JUMPS.load(Ordering::Relaxed);
                let mut rx = Chunks { bytes: &input, size };
                let mut output = [0u8; 2048];
                let capacity = output.len();
                let mut tx = output.as_mut_slice();
                block_on(service.run_session(&mut rx, &mut tx));
                let written = capacity - tx.len();
                let mut df = Deframer::new();
                df.commit(written);
                let mut replies = std::vec::Vec::new();
                while let Some(len) = df.next(&mut output) {
                    let msg = RynkMessage::from_decoded(&mut output, len);
                    assert_eq!(msg.header().cmd, commands[replies.len()]);
                    assert_eq!(msg.header().seq as usize, replies.len());
                    replies.push(msg.payload().to_vec());
                }
                assert_eq!(replies.len(), commands.len());
                assert_eq!(
                    postcard::from_bytes::<Result<ProtocolVersion, RynkError>>(&replies[0]).unwrap(),
                    Ok(ProtocolVersion::CURRENT)
                );
                assert_eq!(
                    postcard::from_bytes::<Result<(), RynkError>>(&replies[1]).unwrap(),
                    Err(RynkError::Locked)
                );
                assert_eq!(
                    postcard::from_bytes::<Result<DeviceInfo, RynkError>>(&replies[3])
                        .unwrap()
                        .unwrap()
                        .product_name
                        .as_str(),
                    "Right"
                );
                let capabilities = postcard::from_bytes::<Result<DeviceCapabilities, RynkError>>(&replies[4])
                    .unwrap()
                    .unwrap();
                assert_eq!(capabilities.num_layers, 0);
                assert!(
                    !capabilities.ble_enabled,
                    "the peripheral does not offer BLE host-profile commands"
                );
                let status = postcard::from_bytes::<Result<LockStatus, RynkError>>(&replies[5])
                    .unwrap()
                    .unwrap();
                assert_eq!(status.locked, !held, "queries preserve the local authorization");
                assert_eq!(status.key_positions.as_slice(), &[(2, 3), (3, 4)]);
                assert_eq!(
                    postcard::from_bytes::<Result<LayoutChunk, RynkError>>(&replies[6])
                        .unwrap()
                        .unwrap()
                        .bytes
                        .as_slice(),
                    b"shared layout"
                );
                assert_eq!(
                    postcard::from_bytes::<Result<(), RynkError>>(&replies[7]).unwrap(),
                    Err(RynkError::UnknownCmd)
                );
                assert_eq!(
                    postcard::from_bytes::<Result<(), RynkError>>(&replies[8]).unwrap(),
                    if held { Ok(()) } else { Err(RynkError::Locked) }
                );
                assert_eq!(
                    postcard::from_bytes::<Result<(), RynkError>>(&replies[9]).unwrap(),
                    Err(RynkError::Locked)
                );
                assert!(
                    postcard::from_bytes::<Result<LockStatus, RynkError>>(&replies[10])
                        .unwrap()
                        .unwrap()
                        .locked
                );
                assert_eq!(
                    crate::boot::BOOTLOADER_JUMPS.load(Ordering::Relaxed),
                    before + usize::from(held)
                );
            }
        }
    }
}
