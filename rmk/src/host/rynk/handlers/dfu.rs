//! DFU handler — bridges rynk DFU commands to the internal `DfuCmdEvent` pubsub.
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
//!   running_crc       current_offset
//!   ───────────       ──────────────
//!   init: Crc32::new()        0
//!
//!   DfuWrite:          .update(data)    offset+len
//!
//!   DfuCrcSync:        —                —
//!     (on match: Ok; on mismatch: Err)
//!
//!   DfuCrcRewind:      from req         from req
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
use crate::dfu::{BLOCK_SIZE_DFU, DFU_WRITE_FAILED, DfuCmd, DfuTarget};
use crate::event::{DfuCmdEvent, DfuStatusEvent, publish_event, publish_event_async};

/// Firmware-side DFU state for CRC tracking.
pub(crate) struct DfuRynkState {
    running_crc: Crc32,
    current_offset: u32,
}

impl DfuRynkState {
    const fn new() -> Self {
        Self {
            running_crc: Crc32::new(),
            current_offset: 0,
        }
    }

    fn reset(&mut self) {
        self.running_crc = Crc32::new();
        self.current_offset = 0;
    }
}

/// Shared DFU state for the rynk path, guarded by a mutex.
///
/// This is locked per rynk request — the critical section is short (CRC
/// update + channel send) so contention is negligible.
pub(crate) static DFU_RYNK_STATE: Mutex<CriticalSectionRawMutex, DfuRynkState> = Mutex::new(DfuRynkState::new());

/// ProxyRynkDfuHandler bridges rynk DFU commands to the internal `DfuCmdEvent` pubsub,
/// mirroring the role of `ProxyUsbDfuHandler` for the USB path.
pub(crate) struct ProxyRynkDfuHandler;

/// Start a DFU download session. Resets CRC state, publishes
/// [`DfuCmd::Start(Central)`](DfuTarget::Central), and emits `DfuStatus::Started`.
impl Handle<DfuStart> for ProxyRynkDfuHandler {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        {
            let mut state = DFU_RYNK_STATE.lock().await;
            state.reset();
        }
        publish_event(DfuCmdEvent(DfuCmd::Start(DfuTarget::Central)));
        publish_event(DfuStatusEvent::new(DfuStatus::Started));
        info!("dfu_rynk: DFU download started");
        Ok(())
    }
}

/// Write a firmware data chunk. Returns `Err` immediately if a previous
/// write failed ([`DFU_WRITE_FAILED`]). Accumulates CRC under the lock,
/// then publishes [`DfuCmd::Write(Central)`](DfuTarget::Central) events
/// in `BLOCK_SIZE_DFU` pieces.
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

        // Split into BLOCK_SIZE_DFU chunks for the DfuCmdEvent pubsub.
        // This must happen outside the lock to avoid holding it across the
        // async channel send. Uses publish_event_async (backpressure) instead
        // of publish_event (drop on full): a full channel means the flash
        // writer is behind, and dropping a chunk here would corrupt the image
        // while the CRC still advances. Blocking propagates backpressure to
        // the host instead.
        for (i, chunk) in req.data.chunks(BLOCK_SIZE_DFU).enumerate() {
            let chunk_offset = req.offset + (i * BLOCK_SIZE_DFU) as u32;
            publish_event_async(DfuCmdEvent(DfuCmd::Write(
                DfuTarget::Central,
                chunk_offset,
                heapless::Vec::from_slice(chunk).map_err(|_| RynkError::Internal)?,
            )))
            .await;
        }

        publish_event(DfuStatusEvent::new(DfuStatus::Downloading));
        Ok(())
    }
}

/// Periodic CRC-32 check. Finalizes the running CRC and compares it to
/// the host's value. On match, returns `Ok`. On mismatch, returns
/// `Err(RynkError::Internal)`.
impl Handle<DfuCrcSync> for ProxyRynkDfuHandler {
    async fn handle(&self, req: DfuCrcSyncRequest) -> Result<(), RynkError> {
        let state = DFU_RYNK_STATE.lock().await;
        // finalize() is &self — running_crc continues accumulating across syncs.
        let actual_crc = state.running_crc.finalize();
        if actual_crc == req.expected_crc {
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

/// Rewind the CRC state to a previous checkpoint after a sync mismatch.
/// Restores `running_crc` from `req.crc` and resets `current_offset` so
/// the host can retransmit from `req.offset`.
impl Handle<DfuCrcRewind> for ProxyRynkDfuHandler {
    async fn handle(&self, req: DfuCrcRewindRequest) -> Result<(), RynkError> {
        let mut state = DFU_RYNK_STATE.lock().await;
        state.running_crc = Crc32::from_state(req.crc);
        state.current_offset = req.offset;
        info!("dfu_rynk: rewound to offset {} ({:#010x})", req.offset, req.crc);
        Ok(())
    }
}

/// End-of-transfer verification. Finalizes the running CRC and compares
/// it to the host's image-wide CRC. This only checks integrity — the
/// actual flash sanity check happens in [`DfuFinish`].
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

/// Finalize the DFU transfer. Publishes [`DfuCmd::Finish(Central)`](DfuTarget::Central)
/// which triggers the flash sanity check and reset in `FlashDfuHandler`.
impl Handle<DfuFinish> for ProxyRynkDfuHandler {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        publish_event(DfuCmdEvent(DfuCmd::Finish(DfuTarget::Central)));
        publish_event(DfuStatusEvent::new(DfuStatus::Finished));
        info!("dfu_rynk: DFU download complete");
        Ok(())
    }
}

/// Request a hard system reset without completing the DFU transfer.
/// Publishes [`DfuCmd::SystemReset(Central)`](DfuTarget::Central);
/// the firmware resets before it can reply.
impl Handle<DfuReset> for ProxyRynkDfuHandler {
    async fn handle(&self, _: ()) -> Result<(), RynkError> {
        publish_event(DfuCmdEvent(DfuCmd::SystemReset(DfuTarget::Central)));
        info!("dfu_rynk: system reset requested");
        Ok(())
    }
}

/// Dispatch a rynk DFU command to [`ProxyRynkDfuHandler`].
///
/// Used by the split peripheral's DFU GATT event loop to route incoming
/// rynk-framed DFU commands to the handler, which sends them through
/// [`DfuCmdEvent`] for processing by [`FlashDfuHandler`](crate::dfu::FlashDfuHandler).
pub(crate) async fn dispatch_dfu_cmd(cmd: Cmd, payload: &[u8]) -> Result<(), RynkError> {
    match cmd {
        Cmd::DfuStart => <ProxyRynkDfuHandler as Handle<DfuStart>>::handle(&ProxyRynkDfuHandler, ()).await,
        Cmd::DfuWrite => {
            let req = postcard::from_bytes::<DfuWriteRequest>(payload).map_err(|_| RynkError::Internal)?;
            <ProxyRynkDfuHandler as Handle<DfuWrite>>::handle(&ProxyRynkDfuHandler, req).await
        }
        Cmd::DfuCrcSync => {
            let req = postcard::from_bytes::<DfuCrcSyncRequest>(payload).map_err(|_| RynkError::Internal)?;
            <ProxyRynkDfuHandler as Handle<DfuCrcSync>>::handle(&ProxyRynkDfuHandler, req).await
        }
        Cmd::DfuCrcRewind => {
            let req = postcard::from_bytes::<DfuCrcRewindRequest>(payload).map_err(|_| RynkError::Internal)?;
            <ProxyRynkDfuHandler as Handle<DfuCrcRewind>>::handle(&ProxyRynkDfuHandler, req).await
        }
        Cmd::DfuVerify => {
            let req = postcard::from_bytes::<DfuVerifyRequest>(payload).map_err(|_| RynkError::Internal)?;
            <ProxyRynkDfuHandler as Handle<DfuVerify>>::handle(&ProxyRynkDfuHandler, req).await
        }
        Cmd::DfuFinish => <ProxyRynkDfuHandler as Handle<DfuFinish>>::handle(&ProxyRynkDfuHandler, ()).await,
        Cmd::DfuReset => <ProxyRynkDfuHandler as Handle<DfuReset>>::handle(&ProxyRynkDfuHandler, ()).await,
        _ => Err(RynkError::UnknownCmd),
    }
}
