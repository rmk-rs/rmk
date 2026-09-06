//! DFU handler — bridges rynk DFU commands to the internal `DFU_CHANNEL`.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                  BLE DFU OVER RYNK                                  │
//! │   (host: rynk-wtf,  firmware: ProxyRynkDfuHandler)                 │
//! └─────────────────────────────────────────────────────────────────────┘
//!
//!
//! ═══ PHASE 0: CAPABILITIES + UNLOCK ──────────────────────────────────
//!
//! ```text
//!   Host                              Firmware
//!     │                                   │
//!     │  GetCapabilities                  │
//!     │<── { dfu_enabled: true,           │
//!     │      max_payload_size: N } ───────┤
//!     │                                   │
//!     │  GetLockStatus                    │
//!     │<── { locked: true,                │
//!     │      key_positions: [0,1] } ──────┤
//!     │                                   │
//!     │  (user holds keys)                │
//!     │  UnlockPoll                       │
//!     │<── { locked: false } ─────────────┤
//! ```
//!
//!
//! ═══ PHASE 1: TRANSFER ── chunked CRC-32 ─────────────────────────────
//!
//! ```text
//!   Host                              Firmware
//!     │                                   │
//!     │  DfuStart                         │
//!     │  → DFU_CHANNEL.send(Start)        │
//!     │<── Ok ────────────────────────────┤
//!     │                                   │
//!     │  for chunk in firmware:           │
//!     │    DfuWrite{offset, data}         │
//!     │    → running_crc.update(data)     │
//!     │    → DFU_CHANNEL.send(Write)      │
//!     │<── Ok ────────────────────────────┤
//!     │                                   │
//!     │  (every N chunks)                 │
//!     │  DfuCrcSync{expected_crc}         │
//!     │    firmware CRC == host CRC?      │
//!     │      ├─ Yes → checkpoint, Ok      │
//!     │      └─ No  → Err(Internal) ──────┤
//!     │                                   │
//! ```
//!
//!
//! ═══ PHASE 1b: REWIND (on CRC mismatch) ─────────────────────────────
//!
//! After a CRC mismatch the host rewinds the firmware's running CRC and
//! write offset to the last known good checkpoint, then retransmits from
//! there.  Not to be confused with the rmk-boot rollback, which reverts
//! to a previous firmware image when `mark_booted` is not seen.
//!
//! ```text
//!   Host                              Firmware
//!     │                                   │
//!     │  DfuCrcRewind{offset, crc}        │
//!     │  → restore running_crc from crc   │
//!     │  → reset current_offset           │
//!     │<── Ok ────────────────────────────┤
//!     │                                   │
//!     │  (retransmit from checkpoint)     │
//!     │  DfuWrite{offset, data} ...       │
//!     │                                   │
//! ```
//!
//!
//! ═══ PHASE 2: VERIFY + FINISH ────────────────────────────────────────
//!
//! ```text
//!   Host                              Firmware
//!     │                                   │
//!     │  DfuVerify{crc32}                 │
//!     │    running_crc == host crc?       │
//!     │      ├─ Yes → Ok                  │
//!     │      └─ No  → Err(Internal) ──────┤
//!     │                                   │
//!     │  DfuFinish                        │
//!     │  → DFU_CHANNEL.send(Finish)       │
//!     │  → FlashDfuHandler: sanity check  │
//!     │<── Ok ────────────────────────────┤
//!     │                                   │
//!     │  DfuReset (optional)              │
//!     │  → DFU_CHANNEL.send(SystemReset)  │
//!     │                                   │
//! ```
//!
//!
//! ═══ CRC STATE MACHINE ═══════════════════════════════════════════════
//!
//! ```text
//!   running_crc       checkpoint_offset   checkpoint_crc   current_offset
//!   ───────────       ─────────────────   ──────────────   ──────────────
//!   init: Crc32::new()           0                  0                0
//!
//!   DfuWrite:          .update(data)  —                  —    offset+len
//!
//!   DfuCrcSync:        —              current_off   running_crc        —
//!     (on match: save checkpoint)
//!
//!   DfuCrcRewind:      from req       from req      from req          from req
//!
//!   DfuVerify:         .finalize() compared to host crc32
//! ```

use core::sync::atomic::Ordering;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use rmk_types::dfu::DfuStatus;
use rmk_types::protocol::rynk::command::{
    Cmd, DfuCrcRewind, DfuCrcSync, DfuFinish, DfuReset, DfuStart, DfuVerify, DfuWrite,
};
use rmk_types::protocol::rynk::{DfuCrcRewindRequest, DfuCrcSyncRequest, DfuVerifyRequest, DfuWriteRequest, RynkError};

use super::Handle;
use crate::crc32::Crc32;
use crate::dfu::{BLOCK_SIZE_DFU, DFU_CHANNEL, DFU_WRITE_FAILED, DfuCmd, DfuTarget};
use crate::event::{DfuStatusEvent, publish_event};

/// Firmware-side DFU state for CRC checkpoint/rewind.
struct DfuRynkState {
    running_crc: Crc32,
    checkpoint_offset: u32,
    checkpoint_crc: u32,
    current_offset: u32,
}

impl DfuRynkState {
    const fn new() -> Self {
        Self {
            running_crc: Crc32::new(),
            checkpoint_offset: 0,
            checkpoint_crc: 0,
            current_offset: 0,
        }
    }

    fn reset(&mut self) {
        self.running_crc = Crc32::new();
        self.checkpoint_offset = 0;
        self.checkpoint_crc = Crc32::new().finalize();
        self.current_offset = 0;
    }
}

/// Shared DFU state for the rynk path, guarded by a mutex.
///
/// This is locked per rynk request — the critical section is short (CRC
/// update + channel send) so contention is negligible.
pub(crate) static DFU_RYNK_STATE: Mutex<CriticalSectionRawMutex, DfuRynkState> = Mutex::new(DfuRynkState::new());

/// ProxyRynkDfuHandler bridges rynk DFU commands to the internal `DFU_CHANNEL`,
/// mirroring the role of `ProxyUsbDfuHandler` for the USB path.
pub(crate) struct ProxyRynkDfuHandler;

impl Handle<DfuStart> for ProxyRynkDfuHandler {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        {
            let mut state = DFU_RYNK_STATE.lock().await;
            state.reset();
        }
        DFU_CHANNEL
            .try_send(DfuCmd::Start(DfuTarget::Central))
            .map_err(|_| RynkError::Internal)?;
        #[cfg(feature = "dfu_lock")]
        crate::dfu::DFU_STARTED.store(true, Ordering::Release);
        publish_event(DfuStatusEvent::new(DfuStatus::Started));
        info!("dfu_rynk: DFU download started");
        Ok(())
    }
}

impl Handle<DfuWrite> for ProxyRynkDfuHandler {
    async fn handle(&self, req: DfuWriteRequest) -> Result<(), RynkError> {
        if DFU_WRITE_FAILED.load(Ordering::Acquire) {
            return Err(RynkError::Internal);
        }

        // Accumulate CRC and track offset under the lock.
        {
            let mut state = DFU_RYNK_STATE.lock().await;
            state.running_crc.update(&req.data);
            state.current_offset = req.offset + req.data.len() as u32;
        }

        // Split into BLOCK_SIZE_DFU chunks for the DFU_CHANNEL.
        // This must happen outside the lock to avoid holding it across the
        // async channel send.
        for (i, chunk) in req.data.chunks(BLOCK_SIZE_DFU).enumerate() {
            let mut buf = heapless::Vec::new();
            buf.extend_from_slice(chunk).map_err(|_| RynkError::Internal)?;
            let chunk_offset = req.offset + (i * BLOCK_SIZE_DFU) as u32;
            DFU_CHANNEL
                .send(DfuCmd::Write(DfuTarget::Central, chunk_offset, buf))
                .await;
        }

        publish_event(DfuStatusEvent::new(DfuStatus::Downloading));
        Ok(())
    }
}

impl Handle<DfuCrcSync> for ProxyRynkDfuHandler {
    async fn handle(&self, req: DfuCrcSyncRequest) -> Result<(), RynkError> {
        let mut state = DFU_RYNK_STATE.lock().await;
        let actual_crc = state.running_crc.finalize();
        if actual_crc == req.expected_crc {
            state.checkpoint_offset = state.current_offset;
            state.checkpoint_crc = actual_crc;
            info!(
                "dfu_rynk: CRC sync OK at offset {} ({:#010x})",
                state.current_offset, actual_crc
            );
            Ok(())
        } else {
            warn!(
                "dfu_rynk: CRC mismatch (expected {:#010x}, got {:#010x})",
                req.expected_crc, actual_crc
            );
            Err(RynkError::Internal)
        }
    }
}

impl Handle<DfuCrcRewind> for ProxyRynkDfuHandler {
    async fn handle(&self, req: DfuCrcRewindRequest) -> Result<(), RynkError> {
        let mut state = DFU_RYNK_STATE.lock().await;
        state.running_crc = Crc32::from_state(req.crc);
        state.current_offset = req.offset;
        info!("dfu_rynk: rewound to offset {} ({:#010x})", req.offset, req.crc);
        Ok(())
    }
}

impl Handle<DfuVerify> for ProxyRynkDfuHandler {
    async fn handle(&self, req: DfuVerifyRequest) -> Result<(), RynkError> {
        let state = DFU_RYNK_STATE.lock().await;
        let actual_crc = state.running_crc.finalize();
        drop(state);

        if actual_crc != req.crc32 {
            warn!(
                "dfu_rynk: verify CRC mismatch (expected {:#010x}, got {:#010x})",
                req.crc32, actual_crc
            );
            return Err(RynkError::Internal);
        }

        info!("dfu_rynk: verify OK ({:#010x})", actual_crc);
        Ok(())
    }
}

impl Handle<DfuFinish> for ProxyRynkDfuHandler {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        DFU_CHANNEL
            .try_send(DfuCmd::Finish(DfuTarget::Central))
            .map_err(|_| RynkError::Internal)?;
        publish_event(DfuStatusEvent::new(DfuStatus::Finished));
        info!("dfu_rynk: DFU download complete");
        Ok(())
    }
}

impl Handle<DfuReset> for ProxyRynkDfuHandler {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        DFU_CHANNEL
            .try_send(DfuCmd::SystemReset(DfuTarget::Central))
            .map_err(|_| RynkError::Internal)?;
        info!("dfu_rynk: system reset requested");
        Ok(())
    }
}

/// Dispatch a rynk DFU command to [`ProxyRynkDfuHandler`].
///
/// Used by the split peripheral's DFU GATT event loop to route incoming
/// rynk-framed DFU commands to the handler, which sends them through
/// [`DFU_CHANNEL`] for processing by [`FlashDfuHandler`](crate::dfu::FlashDfuHandler).
pub(crate) async fn dispatch_dfu_cmd(cmd: Cmd, payload: &[u8]) -> Result<(), RynkError> {
    match cmd {
        Cmd::DfuStart => ProxyRynkDfuHandler.handle(()).await,
        Cmd::DfuWrite => {
            let req = postcard::from_bytes::<DfuWriteRequest>(payload).map_err(|_| RynkError::Internal)?;
            ProxyRynkDfuHandler.handle(req).await
        }
        Cmd::DfuCrcSync => {
            let req = postcard::from_bytes::<DfuCrcSyncRequest>(payload).map_err(|_| RynkError::Internal)?;
            ProxyRynkDfuHandler.handle(req).await
        }
        Cmd::DfuCrcRewind => {
            let req = postcard::from_bytes::<DfuCrcRewindRequest>(payload).map_err(|_| RynkError::Internal)?;
            ProxyRynkDfuHandler.handle(req).await
        }
        Cmd::DfuVerify => {
            let req = postcard::from_bytes::<DfuVerifyRequest>(payload).map_err(|_| RynkError::Internal)?;
            ProxyRynkDfuHandler.handle(req).await
        }
        Cmd::DfuFinish => ProxyRynkDfuHandler.handle(()).await,
        Cmd::DfuReset => ProxyRynkDfuHandler.handle(()).await,
        _ => Err(RynkError::UnknownCmd),
    }
}
