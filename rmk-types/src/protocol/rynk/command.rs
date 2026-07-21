//! The Rynk command identifier and the command table.
//!
//! [`Cmd`] is the 16-bit identifier carried in the header CMD field. The most
//! significant bit (`0x8000`) acts as a flag to identify "Topics":
//!
//! - `0x0000..=0x7FFF` (Bit 15 = 0): Request/Response pairs.
//! - `0x8000..=0xFFFF` (Bit 15 = 1): Topics (Server -> Host push).
//!

#[cfg(not(feature = "host"))]
use postcard::experimental::max_size::MaxSize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::message::{RynkHeader, encode_frame};
use super::{
    BehaviorConfig, BuildInfo, DeviceCapabilities, DeviceInfo, GetComboBulkRequest, GetComboBulkResponse,
    GetEncoderRequest, GetKeymapBulkRequest, GetKeymapBulkResponse, GetMacroRequest, GetMorseBulkRequest,
    GetMorseBulkResponse, KeyPosition, LayerState, LayoutChunk, LockStatus, MacroData, MatrixState, ProtocolVersion,
    RynkError, SetComboBulkRequest, SetComboRequest, SetEncoderRequest, SetForkRequest, SetKeyRequest,
    SetKeymapBulkRequest, SetMacroRequest, SetMorseBulkRequest, SetMorseRequest, StorageResetMode,
};
use crate::action::{EncoderAction, KeyAction};
#[cfg(feature = "_ble")]
use crate::battery::BatteryStatus;
#[cfg(feature = "_ble")]
use crate::ble::BleStatus;
use crate::combo::Combo;
use crate::connection::{ConnectionStatus, ConnectionType};
use crate::fork::Fork;
use crate::led_indicator::LedIndicator;
use crate::modifier::ModifierCombination;
use crate::morse::Morse;
#[cfg(feature = "split")]
use crate::protocol::rynk::PeripheralStatus;
#[cfg(feature = "lighting")]
use crate::protocol::rynk::{
    AbortLightingOverlayReplaceRequest, AbortLightingSceneReplaceRequest, BeginLightingOverlayReplaceRequest,
    BeginLightingSceneReplaceRequest, ClearLightingOverlayRequest, CommitLightingOverlayReplaceRequest,
    CommitLightingSceneReplaceRequest, LightingCapabilitiesResult, LightingChanged, LightingCompiledSceneStatusResult,
    LightingCompiledScenesPageResult, LightingConditionalSceneStatusResult, LightingConditionalScenesPageResult,
    LightingKeysPageResult, LightingLedsPageResult, LightingOutputsPageResult, LightingOverlayPageRequest,
    LightingOverlayPageResult, LightingOverlayTransactionResult, LightingPageRequest, LightingPhysicalKeysPageResult,
    LightingRoutesPageResult, LightingScenePageRequest, LightingSceneStatusResult, LightingSceneTransactionResult,
    LightingScenesPageResult, LightingStateResult, LightingUnitResult, LightingZoneMembershipsPageResult,
    LightingZonesPageResult, PutLightingOverlayChunkRequest, PutLightingSceneChunkRequest,
    SetLightingLayerPolicyRequest, SetLightingOverlayRequest, SetLightingSceneCellRequest, SetLightingStateRequest,
    UnsetLightingOverlayRequest, UnsetLightingSceneCellRequest,
};
#[cfg(all(feature = "_ble", feature = "split"))]
use crate::protocol::rynk::{SplitCentralLatencyPolicy, SplitCentralLatencyState};

/// CMD high bit marking a topic (server → host push).
const RYNK_TOPIC_BIT: u16 = 0x8000;

/// A request/response endpoint: its [`Cmd`] plus both payload types — the wire
/// schema and nothing else. Buffer sizing stays out of it: postcard is
/// slice-driven, so `MaxSize` matters only to the no-allocator firmware, which
/// folds it into `MAX_ENDPOINT_PAYLOAD` below.
pub trait Endpoint {
    const CMD: Cmd;
    type Request: Serialize + DeserializeOwned;
    type Response: Serialize + DeserializeOwned;
}

/// The command identifier carried in the header CMD field. The named
/// `Cmd` constants are generated from the `endpoints!`/`topics!` table below.
#[repr(transparent)]
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct Cmd(u16);

impl Cmd {
    /// Build a `Cmd` from its raw wire value.
    pub const fn from_raw(raw: u16) -> Self {
        Self(raw)
    }

    /// Build a `Cmd` from the header's little-endian CMD bytes.
    pub const fn from_le_bytes(bytes: [u8; 2]) -> Self {
        Self(u16::from_le_bytes(bytes))
    }

    /// Return the raw wire value.
    pub const fn raw(self) -> u16 {
        self.0
    }

    /// Return the header's little-endian CMD bytes.
    pub const fn to_le_bytes(self) -> [u8; 2] {
        self.0.to_le_bytes()
    }

    /// Returns `true` for topic / unsolicited push CMDs (high bit set).
    pub const fn is_topic(self) -> bool {
        self.0 & RYNK_TOPIC_BIT != 0
    }
}

impl core::fmt::Debug for Cmd {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Cmd(0x{:04x})", self.0)
    }
}

#[cfg(feature = "defmt")]
impl defmt::Format for Cmd {
    fn format(&self, fmt: defmt::Formatter) {
        defmt::write!(fmt, "Cmd(0x{=u16:04x})", self.0)
    }
}

/// A command-table row as data for the generated protocol reference. Not
/// feature-gated — the reference lists the whole protocol; a row's gate rides
/// along in [`attrs`](Self::attrs)/[`bulk`](Self::bulk). `#[cfg(test)]`, so
/// these stringified names never reach a firmware binary.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct EndpointMeta {
    pub name: &'static str,
    pub cmd: u16,
    pub request: &'static str,
    pub response: &'static str,
    /// Stringified row attributes: doc comments and the `cfg` gate.
    pub attrs: &'static str,
}

/// Push counterpart of [`EndpointMeta`]; test-only, same rules.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct TopicMeta {
    pub name: &'static str,
    pub cmd: u16,
    pub payload: &'static str,
    /// Stringified row attributes: doc comments and the `cfg` gate.
    pub attrs: &'static str,
}

/// Compile-time guard: Check whether the command value is unique.
const fn assert_unique(cmds: &[u16]) {
    let mut i = 0;
    while i < cmds.len() {
        let mut j = i + 1;
        while j < cmds.len() {
            core::assert!(cmds[i] != cmds[j], "duplicate CMD value in the command table");
            j += 1;
        }
        i += 1;
    }
}

/// Macro for defining the endpoint (request/response) table.
///
/// Rows are uniform `Name = cmd: Req => Resp;`. Under non-`host` builds the table
/// folds every request and wrapped response into `MAX_ENDPOINT_PAYLOAD`,
/// which the firmware buffer must hold; host builds skip the fold, since bulk
/// payloads are unbounded there and carry no `MaxSize`.
macro_rules! endpoints {
    ($( $(#[$meta:meta])* $name:ident = $cmd:literal : $req:ty => $resp:ty; )*) => {
        #[allow(non_upper_case_globals)]
        impl Cmd {
            $( $(#[$meta])* pub const $name: Self = Cmd::from_raw($cmd); )*
        }
        $(
            $(#[$meta])*
            pub enum $name {}
            $(#[$meta])*
            impl Endpoint for $name {
                const CMD: Cmd = Cmd::$name;
                type Request = $req;
                type Response = $resp;
            }
        )*
        const _: () = {
            $( core::assert!(!Cmd::from_raw($cmd).is_topic(), "request CMD value in the topic range"); )*
            assert_unique(&[$($cmd),*]);
        };
        /// Largest request-or-wrapped-response across the whole endpoint table
        /// (bulk included) — folded firmware-side only, where every payload is a
        /// bounded type with a `MaxSize`.
        #[cfg(not(feature = "host"))]
        #[allow(unused_doc_comments)] // row docs also land on the fold statements
        const MAX_ENDPOINT_PAYLOAD: usize = {
            let mut m = 0;
            $( $(#[$meta])* {
                let req = <$req as MaxSize>::POSTCARD_MAX_SIZE;
                if req > m { m = req; }
                let resp = <Result<$resp, RynkError> as MaxSize>::POSTCARD_MAX_SIZE;
                if resp > m { m = resp; }
            } )*
            m
        };
        /// Endpoint rows as data, in table order — the protocol-reference source
        /// (see [`EndpointMeta`]).
        #[cfg(test)]
        pub const ENDPOINT_META: &[EndpointMeta] = &[
            $( EndpointMeta {
                name: stringify!($name),
                cmd: $cmd,
                request: stringify!($req),
                response: stringify!($resp),
                attrs: stringify!($(#[$meta])*),
            }, )*
        ];
    };
}

/// Macro for defining the topic table. Topics have no marker types: the
/// generated [`TopicEvent`] union is the whole consumer side.
macro_rules! topics {
    ($( $(#[$meta:meta])* $name:ident = $cmd:literal : $payload:ty; )*) => {
        #[allow(non_upper_case_globals)]
        impl Cmd {
            $( $(#[$meta])* pub const $name: Self = Cmd::from_raw($cmd); )*
        }
        const _: () = {
            $( core::assert!(Cmd::from_raw($cmd).is_topic(), "topic CMD value outside the topic range"); )*
            assert_unique(&[$($cmd),*]);
        };
        /// Largest payload across the whole topic table — feeds the firmware
        /// buffer assertion below. Absent on `host` builds, which are alloc
        /// and need no bound.
        #[cfg(not(feature = "host"))]
        #[allow(unused_doc_comments)]
        const MAX_TOPIC_PAYLOAD: usize = {
            let mut m = 0;
            $( $(#[$meta])* {
                let p = <$payload as MaxSize>::POSTCARD_MAX_SIZE;
                if p > m { m = p; }
            } )*
            m
        };
        /// Topic rows as data, in table order (see [`ENDPOINT_META`]).
        #[cfg(test)]
        pub const TOPIC_META: &[TopicMeta] = &[
            $( TopicMeta {
                name: stringify!($name),
                cmd: $cmd,
                payload: stringify!($payload),
                attrs: stringify!($(#[$meta])*),
            }, )*
        ];

        /// A decoded topic push (server → host), one variant per row of the
        /// topic table above — generated from it. `Serialize` lets the host
        /// re-emit a decoded topic as JSON (every payload is already a wire type).
        #[derive(Debug, Clone, serde::Serialize)]
        #[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
        #[cfg_attr(feature = "wasm", tsify(into_wasm_abi))]
        pub enum TopicEvent {
            $( $(#[$meta])* $name($payload), )*
        }

        impl TopicEvent {
            /// Decode a topic frame's `payload` as the topic named by `cmd`.
            /// `None` for a `cmd` outside the topic table, or a payload that
            /// fails to decode. Trailing bytes are ignored.
            pub fn decode(cmd: Cmd, payload: &[u8]) -> Option<Self> {
                match cmd {
                    $( $(#[$meta])* Cmd::$name => postcard::take_from_bytes::<$payload>(payload)
                        .ok()
                        .map(|(v, _)| TopicEvent::$name(v)), )*
                    _ => None,
                }
            }

            /// Encode this event into `buf` as a topic frame (SEQ = 0).
            /// Returns the framed length; the caller sends `&buf[..len]`.
            pub fn encode(&self, buf: &mut [u8]) -> Result<usize, RynkError> {
                match self {
                    $( $(#[$meta])* TopicEvent::$name(v) =>
                        encode_frame(buf, RynkHeader { cmd: Cmd::$name, seq: 0 }, v), )*
                }
            }
        }
    };
}

// Define endpoints: `Name = value: Request => Response;`
endpoints! {
    // System (0x00xx); 0x0009 reserved for layout.
    GetVersion = 0x0001: () => ProtocolVersion;
    GetCapabilities = 0x0002: () => DeviceCapabilities;
    Reboot = 0x0003: () => ();
    BootloaderJump = 0x0004: () => ();
    StorageReset = 0x0005: StorageResetMode => ();

    // Lock gate. All three stay dispatchable while locked.
    /// Pure read of the current lock state — no side effects.
    GetLockStatus = 0x0006: () => LockStatus;
    /// Arms/refreshes the unlock attempt and samples the held challenge keys.
    UnlockPoll = 0x0007: () => LockStatus;
    /// Relock immediately.
    Lock = 0x0008: () => ();
    /// Get layout blob chunk. `u32` is the byte offset.
    GetLayout = 0x0009: u32 => LayoutChunk;
    /// Identity strings and USB ids; feature gating stays in `GetCapabilities`.
    GetDeviceInfo = 0x000A: () => DeviceInfo;
    /// Application-defined diagnostic build label; never used for compatibility.
    GetBuildInfo = 0x000B: () => BuildInfo;
    /// Ask the application to route a bootloader jump to one split peripheral.
    PeripheralBootloaderJump = 0x000C: u8 => ();

    // Keymap (0x01xx) — includes encoder.
    GetKeyAction = 0x0101: KeyPosition => KeyAction;
    SetKeyAction = 0x0102: SetKeyRequest => ();
    GetDefaultLayer = 0x0103: () => u8;
    SetDefaultLayer = 0x0104: u8 => ();
    GetEncoderAction = 0x0105: GetEncoderRequest => EncoderAction;
    SetEncoderAction = 0x0106: SetEncoderRequest => ();
    GetKeymapBulk = 0x0107: GetKeymapBulkRequest => GetKeymapBulkResponse;
    SetKeymapBulk = 0x0108: SetKeymapBulkRequest => ();

    // Macro (0x02xx).
    GetMacro = 0x0201: GetMacroRequest => MacroData;
    SetMacro = 0x0202: SetMacroRequest => ();

    // Combo (0x03xx).
    GetCombo = 0x0301: u8 => Combo;
    SetCombo = 0x0302: SetComboRequest => ();
    GetComboBulk = 0x0303: GetComboBulkRequest => GetComboBulkResponse;
    SetComboBulk = 0x0304: SetComboBulkRequest => ();

    // Morse (0x04xx).
    GetMorse = 0x0401: u8 => Morse;
    SetMorse = 0x0402: SetMorseRequest => ();
    GetMorseBulk = 0x0403: GetMorseBulkRequest => GetMorseBulkResponse;
    SetMorseBulk = 0x0404: SetMorseBulkRequest => ();

    // Fork (0x05xx).
    GetFork = 0x0501: u8 => Fork;
    SetFork = 0x0502: SetForkRequest => ();

    // Behavior (0x06xx).
    GetBehaviorConfig = 0x0601: () => BehaviorConfig;
    SetBehaviorConfig = 0x0602: BehaviorConfig => ();

    // Connection (0x07xx).
    GetConnectionType = 0x0701: () => ConnectionType;
    /// Full `ConnectionStatus` snapshot.
    GetConnectionStatus = 0x0702: () => ConnectionStatus;
    #[cfg(feature = "_ble")]
    GetBleStatus = 0x0703: () => BleStatus;
    #[cfg(feature = "_ble")]
    SwitchBleProfile = 0x0704: u8 => ();
    #[cfg(feature = "_ble")]
    ClearBleProfile = 0x0705: u8 => ();
    #[cfg(all(feature = "_ble", feature = "split"))]
    /// Read the volatile active-mode policy, current USB-power selection, and effective value.
    GetSplitCentralLatency = 0x0706: () => SplitCentralLatencyState;
    #[cfg(all(feature = "_ble", feature = "split"))]
    /// Replace the volatile policy. Each connection-event count must be `0..=499`.
    SetSplitCentralLatency = 0x0707: SplitCentralLatencyPolicy => SplitCentralLatencyState;

    // Status (0x08xx).
    GetCurrentLayer = 0x0801: () => u8;
    GetMatrixState = 0x0802: () => MatrixState;
    #[cfg(feature = "_ble")]
    GetBatteryStatus = 0x0803: () => BatteryStatus;
    #[cfg(feature = "split")]
    GetPeripheralStatus = 0x0804: u8 => PeripheralStatus;
    /// Latest WPM, sourced from the `WpmUpdate` topic snapshot.
    GetWpm = 0x0805: () => u16;
    /// Latest sleep flag, sourced from the `SleepState` topic snapshot.
    GetSleepState = 0x0806: () => bool;
    /// Latest HID LED bitmap, sourced from the `LedIndicatorChange` topic snapshot.
    GetLedIndicator = 0x0807: () => LedIndicator;
    /// Default layer and complete active-layer bitmap.
    GetLayerState = 0x0808: () => LayerState;
    /// Final resolved modifier bitmap used by the HID keyboard report.
    GetModifierState = 0x0809: () => ModifierCombination;

    // Lighting (0x09xx). Lighting-domain errors are nested inside Rynk's
    // outer protocol result so hosts retain precise rejection reasons.
    #[cfg(feature = "lighting")]
    GetLightingCapabilities = 0x0901: () => LightingCapabilitiesResult;
    #[cfg(feature = "lighting")]
    GetLightingState = 0x0902: () => LightingStateResult;
    #[cfg(feature = "lighting")]
    SetLightingState = 0x0903: SetLightingStateRequest => LightingStateResult;
    #[cfg(feature = "lighting")]
    GetLightingPhysicalKeys = 0x0904: LightingPageRequest => LightingPhysicalKeysPageResult;
    #[cfg(feature = "lighting")]
    GetLightingLeds = 0x0905: LightingPageRequest => LightingLedsPageResult;
    #[cfg(feature = "lighting")]
    GetLightingZones = 0x0906: LightingPageRequest => LightingZonesPageResult;
    #[cfg(feature = "lighting")]
    GetLightingZoneMemberships = 0x0907: LightingPageRequest => LightingZoneMembershipsPageResult;
    #[cfg(feature = "lighting")]
    GetLightingOutputs = 0x0908: LightingPageRequest => LightingOutputsPageResult;
    #[cfg(feature = "lighting")]
    GetLightingRoutes = 0x0909: LightingPageRequest => LightingRoutesPageResult;
    #[cfg(feature = "lighting")]
    SetLightingOverlay = 0x090A: SetLightingOverlayRequest => LightingStateResult;
    #[cfg(feature = "lighting")]
    UnsetLightingOverlay = 0x090B: UnsetLightingOverlayRequest => LightingStateResult;
    #[cfg(feature = "lighting")]
    ClearLightingOverlay = 0x090C: ClearLightingOverlayRequest => LightingStateResult;
    #[cfg(feature = "lighting")]
    BeginLightingOverlayReplace = 0x090D: BeginLightingOverlayReplaceRequest => LightingOverlayTransactionResult;
    #[cfg(feature = "lighting")]
    PutLightingOverlayChunk = 0x090E: PutLightingOverlayChunkRequest => LightingUnitResult;
    #[cfg(feature = "lighting")]
    CommitLightingOverlayReplace = 0x090F: CommitLightingOverlayReplaceRequest => LightingStateResult;
    #[cfg(feature = "lighting")]
    AbortLightingOverlayReplace = 0x0910: AbortLightingOverlayReplaceRequest => LightingUnitResult;
    /// Logical matrix keys are distinct from optional physical geometry.
    #[cfg(feature = "lighting")]
    GetLightingKeys = 0x0911: LightingPageRequest => LightingKeysPageResult;
    /// Scene discovery lives outside `LightingCapabilities`/`LightingState`
    /// so their postcard layout stays stable for existing hosts.
    #[cfg(feature = "lighting")]
    GetLightingSceneStatus = 0x0912: () => LightingSceneStatusResult;
    /// Scene pages are pinned to `LightingState.revision` for consistency.
    #[cfg(feature = "lighting")]
    GetLightingScenes = 0x0913: LightingScenePageRequest => LightingScenesPageResult;
    #[cfg(feature = "lighting")]
    SetLightingSceneCell = 0x0914: SetLightingSceneCellRequest => LightingStateResult;
    #[cfg(feature = "lighting")]
    UnsetLightingSceneCell = 0x0915: UnsetLightingSceneCellRequest => LightingStateResult;
    #[cfg(feature = "lighting")]
    BeginLightingSceneReplace = 0x0916: BeginLightingSceneReplaceRequest => LightingSceneTransactionResult;
    #[cfg(feature = "lighting")]
    PutLightingSceneChunk = 0x0917: PutLightingSceneChunkRequest => LightingUnitResult;
    #[cfg(feature = "lighting")]
    CommitLightingSceneReplace = 0x0918: CommitLightingSceneReplaceRequest => LightingStateResult;
    #[cfg(feature = "lighting")]
    AbortLightingSceneReplace = 0x0919: AbortLightingSceneReplaceRequest => LightingUnitResult;
    #[cfg(feature = "lighting")]
    SetLightingLayerPolicy = 0x091A: SetLightingLayerPolicyRequest => LightingStateResult;
    /// Overlay pages are pinned to `LightingState.revision` for consistency.
    #[cfg(feature = "lighting")]
    GetLightingOverlay = 0x091B: LightingOverlayPageRequest => LightingOverlayPageResult;
    /// Discover the immutable board-compiled layer-scene source.
    #[cfg(feature = "lighting")]
    GetLightingCompiledSceneStatus = 0x091C: () => LightingCompiledSceneStatusResult;
    /// Compiled-scene pages are pinned to the firmware topology revision.
    #[cfg(feature = "lighting")]
    GetLightingCompiledScenes = 0x091D: LightingPageRequest => LightingCompiledScenesPageResult;
    /// Discover immutable conditional lighting compiled from board config.
    #[cfg(feature = "lighting")]
    GetLightingConditionalSceneStatus = 0x091E: () => LightingConditionalSceneStatusResult;
    /// Conditional-scene pages are pinned to the firmware topology revision.
    #[cfg(feature = "lighting")]
    GetLightingConditionalScenes = 0x091F: LightingPageRequest => LightingConditionalScenesPageResult;
}

// Define topics: `Name = value: Payload;`
topics! {
    // Topics (0x80xx, server → host push).
    LayerChange = 0x8001: u8;
    WpmUpdate = 0x8002: u16;
    ConnectionChange = 0x8003: ConnectionStatus;
    SleepState = 0x8004: bool;
    LedIndicatorChange = 0x8005: LedIndicator;
    #[cfg(feature = "_ble")]
    BatteryStatusChange = 0x8006: BatteryStatus;
    #[cfg(feature = "lighting")]
    LightingChange = 0x8007: LightingChanged;
    // Final resolved modifier bitmap changed.
    ModifierChange = 0x8008: ModifierCombination;
}

/// The payload budget advertised to hosts must cover the largest payload
/// either table can produce.
#[cfg(not(feature = "host"))]
const _: () = core::assert!(
    super::message::RYNK_MAX_PAYLOAD_SIZE >= MAX_ENDPOINT_PAYLOAD
        && super::message::RYNK_MAX_PAYLOAD_SIZE >= MAX_TOPIC_PAYLOAD,
    "rynk_buffer_size is too small to hold the largest rynk frame (including bulk and COBS overhead); increase it"
);

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::format;

    use postcard::experimental::max_size::MaxSize;

    use super::*;
    use crate::protocol::rynk::{Deframer, RYNK_HEADER_SIZE, RynkError, RynkHeader};

    #[test]
    fn topic_mask_is_the_high_bit() {
        assert!(Cmd::from_raw(0x8000).is_topic());
        assert!(Cmd::from_raw(0x80ff).is_topic());
        assert!(!Cmd::from_raw(0x0001).is_topic());
        assert!(!Cmd::from_raw(0x7fff).is_topic());
    }

    #[test]
    fn raw_values_round_trip() {
        for cmd in [Cmd::from_raw(0x0001), Cmd::from_raw(0x8001), Cmd::from_raw(0xffff)] {
            assert_eq!(Cmd::from_raw(cmd.raw()), cmd);
            assert_eq!(Cmd::from_le_bytes(cmd.to_le_bytes()), cmd);
        }
    }

    #[test]
    fn debug_is_compact_raw_value() {
        assert_eq!(format!("{:?}", Cmd::from_raw(0x0001)), "Cmd(0x0001)");
        assert_eq!(format!("{:?}", Cmd::from_raw(0x80ff)), "Cmd(0x80ff)");
    }

    #[test]
    fn table_cmds_land_in_their_ranges() {
        assert!(Cmd::LayerChange.is_topic());
        assert!(Cmd::WpmUpdate.is_topic());
        assert!(!Cmd::GetVersion.is_topic());
        assert!(!Cmd::SetKeyAction.is_topic());
    }

    #[test]
    fn topic_event_round_trips_through_the_wire() {
        // The generated enum encodes to a topic frame the host decodes back to
        // the same variant — the producer and consumer halves share one table.
        let mut buf = [0u8; 64];
        let ev = TopicEvent::LayerChange(7);
        let framed_len = ev.encode(&mut buf).unwrap();

        // Decode the COBS frame back to the logical [cmd, seq, payload].
        let mut df = Deframer::new();
        df.commit(framed_len);
        let n = df.next(&mut buf).expect("one whole topic frame");
        let header = RynkHeader::parse(buf[..RYNK_HEADER_SIZE].try_into().unwrap());
        assert_eq!(header.cmd, Cmd::LayerChange);
        assert_eq!(header.seq, 0, "topics push with SEQ 0");
        let decoded = TopicEvent::decode(header.cmd, &buf[RYNK_HEADER_SIZE..n]);
        assert!(matches!(decoded, Some(TopicEvent::LayerChange(7))));
    }

    #[test]
    fn topic_event_decode_rejects_non_topic_and_garbage() {
        // A request-range cmd is not in the topic table.
        assert!(TopicEvent::decode(Cmd::GetVersion, &[]).is_none());
        // A known topic cmd whose payload can't decode (LayerChange needs a byte).
        assert!(TopicEvent::decode(Cmd::LayerChange, &[]).is_none());
    }

    #[test]
    fn response_wrapping_adds_one_byte() {
        // Postcard's Result tag is 1 byte. The wrapped size of any
        // non-trivial T must equal `1 + T::POSTCARD_MAX_SIZE`.
        let bare = <DeviceCapabilities as MaxSize>::POSTCARD_MAX_SIZE;
        let wrapped = <Result<DeviceCapabilities, RynkError> as MaxSize>::POSTCARD_MAX_SIZE;
        assert_eq!(wrapped, bare + 1);
    }
}
