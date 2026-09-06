//! DFU split firmware update — update split peripherals over the split link.
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────┐
//! │                      SPLIT DFU UPDATE                               │
//! │   (peripheral connects / is connected)                              │
//! └─────────────────────────────────────────────────────────────────────┘
//!
//!
//! ═══ PHASE 0: HANDSHAKE ── hash comparison ───────────────────────────
//! Do we need to update the peripheral?
//!
//! ```text
//!   Central                           Peripheral
//!     │                                   │
//!     │                                   ├── (on boot) FirmwareHashResponse(hash)
//!     │                                   │   <- read_embedded_firmware_hash()
//!     │                                   │   = CRC32(__vector_table..__veneer_limit)
//!     │                                   │
//!     │  ── or ──                         │
//!     │                                   │
//!     ├── FirmwareHashQuery ─────────────>│
//!     │<── FirmwareHashResponse(hash) ────┤
//!     │                                   │
//!     │  hash == expected_hash?           │
//!     │    ├─ Yes → STOP (up-to-date)     │
//!     │    └─ No  → ↓                     │
//!     │                                   │
//! ```
//!
//!
//! ═══ PHASE 1: CHUNK TRANSFER ── per-chunk CRC ────────────────────────
//!
//! Central calculates CRC of every chunk it sends, peripheral does as well
//! and sends the CRC back in a FirmwareChunkAck.  If they match, central
//! sends the next chunk; if not, retry.  Central also accumulates the
//! overall CRC (`central_crc.update(chunk)`).
//!
//! ```text
//!   (outer attempt loop: 1..3)
//!
//!   Central                           Peripheral
//!     │                                   │
//!     │ central_crc = Crc32::new()        │
//!     │                                   │
//!     │   for chunk in firmware[256]:     │
//!     │     chunk_crc = CRC32(chunk)      │
//!     │     central_crc.update(chunk)     │
//!     │                                   │
//!     │   retry = 0                       │
//!     │   ┌─ retry < 3 ─────────────────┐ │
//!     │   │                             │ │
//!     ├── FirmwareChunk{offset,data} ──>│ │
//!     │   │                             │ │
//!     │   │               handler.write_chunk()
//!     │   │               — incremental erase (page-by-page)
//!     │   │               — write to flash
//!     │   │               chunk_crc = CRC32(chunk) │
//!     │   │                             │ │
//!     │   │<─ FirmwareChunkAck{offset,──┤ │
//!     │   │         crc: chunk_crc}     │ │
//!     │   │                             │ │
//!     │   CRC match:                    │ │
//!     │   ack_crc == chunk_crc?         │ │
//!     │     ├─ Yes → next chunk         │ │
//!     │     └─ No  → retry++ ────── ────┘ │
//!     │                                   │
//!     │   All chunks acked?               │
//!     │     ├─ Yes → ↓                    │
//!     │     └─ No  → attempt++ → Phase 1  │
//! ```
//!
//!
//! ═══ PHASE 2: END-TO-END CRC ── flash readback ──────────────────────
//!
//! Central compares its accumulated CRC with the firmware hash (direct) or
//! with the peripheral's CRC (passthrough).  If ok it sends
//! FirmwareUpdateComplete.  Peripheral reads back the DFU partition from
//! flash and sends FirmwareCrcReport.  Central verifies; on match it sends
//! FirmwareCrcOk, peripheral marks the firmware valid and resets.
//!
//! ```text
//!   Central                           Peripheral
//!     │                                   │
//!     │ (direct only)                     │
//!     │ central_crc == expected_hash?     │
//!     │   ├─ No  → ABORT (binary bug!)    │
//!     │   └─ Yes → ↓                      │
//!     │                                   │
//!     ├── FirmwareUpdateComplete ────────>│
//!     │                                   │
//!     │                     handler.compute_dfu_crc()
//!     │                     = CRC32(whole DFU partition)
//!     │                     via flash readback (256B blocks)
//!     │                                   │
//!     │<── FirmwareCrcReport(dfu_crc) ────┤
//!     │                                   │
//!     │  (direct)  dfu_crc == expected_hash?
//!     │  (passthru) dfu_crc == central_crc?
//!     │    ├─ Yes → ↓                     │
//!     │    └─ No  → send CrcFail          │
//!     │         (direct: attempt++ → P1)  │
//!     │         (passthru: abort)         │
//!     │                                   │
//!     │    ├── FirmwareCrcOk ────────>│   │
//!     │    │                          │   │
//!     │    │                handler.mark_updated_and_reset()
//!     │    │               (only after CrcOk — never into corrupt FW)
//!     │    │                          │   │
//!     │    │<─ FirmwareUpdateConfirm ─┤   │
//!     │    │                          │   │
//!     │    │                          │   │
//!     │    └── CrcOk in 5s? ──────────│   │
//!     │         No → timeout → abort  │   │
//!     │                               │   │
//!     │    └── CrcFail in 5s? ───── ──┤   │
//!     │         No → timeout → abort ─┘   │
//! ```
//!
//!
//! ═══ RETRY SUMMARY ════════════════════════════════════════════════════
//!
//! ```text
//!   Layer           Max     Trigger                    Consequence
//!   ─────           ───     ───────                    ──────────
//!   Per-chunk       3×      Ack CRC mismatch           Re-send same chunk
//!                           or 2s timeout              (both paths)
//!
//!   Outer attempt   3×      Chunk never acked          Full restart of
//!   (direct only)           or E2E CRC mismatch        Phase 1 + 2
//!                           or CRC timeout             (send_firmware_update)
//!
//!   No retry        —       Central CRC != expected    Abort — binary
//!   (direct only)           (central_crc.finalize())   mismatch, not TX
//!
//!   Passthrough     —       Chunk fails after 3×       Abort immediately
//!                           or E2E CRC mismatch        (no outer retry)
//! ```
//!
//!
//! ═══ SAFETY GATES ════════════════════════════════════════════════════
//!
//! ```text
//!   Where               What                          Why
//!   ─────               ────                          ───
//!   Peripheral boot     mark_updated_and_reset()       Never boot into
//!                       only on FirmwareCrcOk          corrupt firmware
//!
//!   After all chunks    central_crc == expected_hash   Catch binary bugs
//!                                                      before asking peripheral
//!
//!   End-to-end          compute_dfu_crc()             Catch silent flash
//!                       flash readback                write errors
//!
//!   Per-chunk           CRC32(data) in Ack            Catch bitflips,
//!                                                      packet loss,
//!                                                      wrong offset
//! ```
mod central;
mod peripheral;

pub use central::{get_firmware_update_data, set_firmware_update_data};
pub use peripheral::read_embedded_firmware_hash;
