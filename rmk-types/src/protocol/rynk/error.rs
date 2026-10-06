//! The protocol-level error type.

use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Protocol-level error returned in response payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[non_exhaustive]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub enum RynkError {
    /// The request could not be decoded
    Malformed,
    /// Device is not currently in a state to satisfy the request
    NotReady,
    /// Persistent storage failed (flash read, write or erase error)
    StorageFault,
    /// Internal firmware fault.
    Internal,
    /// Command is recognized but the handler is not implemented yet.
    Unimplemented,
    /// The request decoded cleanly but is semantically invalid.
    Invalid,
    /// The frame is well-formed but its CMD is unknown.
    UnknownCmd,
    /// The command is gated by the lock and this session is locked.
    /// The host must complete the unlock ceremony (see `UnlockPoll`) first.
    Locked,
    /// Transient backpressure: the reply did not fit the buffer space left
    /// beside pipelined requests still queued in the session buffer. The
    /// request itself was valid — retry once in-flight requests complete.
    Busy,
}
