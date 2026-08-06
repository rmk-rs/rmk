//! Ready-to-use compositor engine for ordinary RMK lighting.
//!
//! Boards provide static layer scenes plus optional extension and status
//! sources. The engine supplies a controllable uniform background, TTL host
//! overlay, deterministic priority bands, output brightness, frame history,
//! `LightAction` handling, and protocol-independent commands/readback.

/// Cells per overlay readback page. Kept equal to the wire chunk size so
/// protocol adapters can forward pages without re-batching.
pub const OVERLAY_CHUNK_SIZE: usize = 8;

/// Cells per scene page/replacement chunk. Kept equal to the wire chunk size
/// so protocol adapters can forward chunks without re-batching.
pub const SCENE_CHUNK_SIZE: usize = 8;

/// Cells per runtime conditional-scene page/replacement chunk.
pub const CONDITIONAL_SCENE_CHUNK_SIZE: usize = 7;

/// The engine pages conditional cells into a wire-sized `Vec`, so paging more
/// per page than the wire holds turns every readback into a rejected request.
/// Nothing tied the two together before, which is exactly how that happened.
// `core::assert!` explicitly: with defmt enabled the bare macro resolves to
// defmt's, which is not const-evaluable.
const _: () = core::assert!(
    CONDITIONAL_SCENE_CHUNK_SIZE == rmk_types::protocol::rynk::LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE,
    "engine conditional page size must equal the wire chunk size"
);

const _: () = core::assert!(
    command::FRAME_CHUNK_SIZE == rmk_types::protocol::rynk::LIGHTING_FRAME_CHUNK_SIZE,
    "engine frame page size must equal the wire chunk size"
);

/// A staged scene replacement expires after this much command inactivity.
pub const SCENE_TRANSACTION_TIMEOUT_MS: u64 = 5_000;

/// Stable default priority bands. Equal-priority call order remains stable.
pub mod priority {
    pub const BACKGROUND: u8 = 0;
    pub const EXTENSION: u8 = 32;
    pub const LAYER: u8 = 64;
    pub const HOST_OVERLAY: u8 = 128;
    pub const STATUS: u8 = 192;
}

mod background;
mod command;
mod conditional;
mod engine;
mod overlay;
mod scenes;
#[cfg(test)]
mod tests;

pub use background::*;
pub use command::*;
pub use conditional::*;
pub use engine::*;
pub use overlay::*;
pub use scenes::*;
