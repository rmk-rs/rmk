//! Rynk host service — RMK-native protocol server.
//!
//! `RynkService` owns the global keyboard state and dispatch policy. Each
//! [`run_session`](RynkService::run_session) creates its own authorization gate
//! ([`HostLock`]) and topic subscriptions, so transports never share either.

mod handlers;
#[cfg(feature = "lighting")]
mod lighting;
mod topics;

use embassy_futures::select::{Either, select};
use embedded_io_async::{Read, Write};
use postcard::experimental::max_size::MaxSize;
#[cfg(feature = "lighting")]
pub use lighting::{
    RYNK_LIGHTING_TRANSACTION_CAPACITY, RynkLightingController, RynkLightingDescriptor, RynkLightingMailbox,
    StandardRynkLightingAdapter,
};
use rmk_types::constants::RYNK_BUFFER_SIZE;
use rmk_types::protocol::rynk::{
    Cmd, Deframer, RYNK_HEADER_SIZE, RynkError, RynkMessage, command, encode_frame, max_wire_size,
};

use self::handlers::{serve, serve_bulk};
use self::topics::TopicSubscribers;
use super::context::KeyboardContext;
use super::lock::HostLock;
use crate::config::{DeviceConfig, LockConfig, RmkConfig};
use crate::keymap::KeyMap;

/// Unlock attempts live long enough for BLE WebHID round trips.
const RYNK_UNLOCK_WINDOW: embassy_time::Duration = embassy_time::Duration::from_millis(500);

/// Transport-agnostic Rynk service.
pub struct RynkService<'a> {
    ctx: KeyboardContext<'a>,
    /// Device identity served by `GetDeviceInfo`.
    device: DeviceConfig<'static>,
    /// Policy copied into each session's authorization gate.
    lock_config: LockConfig,
    #[cfg(feature = "lighting")]
    lighting: Option<RynkLightingController<'a>>,
}

/// Per-session state that has to outlive a single dispatch. The authorization
/// gate and the topic table are locals in [`RynkService::run_session`]; the
/// lighting overlay transaction cannot be, because it spans the
/// Begin/Put/Commit exchange.
#[derive(Default)]
struct RynkSession {
    #[cfg(feature = "lighting")]
    lighting: embassy_sync::mutex::Mutex<crate::RawMutex, handlers::lighting::LightingTransactionState>,
}


impl<'a> RynkService<'a> {
    pub fn new(keymap: &'a KeyMap<'a>, config: &RmkConfig<'static>) -> Self {
        let mut ctx = KeyboardContext::new(keymap);
        // Layout is fixed at macro expansion time, like Vial's keyboard-def.
        ctx.layout_blob = config.layout_blob;
        Self {
            ctx,
            device: config.device_config,
            lock_config: config.lock_config,
            #[cfg(feature = "lighting")]
            lighting: None,
        }
    }

    /// Attach a concrete lighting controller. Merely compiling the lighting
    /// feature does not advertise support: discovery turns on only after this
    /// binding is present and its bridge task is running.
    #[cfg(feature = "lighting")]
    pub fn with_lighting(mut self, lighting: RynkLightingController<'a>) -> Self {
        self.lighting = Some(lighting);
        self
    }

    /// Whether `cmd` needs an unlocked device.
    fn requires_unlock(&self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::BootloaderJump | Cmd::StorageReset | Cmd::GetMatrixState => true,
            // Deleting a bond opens a re-pair hijack window; BLE-only command.
            #[cfg(feature = "_ble")]
            Cmd::ClearBleProfile => true,
            Cmd::SetKeyAction
            | Cmd::SetDefaultLayer
            | Cmd::SetEncoderAction
            | Cmd::SetMacro
            | Cmd::SetCombo
            | Cmd::SetMorse
            | Cmd::SetFork
            | Cmd::SetBehaviorConfig
            | Cmd::SetKeymapBulk
            | Cmd::SetComboBulk
            | Cmd::SetMorseBulk => self.lock_config.write_requires_unlock,
            #[cfg(feature = "lighting")]
            Cmd::SetLightingState
            | Cmd::SetLightingOverlay
            | Cmd::UnsetLightingOverlay
            | Cmd::ClearLightingOverlay
            | Cmd::BeginLightingOverlayReplace
            | Cmd::PutLightingOverlayChunk
            | Cmd::CommitLightingOverlayReplace
            | Cmd::AbortLightingOverlayReplace => self.lock_config.write_requires_unlock,
            _ => false,
        }
    }

    /// Serve one inbound message: on success the reply frame replaces the
    /// payload in place; on error the caller answers with the error envelope.
    async fn dispatch(
        &self,
        locker: &HostLock<'_>,
        session: &RynkSession,
        msg: &mut RynkMessage<'_>,
    ) -> Result<(), RynkError> {
        let cmd = msg.header().cmd;

        if self.requires_unlock(cmd) && !locker.is_unlocked() {
            return Err(RynkError::Locked);
        }

        match cmd {
            Cmd::GetVersion => serve::<command::GetVersion, _>(self, msg).await,
            Cmd::GetCapabilities => serve::<command::GetCapabilities, _>(self, msg).await,
            Cmd::Reboot => serve::<command::Reboot, _>(self, msg).await,
            Cmd::BootloaderJump => serve::<command::BootloaderJump, _>(self, msg).await,
            Cmd::StorageReset => serve::<command::StorageReset, _>(self, msg).await,
            Cmd::GetLockStatus => serve::<command::GetLockStatus, _>(locker, msg).await,
            Cmd::UnlockPoll => serve::<command::UnlockPoll, _>(locker, msg).await,
            Cmd::Lock => serve::<command::Lock, _>(locker, msg).await,
            Cmd::GetDeviceInfo => serve::<command::GetDeviceInfo, _>(self, msg).await,

            Cmd::GetKeyAction => serve::<command::GetKeyAction, _>(self, msg).await,
            Cmd::SetKeyAction => serve::<command::SetKeyAction, _>(self, msg).await,
            Cmd::GetDefaultLayer => serve::<command::GetDefaultLayer, _>(self, msg).await,
            Cmd::SetDefaultLayer => serve::<command::SetDefaultLayer, _>(self, msg).await,
            Cmd::GetEncoderAction => serve::<command::GetEncoderAction, _>(self, msg).await,
            Cmd::SetEncoderAction => serve::<command::SetEncoderAction, _>(self, msg).await,
            Cmd::GetKeymapBulk => serve_bulk::<command::GetKeymapBulk, _>(self, msg).await,
            Cmd::SetKeymapBulk => serve_bulk::<command::SetKeymapBulk, _>(self, msg).await,

            Cmd::GetMacro => serve::<command::GetMacro, _>(self, msg).await,
            Cmd::SetMacro => serve::<command::SetMacro, _>(self, msg).await,

            Cmd::GetCombo => serve::<command::GetCombo, _>(self, msg).await,
            Cmd::SetCombo => serve::<command::SetCombo, _>(self, msg).await,
            Cmd::GetComboBulk => serve_bulk::<command::GetComboBulk, _>(self, msg).await,
            Cmd::SetComboBulk => serve_bulk::<command::SetComboBulk, _>(self, msg).await,
            Cmd::GetMorse => serve::<command::GetMorse, _>(self, msg).await,
            Cmd::SetMorse => serve::<command::SetMorse, _>(self, msg).await,
            Cmd::GetMorseBulk => serve_bulk::<command::GetMorseBulk, _>(self, msg).await,
            Cmd::SetMorseBulk => serve_bulk::<command::SetMorseBulk, _>(self, msg).await,

            Cmd::GetFork => serve::<command::GetFork, _>(self, msg).await,
            Cmd::SetFork => serve::<command::SetFork, _>(self, msg).await,

            Cmd::GetBehaviorConfig => serve::<command::GetBehaviorConfig, _>(self, msg).await,
            Cmd::SetBehaviorConfig => serve::<command::SetBehaviorConfig, _>(self, msg).await,

            Cmd::GetConnectionType => serve::<command::GetConnectionType, _>(self, msg).await,
            Cmd::GetConnectionStatus => serve::<command::GetConnectionStatus, _>(self, msg).await,
            #[cfg(feature = "_ble")]
            Cmd::GetBleStatus => serve::<command::GetBleStatus, _>(self, msg).await,
            #[cfg(feature = "_ble")]
            Cmd::SwitchBleProfile => serve::<command::SwitchBleProfile, _>(self, msg).await,
            #[cfg(feature = "_ble")]
            Cmd::ClearBleProfile => serve::<command::ClearBleProfile, _>(self, msg).await,

            Cmd::GetCurrentLayer => serve::<command::GetCurrentLayer, _>(self, msg).await,
            Cmd::GetMatrixState => serve::<command::GetMatrixState, _>(self, msg).await,
            #[cfg(feature = "_ble")]
            Cmd::GetBatteryStatus => serve::<command::GetBatteryStatus, _>(self, msg).await,
            #[cfg(feature = "split")]
            Cmd::GetPeripheralStatus => serve::<command::GetPeripheralStatus, _>(self, msg).await,
            Cmd::GetWpm => serve::<command::GetWpm, _>(self, msg).await,
            Cmd::GetSleepState => serve::<command::GetSleepState, _>(self, msg).await,
            Cmd::GetLedIndicator => serve::<command::GetLedIndicator, _>(self, msg).await,

            Cmd::GetLayout => serve::<command::GetLayout, _>(self, msg).await,

            #[cfg(feature = "lighting")]
            Cmd::GetLightingCapabilities => Serve::<command::GetLightingCapabilities, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::GetLightingState => Serve::<command::GetLightingState, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::SetLightingState => Serve::<command::SetLightingState, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::GetLightingKeys => Serve::<command::GetLightingKeys, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::GetLightingPhysicalKeys => Serve::<command::GetLightingPhysicalKeys, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::GetLightingLeds => Serve::<command::GetLightingLeds, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::GetLightingZones => Serve::<command::GetLightingZones, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::GetLightingZoneMemberships => Serve::<command::GetLightingZoneMemberships, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::GetLightingOutputs => Serve::<command::GetLightingOutputs, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::GetLightingRoutes => Serve::<command::GetLightingRoutes, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::SetLightingOverlay => Serve::<command::SetLightingOverlay, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::UnsetLightingOverlay => Serve::<command::UnsetLightingOverlay, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::ClearLightingOverlay => Serve::<command::ClearLightingOverlay, _>::serve(self, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::BeginLightingOverlayReplace => handlers::lighting::serve_begin(self, session, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::PutLightingOverlayChunk => handlers::lighting::serve_put(self, session, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::CommitLightingOverlayReplace => handlers::lighting::serve_commit(self, session, msg).await,
            #[cfg(feature = "lighting")]
            Cmd::AbortLightingOverlayReplace => handlers::lighting::serve_abort(self, session, msg).await,

            _ => Err(RynkError::UnknownCmd),
        }
    }

    /// Drive one rynk session based on embedded-io `rx`/`tx`.
    ///
    /// Owns frame reassembly/dispatch; transport setup and reconnect stay outside.
    pub async fn run_session<R: Read, T: Write>(&self, rx: &mut R, tx: &mut T) {
        let locker = HostLock::new(
            self.lock_config.unlock_keys,
            self.ctx.keymap,
            self.lock_config.insecure,
            RYNK_UNLOCK_WINDOW,
        );
        let mut topics = TopicSubscribers::new();
        let session = RynkSession::default();
        let mut buf = [0u8; RYNK_BUFFER_SIZE];
        let mut df = Deframer::new();
        // Mute topics until the client completes the version handshake.
        let mut handshaked = false;

        loop {
            // Serve all frames already in the buffer.
            // Because the input/output shares a same buffer, while a frame is
            // being served, the pipelined tail behind it is parked at the end
            // of the buffer, so back-to-back requests all get answers.
            while let Some(req_len) = df.next(&mut buf) {
                let parked = df.park_pending(&mut buf);
                let reply_capacity = buf.len() - parked;
                let mut msg = RynkMessage::from_decoded(&mut buf[..reply_capacity], req_len);
                let cmd = msg.header().cmd;
                if cmd.is_topic() {
                    // Hosts never send topic-range cmds; drop without a reply.
                    warn!("Rynk: dropping topic-range request {:?}", cmd);
                } else {
                    let served = self.dispatch(&locker, &session, &mut msg).await;
                    // The version handshake completes on GetCapabilities.
                    handshaked |= cmd == Cmd::GetCapabilities;
                    let written = match served {
                        Ok(()) => tx.write_all(msg.frame()).await,
                        Err(error) => {
                            // A parked tail shrinks the reply window, so an encode
                            // failure there is backpressure, not a firmware fault.
                            let error = if parked > 0 && error == RynkError::Internal {
                                RynkError::Busy
                            } else {
                                error
                            };
                            let mut err_frame = [0u8; max_wire_size(
                                RYNK_HEADER_SIZE + <Result<(), RynkError> as MaxSize>::POSTCARD_MAX_SIZE,
                            )];
                            let len =
                                encode_frame(&mut err_frame, msg.header(), &Err::<(), RynkError>(error)).unwrap_or(0);
                            tx.write_all(&err_frame[..len]).await
                        }
                    };
                    if written.is_err() {
                        return;
                    }
                }
                df.unpark_pending(&mut buf, parked);
            }

            // A half-received frame must be finished first.
            // If the buffer is empty, race a read against the next topic.
            if df.has_pending() {
                match rx.read(df.tail(&mut buf)).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => df.commit(n),
                }
            } else {
                match select(rx.read(df.tail(&mut buf)), topics.next_event()).await {
                    Either::First(Ok(0)) | Either::First(Err(_)) => return,
                    Either::First(Ok(n)) => df.commit(n),
                    Either::Second(event) => {
                        // Not handshaked yet: drain the event but don't forward it.
                        if !handshaked {
                            continue;
                        }
                        // The buffer is empty here, so encoding the topic is safe.
                        match event.encode(&mut buf) {
                            Ok(n) => {
                                if tx.write_all(&buf[..n]).await.is_err() {
                                    return;
                                }
                            }
                            Err(e) => warn!("Rynk topic encode failed: {:?}", e),
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::collections::VecDeque;
    use alloc::vec;
    use alloc::vec::Vec;

    use embassy_futures::join::join;
    use embedded_io_async::{ErrorKind, ErrorType, Read, Write};
    use rmk_types::action::KeyAction;
    use rmk_types::protocol::rynk::{
        LockStatus, MatrixState, ProtocolVersion, RYNK_HEADER_SIZE, RynkHeader, encode_frame,
    };

    use super::*;
    use crate::config::{BehaviorConfig, LockConfig, PositionalConfig, RmkConfig};
    use crate::event::KeyboardEvent;
    use crate::keymap::{KeyMap, KeymapData};
    use crate::test_support::test_block_on as block_on;

    /// Returns each item in `chunks` as a separate `read` call, with partial
    /// buffers handled by draining bytes from the head of the front chunk.
    /// Yields `Ok(0)` (EOF) once all chunks are drained.
    struct ChunkRead {
        chunks: VecDeque<Vec<u8>>,
    }

    impl ErrorType for ChunkRead {
        type Error = ErrorKind;
    }

    impl Read for ChunkRead {
        async fn read(&mut self, buf: &mut [u8]) -> Result<usize, Self::Error> {
            let Some(chunk) = self.chunks.front_mut() else {
                return Ok(0);
            };
            let n = chunk.len().min(buf.len());
            buf[..n].copy_from_slice(&chunk[..n]);
            chunk.drain(..n);
            if chunk.is_empty() {
                self.chunks.pop_front();
            }
            Ok(n)
        }
    }

    /// Captures every byte handed to `write` into a `Vec` for later assertion.
    struct VecWrite {
        captured: Vec<u8>,
    }

    impl ErrorType for VecWrite {
        type Error = ErrorKind;
    }

    impl Write for VecWrite {
        async fn write(&mut self, buf: &[u8]) -> Result<usize, Self::Error> {
            self.captured.extend_from_slice(buf);
            Ok(buf.len())
        }

        async fn flush(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    /// A COBS request frame with an empty payload, as a host sends it.
    fn req(cmd_raw: u16, seq: u8) -> Vec<u8> {
        req_with(cmd_raw, seq, &())
    }

    /// A COBS request frame carrying `payload`, as a host sends it.
    fn req_with<T: serde::Serialize>(cmd_raw: u16, seq: u8, payload: &T) -> Vec<u8> {
        let mut buf = [0u8; 64];
        let n = encode_frame(
            &mut buf,
            RynkHeader {
                cmd: Cmd::from_raw(cmd_raw),
                seq,
            },
            payload,
        )
        .unwrap();
        buf[..n].to_vec()
    }

    /// Decode a captured response stream into `(cmd, seq, decoded payload)` per
    /// frame, using the production Deframer.
    fn decode_frames(buf: &[u8]) -> Vec<(u16, u8, Vec<u8>)> {
        let mut work = buf.to_vec();
        let mut df = Deframer::new();
        df.commit(work.len());
        let mut out = Vec::new();
        while let Some(frame_len) = df.next(&mut work) {
            let frame = &work[..frame_len];
            out.push((
                u16::from_le_bytes([frame[0], frame[1]]),
                frame[2],
                frame[RYNK_HEADER_SIZE..].to_vec(),
            ));
        }
        out
    }

    /// A second session over the same service starts locked again. The gate
    /// itself lives in `tests/scenarios/rynk_lock.toml`, which holds the
    /// challenge with real matrix input; only the session boundary needs a
    /// second `run_session`, which one timeline cannot express.
    #[test]
    fn a_new_session_starts_locked_again() {
        let mut behavior = BehaviorConfig::default();
        let positional: PositionalConfig<2, 2> = PositionalConfig::default();
        let mut data: KeymapData<2, 2, 1, 0> =
            KeymapData::new([[[KeyAction::No, KeyAction::No], [KeyAction::No, KeyAction::No]]]);
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));

        const UNLOCK_KEYS: &[(u8, u8)] = &[(0, 0)];
        let config = RmkConfig {
            lock_config: LockConfig {
                unlock_keys: UNLOCK_KEYS,
                insecure: false,
                write_requires_unlock: false,
            },
            ..Default::default()
        };
        let service = RynkService::new(&keymap, &config);

        // Hold the challenge key throughout, so only the session boundary can
        // account for the second session being locked.
        keymap.update_matrix_state(&KeyboardEvent::key(0, 0, true));

        let mut chunks = VecDeque::new();
        chunks.push_back(req(Cmd::UnlockPoll.raw(), 0));
        chunks.push_back(req(Cmd::GetMatrixState.raw(), 1));
        let mut rx = ChunkRead { chunks };
        let mut tx = VecWrite { captured: Vec::new() };
        block_on(service.run_session(&mut rx, &mut tx));

        let resp = decode_frames(&tx.captured);
        assert!(
            postcard::from_bytes::<Result<MatrixState, RynkError>>(&resp[1].2)
                .unwrap()
                .is_ok(),
            "gated command served once unlocked"
        );

        let mut chunks2 = VecDeque::new();
        chunks2.push_back(req(Cmd::GetMatrixState.raw(), 0));
        let mut rx2 = ChunkRead { chunks: chunks2 };
        let mut tx2 = VecWrite { captured: Vec::new() };
        block_on(service.run_session(&mut rx2, &mut tx2));

        let resp2 = decode_frames(&tx2.captured);
        assert_eq!(resp2.len(), 1);
        assert_eq!(
            postcard::from_bytes::<Result<MatrixState, RynkError>>(&resp2[0].2).unwrap(),
            Err(RynkError::Locked),
            "a fresh session has independent locked state"
        );
    }

    #[test]
    fn sessions_authorize_independently() {
        let mut behavior = BehaviorConfig::default();
        let positional: PositionalConfig<2, 2> = PositionalConfig::default();
        let mut data: KeymapData<2, 2, 1, 0> =
            KeymapData::new([[[KeyAction::No, KeyAction::No], [KeyAction::No, KeyAction::No]]]);
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));

        const UNLOCK_KEYS: &[(u8, u8)] = &[(0, 0)];
        let mut config = RmkConfig::default();
        config.lock_config.unlock_keys = UNLOCK_KEYS;
        let service = RynkService::new(&keymap, &config);
        keymap.update_matrix_state(&KeyboardEvent::key(0, 0, true));

        let mut chunks_a = VecDeque::new();
        chunks_a.push_back(req(Cmd::UnlockPoll.raw(), 0x11));
        chunks_a.push_back(req(Cmd::GetLockStatus.raw(), 0x12));
        chunks_a.push_back(req(Cmd::Lock.raw(), 0x13));
        chunks_a.push_back(req(Cmd::GetMatrixState.raw(), 0x14));
        let mut rx_a = ChunkRead { chunks: chunks_a };
        let mut tx_a = VecWrite { captured: Vec::new() };

        let mut chunks_b = VecDeque::new();
        chunks_b.push_back(req(Cmd::GetLockStatus.raw(), 0x21));
        chunks_b.push_back(req(Cmd::UnlockPoll.raw(), 0x22));
        chunks_b.push_back(req(Cmd::GetMatrixState.raw(), 0x23));
        let mut rx_b = ChunkRead { chunks: chunks_b };
        let mut tx_b = VecWrite { captured: Vec::new() };

        block_on(join(
            service.run_session(&mut rx_a, &mut tx_a),
            service.run_session(&mut rx_b, &mut tx_b),
        ));

        let responses_a = decode_frames(&tx_a.captured);
        assert_eq!(responses_a.len(), 4);
        let unlocked_a = postcard::from_bytes::<Result<LockStatus, RynkError>>(&responses_a[0].2)
            .unwrap()
            .unwrap();
        assert!(!unlocked_a.locked);
        let status_a = postcard::from_bytes::<Result<LockStatus, RynkError>>(&responses_a[1].2)
            .unwrap()
            .unwrap();
        assert!(!status_a.locked);
        assert_eq!(
            postcard::from_bytes::<Result<(), RynkError>>(&responses_a[2].2).unwrap(),
            Ok(())
        );
        assert_eq!(
            postcard::from_bytes::<Result<MatrixState, RynkError>>(&responses_a[3].2).unwrap(),
            Err(RynkError::Locked),
        );

        let responses_b = decode_frames(&tx_b.captured);
        assert_eq!(responses_b.len(), 3);
        let locked_b = postcard::from_bytes::<Result<LockStatus, RynkError>>(&responses_b[0].2)
            .unwrap()
            .unwrap();
        assert!(locked_b.locked, "session A does not unlock session B");
        let unlocked_b = postcard::from_bytes::<Result<LockStatus, RynkError>>(&responses_b[1].2)
            .unwrap()
            .unwrap();
        assert!(!unlocked_b.locked);
        assert!(
            postcard::from_bytes::<Result<MatrixState, RynkError>>(&responses_b[2].2)
                .unwrap()
                .is_ok(),
            "locking session A does not relock session B"
        );
    }

    /// A read carrying more than one frame serves them all, in order: the
    /// pipelined tail is parked out of each reply's way, never dropped.
    #[test]
    fn run_session_serves_every_frame_of_a_coalesced_read() {
        let mut behavior = BehaviorConfig::default();
        let positional: PositionalConfig<1, 1> = PositionalConfig::default();
        let mut data: KeymapData<1, 1, 1, 0> = KeymapData::new([[[KeyAction::No]]]);
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let config = RmkConfig::default();
        let service = RynkService::new(&keymap, &config);

        let mut combined = req(Cmd::GetVersion.raw(), 0);
        combined.extend_from_slice(&req(Cmd::GetVersion.raw(), 1));

        let mut chunks = VecDeque::new();
        chunks.push_back(combined);

        let mut rx = ChunkRead { chunks };
        let mut tx = VecWrite { captured: Vec::new() };

        block_on(service.run_session(&mut rx, &mut tx));

        let resp = decode_frames(&tx.captured);
        assert_eq!(resp.len(), 2, "both coalesced frames are served");
        for (i, frame) in resp.iter().enumerate() {
            assert_eq!(frame.1, i as u8, "replies follow arrival order");
            assert_eq!(
                postcard::from_bytes::<Result<ProtocolVersion, RynkError>>(&frame.2).unwrap(),
                Ok(ProtocolVersion::CURRENT),
            );
        }
    }

    /// A frame split across reads survives the reply served before it completes.
    #[test]
    fn run_session_completes_a_partial_frame_after_serving() {
        let mut behavior = BehaviorConfig::default();
        let positional: PositionalConfig<1, 1> = PositionalConfig::default();
        let mut data: KeymapData<1, 1, 1, 0> = KeymapData::new([[[KeyAction::No]]]);
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let config = RmkConfig::default();
        let service = RynkService::new(&keymap, &config);

        let second = req(Cmd::GetVersion.raw(), 1);
        let split = second.len() / 2;
        let mut chunk1 = req(Cmd::GetVersion.raw(), 0);
        chunk1.extend_from_slice(&second[..split]);

        let mut chunks = VecDeque::new();
        chunks.push_back(chunk1);
        chunks.push_back(second[split..].to_vec());

        let mut rx = ChunkRead { chunks };
        let mut tx = VecWrite { captured: Vec::new() };

        block_on(service.run_session(&mut rx, &mut tx));

        let resp = decode_frames(&tx.captured);
        assert_eq!(resp.len(), 2, "the split frame completes and is served");
        assert_eq!(resp[0].1, 0);
        assert_eq!(resp[1].1, 1);
    }

    /// A bulk page shrinks to the reply window left beside a parked tail
    /// instead of failing, and stays non-empty.
    #[test]
    fn run_session_shrinks_a_bulk_page_beside_a_parked_tail() {
        let mut behavior = BehaviorConfig::default();
        let positional: PositionalConfig<8, 8> = PositionalConfig::default();
        let mut data: KeymapData<8, 8, 1, 0> = KeymapData::new([[[KeyAction::No; 8]; 8]]);
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let config = RmkConfig::default();
        let service = RynkService::new(&keymap, &config);

        // A bulk read followed by 450 delimiter-less bytes (a huge partial
        // frame) in one read: the parked tail squeezes the reply window.
        let mut chunk = req_with(Cmd::GetKeymapBulk.raw(), 0, &[0u8, 0, 0]);
        let tail = vec![0xEEu8; 450];
        let reply_capacity = RYNK_BUFFER_SIZE - tail.len();
        chunk.extend_from_slice(&tail);
        let mut chunks = VecDeque::new();
        chunks.push_back(chunk);

        let mut rx = ChunkRead { chunks };
        let mut tx = VecWrite { captured: Vec::new() };

        block_on(service.run_session(&mut rx, &mut tx));

        let expected = rmk_types::protocol::rynk::bulk_key_capacity(reply_capacity);
        assert!(
            0 < expected && expected < rmk_types::protocol::rynk::MAX_BULK_KEYS,
            "premise: the squeezed window holds a smaller, non-empty page"
        );
        let resp = decode_frames(&tx.captured);
        assert_eq!(resp.len(), 1);
        let page = postcard::from_bytes::<Result<heapless::Vec<KeyAction, 64>, RynkError>>(&resp[0].2)
            .unwrap()
            .unwrap();
        assert_eq!(page.len(), expected, "page sized to the actual window");
    }

    /// When the parked tail leaves no room for even the reply, the request is
    /// still answered — with `Busy`, built outside the squeezed window.
    #[test]
    fn run_session_answers_busy_when_the_reply_cannot_fit() {
        let mut behavior = BehaviorConfig::default();
        let positional: PositionalConfig<1, 1> = PositionalConfig::default();
        let mut data: KeymapData<1, 1, 1, 0> = KeymapData::new([[[KeyAction::No]]]);
        let keymap = block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        let config = RmkConfig::default();
        let service = RynkService::new(&keymap, &config);

        // Fill the whole buffer: a 5-byte GetVersion plus delimiter-less bytes.
        // The parked tail leaves a window too small even for the version reply.
        let mut chunk = req(Cmd::GetVersion.raw(), 9);
        chunk.extend_from_slice(&vec![0xEEu8; RYNK_BUFFER_SIZE - chunk.len()]);
        let mut chunks = VecDeque::new();
        chunks.push_back(chunk);

        let mut rx = ChunkRead { chunks };
        let mut tx = VecWrite { captured: Vec::new() };

        block_on(service.run_session(&mut rx, &mut tx));

        let resp = decode_frames(&tx.captured);
        assert_eq!(resp.len(), 1, "the request is answered despite the squeeze");
        assert_eq!(resp[0].1, 9, "seq echo");
        assert_eq!(
            postcard::from_bytes::<Result<ProtocolVersion, RynkError>>(&resp[0].2).unwrap(),
            Err(RynkError::Busy),
        );
    }
}
