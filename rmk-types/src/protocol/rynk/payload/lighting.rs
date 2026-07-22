//! Lighting protocol types.
//!
//! The wire model deliberately separates stable, board-visible identities
//! from dense compositor slots and electrical chain order. Hosts address
//! lights by [`LightingLedId`]; topology and routing readback explain key
//! association, geometry, zones, split-node ownership, and physical outputs.

use heapless::{String, Vec};
use postcard::experimental::max_size::MaxSize;
use serde::{Deserialize, Serialize};

/// Maximum postcard payload admitted by this first lighting ICD.
pub const LIGHTING_PAYLOAD_SIZE: usize = 256;
/// Number of metadata records in one topology page.
pub const LIGHTING_PAGE_SIZE: usize = 8;
/// Number of overlay cells in one replacement chunk.
pub const LIGHTING_OVERLAY_CHUNK_SIZE: usize = 8;
/// Number of scene cells in one scene page or replacement chunk.
pub const LIGHTING_SCENE_CHUNK_SIZE: usize = 8;
/// Number of immutable conditional cells in one readback page.
pub const LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE: usize = 8;
/// Maximum UTF-8 byte length of a zone name.
pub const LIGHTING_ZONE_NAME_SIZE: usize = 24;
/// Maximum UTF-8 byte length of one extension effect or palette name.
pub const LIGHTING_EXTENSION_NAME_SIZE: usize = 16;
/// Number of names in one extension-names page.
pub const LIGHTING_EXTENSION_NAME_CHUNK: usize = 8;

macro_rules! wire_type {
    ($item:item) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, MaxSize)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        #[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
        #[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
        $item
    };
}

wire_type! {
    /// Stable, board-wide identity of one independently controllable light.
    #[repr(transparent)]
    pub struct LightingLedId(pub u16);
}

wire_type! {
    /// Stable identity of one semantic lighting zone.
    #[repr(transparent)]
    pub struct LightingZoneId(pub u8);
}

wire_type! {
    /// Identity of a lighting processor, such as one half of a split keyboard.
    #[repr(transparent)]
    pub struct LightingNodeId(pub u8);
}

wire_type! {
    /// Identity of one physical output owned by a lighting node.
    #[repr(transparent)]
    pub struct LightingOutputId(pub u8);
}

wire_type! {
    /// One real key in RMK's logical matrix. Matrix holes have no record.
    pub struct LightingMatrixPosition {
        pub row: u8,
        pub col: u8,
    }
}

wire_type! {
    /// Board-global Q8.8 point in key-pitch units.
    pub struct LightingPoint3 {
        pub x: i16,
        pub y: i16,
        pub z: i16,
    }
}

wire_type! {
    /// Positive Q8.8 key dimensions in key-pitch units.
    pub struct LightingKeySize {
        pub width: u16,
        pub height: u16,
    }
}

wire_type! {
    /// Shared physical-key geometry consumed by lighting, displays, and hosts.
    pub struct LightingPhysicalKey {
        pub matrix: LightingMatrixPosition,
        pub center: LightingPoint3,
        pub size: LightingKeySize,
        /// Clockwise rotation in hundredths of one degree.
        pub rotation: i16,
    }
}

wire_type! {
    /// One semantic light. It may have key association, explicit geometry,
    /// both, or neither.
    pub struct LightingLed {
        pub id: LightingLedId,
        pub key: Option<LightingMatrixPosition>,
        pub position: Option<LightingPoint3>,
        /// Span into the flat zone-membership table.
        pub zone_start: u16,
        pub zone_len: u8,
    }
}

/// One named semantic zone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LightingZone {
    pub id: LightingZoneId,
    #[cfg_attr(feature = "wasm", tsify(type = "string"))]
    pub name: String<LIGHTING_ZONE_NAME_SIZE>,
}

impl MaxSize for LightingZone {
    const POSTCARD_MAX_SIZE: usize =
        LightingZoneId::POSTCARD_MAX_SIZE + crate::heapless_vec_max_size::<u8, LIGHTING_ZONE_NAME_SIZE>();
}

wire_type! {
    /// Color and addressability capabilities of a physical output.
    #[repr(transparent)]
    pub struct LightingOutputCapabilities(pub u8);
}

impl LightingOutputCapabilities {
    pub const BINARY: u8 = 1 << 0;
    pub const INTENSITY: u8 = 1 << 1;
    pub const RGB: u8 = 1 << 2;
    pub const WHITE: u8 = 1 << 3;
    pub const ADDRESSABLE: u8 = 1 << 4;

    pub const fn contains(self, bits: u8) -> bool {
        self.0 & bits == bits
    }
}

wire_type! {
    /// Whether all physical pixels of an output must have a logical route.
    pub enum LightingOutputCoverage {
        Complete,
        Sparse,
    }
}

wire_type! {
    /// One concrete output on one lighting node.
    pub struct LightingOutput {
        pub node: LightingNodeId,
        pub id: LightingOutputId,
        pub pixel_count: u16,
        pub capabilities: LightingOutputCapabilities,
        pub coverage: LightingOutputCoverage,
    }
}

wire_type! {
    /// Stable-light to physical-address mapping. Dense compositor slots are
    /// intentionally not part of the public protocol.
    pub struct LightingRoute {
        pub led_id: LightingLedId,
        pub node: LightingNodeId,
        pub output: LightingOutputId,
        pub physical_index: u16,
    }
}

wire_type! {
    /// Optional capabilities beyond the mandatory state/topology surface.
    #[repr(transparent)]
    pub struct LightingFeatureFlags(pub u16);
}

impl LightingFeatureFlags {
    pub const PHYSICAL_GEOMETRY: u16 = 1 << 0;
    pub const ZONES: u16 = 1 << 1;
    pub const ROUTING: u16 = 1 << 2;
    pub const OVERLAY_TTL: u16 = 1 << 3;
    pub const ATOMIC_OVERLAY_REPLACE: u16 = 1 << 4;
    pub const LAYER_AWARE: u16 = 1 << 5;
    /// Runtime-configurable per-layer scenes stored on the device.
    pub const LAYER_SCENES: u16 = 1 << 6;
    /// Revision-pinned readback of the transient overlay.
    pub const OVERLAY_READBACK: u16 = 1 << 7;
    /// Read-only board-compiled layer scenes, separate from runtime scenes.
    pub const COMPILED_LAYER_SCENES: u16 = 1 << 8;
    /// Read-only board-compiled rules driven by layer and battery state.
    pub const COMPILED_CONDITIONAL_SCENES: u16 = 1 << 9;
    /// Declarative three-state output policy and live readback.
    pub const OUTPUT_MODE: u16 = 1 << 10;
    /// Host-selectable animated extension effects served by an effect pack.
    pub const EXTENSION_EFFECTS: u16 = 1 << 11;

    pub const fn contains(self, bits: u16) -> bool {
        self.0 & bits == bits
    }
}

wire_type! {
    /// Built-in effects accepted by this firmware.
    #[repr(transparent)]
    pub struct LightingEffectFlags(pub u8);
}

impl LightingEffectFlags {
    pub const SOLID: u8 = 1 << 0;
    pub const BLINK: u8 = 1 << 1;
    pub const BREATHE: u8 = 1 << 2;

    pub const fn contains(self, bits: u8) -> bool {
        self.0 & bits == bits
    }
}

wire_type! {
    /// Static limits and topology identity for a lighting-enabled device.
    pub struct LightingCapabilities {
        pub topology_revision: u32,
        /// Real logical matrix keys, including keys without measured geometry.
        pub logical_key_count: u16,
        pub physical_key_count: u16,
        pub led_count: u16,
        pub zone_count: u16,
        pub zone_membership_count: u16,
        pub output_count: u16,
        pub route_count: u16,
        pub overlay_capacity: u16,
        pub page_capacity: u8,
        pub overlay_chunk_capacity: u8,
        pub features: LightingFeatureFlags,
        pub effects: LightingEffectFlags,
    }
}

wire_type! {
    /// Revision-pinned request for one metadata page.
    pub struct LightingPageRequest {
        pub topology_revision: u32,
        pub offset: u16,
    }
}

macro_rules! page_type {
    ($name:ident, $item:ty, $ts:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[cfg_attr(feature = "defmt", derive(defmt::Format))]
        #[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
        #[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
        pub struct $name {
            pub topology_revision: u32,
            pub total_count: u16,
            #[cfg_attr(feature = "wasm", tsify(type = $ts))]
            pub items: Vec<$item, LIGHTING_PAGE_SIZE>,
        }

        impl MaxSize for $name {
            const POSTCARD_MAX_SIZE: usize = u32::POSTCARD_MAX_SIZE
                + u16::POSTCARD_MAX_SIZE
                + crate::heapless_vec_max_size::<$item, LIGHTING_PAGE_SIZE>();
        }
    };
}

page_type!(LightingKeysPage, LightingMatrixPosition, "LightingMatrixPosition[]");
page_type!(LightingPhysicalKeysPage, LightingPhysicalKey, "LightingPhysicalKey[]");
page_type!(LightingLedsPage, LightingLed, "LightingLed[]");
page_type!(LightingZonesPage, LightingZone, "LightingZone[]");
page_type!(LightingZoneMembershipsPage, LightingZoneId, "LightingZoneId[]");
page_type!(LightingOutputsPage, LightingOutput, "LightingOutput[]");
page_type!(LightingRoutesPage, LightingRoute, "LightingRoute[]");

wire_type! {
    /// Device-independent linear RGB sample.
    pub struct LightingRgb8 {
        pub r: u8,
        pub g: u8,
        pub b: u8,
    }
}

wire_type! {
    /// Bounded set of standard effects understood by the RMK lighting engine.
    pub enum LightingEffect {
        Solid {
            color: LightingRgb8,
        },
        Blink {
            color: LightingRgb8,
            period_ms: u32,
            phase_ms: u32,
            duty: u8,
        },
        Breathe {
            color: LightingRgb8,
            period_ms: u32,
            phase_ms: u32,
            step_ms: u16,
        },
    }
}

impl LightingEffect {
    /// Validate parameters before adapting this wire value to the standard
    /// engine. Invalid effects never partially mutate live lighting state.
    pub const fn validate(&self) -> LightingResult<()> {
        match *self {
            Self::Solid { .. } => Ok(()),
            Self::Blink { period_ms, duty, .. } if period_ms != 0 && duty <= 100 => Ok(()),
            Self::Breathe { period_ms, step_ms, .. }
                if period_ms >= 2 && step_ms != 0 && (step_ms as u32) < period_ms =>
            {
                Ok(())
            }
            _ => Err(LightingError::InvalidEffect),
        }
    }
}

wire_type! {
    pub enum LightingBackgroundMode {
        Solid,
        Breathe,
    }
}

wire_type! {
    /// VIA-compatible designated background. It is only the lowest standard
    /// source; disabling it does not disable layers, overlays, or status.
    pub struct LightingBackgroundState {
        pub enabled: bool,
        pub hue: u8,
        pub saturation: u8,
        pub value: u8,
        pub speed: u8,
        pub mode: LightingBackgroundMode,
    }
}

wire_type! {
    pub struct LightingMutableState {
        pub output_enabled: bool,
        pub output_brightness: u8,
        pub background: LightingBackgroundState,
    }
}

wire_type! {
    /// Authoritative mutable state and optimistic-concurrency revision.
    pub struct LightingState {
        pub revision: u32,
        pub output_enabled: bool,
        pub output_brightness: u8,
        pub background: LightingBackgroundState,
        pub overlay_len: u16,
    }
}

wire_type! {
    pub struct SetLightingStateRequest {
        pub expected_revision: u32,
        pub state: LightingMutableState,
    }
}

wire_type! {
    /// One transient overlay cell addressed by stable LED identity.
pub struct LightingOverlayCell {
        pub led_id: LightingLedId,
        pub effect: LightingEffect,
        /// Relative lifetime. `None` lasts until unset, clear, or reboot;
        /// `Some(0)` is invalid.
        pub ttl_ms: Option<u32>,
    }
}

wire_type! {
    /// Revision-pinned request for one transient overlay page.
    pub struct LightingOverlayPageRequest {
        /// Expected [`LightingState::revision`].
        pub revision: u32,
        pub offset: u16,
    }
}

/// One atomically sampled page of transient overlay cells. Cell TTLs are
/// remaining relative lifetimes at the sample time, never firmware deadlines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LightingOverlayPage {
    pub revision: u32,
    pub total_count: u16,
    #[cfg_attr(feature = "wasm", tsify(type = "LightingOverlayCell[]"))]
    pub items: Vec<LightingOverlayCell, LIGHTING_OVERLAY_CHUNK_SIZE>,
}

impl MaxSize for LightingOverlayPage {
    const POSTCARD_MAX_SIZE: usize = u32::POSTCARD_MAX_SIZE
        + u16::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<LightingOverlayCell, LIGHTING_OVERLAY_CHUNK_SIZE>();
}

impl LightingOverlayCell {
    /// Validate the effect and relative lifetime. `None` is persistent and a
    /// positive TTL expires in firmware time; zero is never ambiguous.
    pub const fn validate(&self) -> LightingResult<()> {
        if matches!(self.ttl_ms, Some(0)) {
            return Err(LightingError::InvalidTtl);
        }
        self.effect.validate()
    }
}

wire_type! {
    pub struct SetLightingOverlayRequest {
        pub expected_revision: u32,
        pub cell: LightingOverlayCell,
    }
}

wire_type! {
    pub struct UnsetLightingOverlayRequest {
        pub expected_revision: u32,
        pub led_id: LightingLedId,
    }
}

wire_type! {
    pub struct ClearLightingOverlayRequest {
        pub expected_revision: u32,
    }
}

wire_type! {
    /// Begin an atomic, multi-packet overlay replacement.
    pub struct BeginLightingOverlayReplaceRequest {
        pub expected_revision: u32,
        pub cell_count: u16,
    }
}

wire_type! {
    /// Opaque transaction token allocated by the firmware.
    pub struct LightingOverlayTransaction {
        pub id: u32,
        pub cell_count: u16,
    }
}

/// One ordered transaction chunk. Chunks are applied only by commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct PutLightingOverlayChunkRequest {
    pub transaction_id: u32,
    pub offset: u16,
    #[cfg_attr(feature = "wasm", tsify(type = "LightingOverlayCell[]"))]
    pub cells: Vec<LightingOverlayCell, LIGHTING_OVERLAY_CHUNK_SIZE>,
}

impl MaxSize for PutLightingOverlayChunkRequest {
    const POSTCARD_MAX_SIZE: usize = u32::POSTCARD_MAX_SIZE
        + u16::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<LightingOverlayCell, LIGHTING_OVERLAY_CHUNK_SIZE>();
}

wire_type! {
    pub struct CommitLightingOverlayReplaceRequest {
        pub transaction_id: u32,
    }
}

wire_type! {
    pub struct AbortLightingOverlayReplaceRequest {
        pub transaction_id: u32,
    }
}

wire_type! {
    /// Wire mirror of the engine's layer composition policy.
    pub enum LightingLayerPolicy {
        /// Only the effective layer contributes scene cells.
        EffectiveOnly,
        /// Default first, then the active set in ascending precedence, with
        /// the effective layer last. Sparse cells fall through.
        ActiveStack,
    }
}

wire_type! {
    /// One durable scene cell: an effect bound to a stable LED on one layer.
    pub struct LightingSceneCell {
        pub layer: u8,
        pub led_id: LightingLedId,
        pub effect: LightingEffect,
    }
}

impl LightingSceneCell {
    /// Validate the effect. Layer and LED bounds are checked against the
    /// live keymap and topology by the firmware service.
    pub const fn validate(&self) -> LightingResult<()> {
        self.effect.validate()
    }
}

wire_type! {
    /// Scene limits and current occupancy. Kept out of
    /// [`LightingCapabilities`]/[`LightingState`] so their postcard layout is
    /// unchanged for existing hosts; discovery uses
    /// [`LightingFeatureFlags::LAYER_SCENES`] plus this endpoint.
pub struct LightingSceneStatus {
        /// Current [`LightingState::revision`]; scene mutations advance it.
        pub revision: u32,
        /// Maximum stored scene cells. `0` means scenes are absent.
        pub capacity: u16,
        pub scene_len: u16,
        pub policy: LightingLayerPolicy,
        /// Cells per `GetLightingScenes` page and per replacement chunk.
        pub chunk_capacity: u8,
    }
}

wire_type! {
    /// Occupancy of the immutable board-compiled layer-scene source.
    ///
    /// This source is distinct from [`LightingSceneStatus`]'s mutable table
    /// and is pinned to the topology revision for the firmware build.
    pub struct LightingCompiledSceneStatus {
        pub topology_revision: u32,
        pub scene_len: u16,
        /// Composition policy of the immutable compiled source. This is
        /// independent from the mutable runtime scene table's policy.
        pub policy: LightingLayerPolicy,
        pub chunk_capacity: u8,
    }
}

wire_type! {
    /// Revision-pinned request for one scene page. `revision` is the expected
    /// [`LightingState::revision`]; a stale read is rejected so multi-page
    /// reads stay self-consistent.
    pub struct LightingScenePageRequest {
        pub revision: u32,
        pub offset: u16,
    }
}

/// One page of stored scene cells, echoing the pinned state revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LightingScenesPage {
    pub revision: u32,
    pub total_count: u16,
    #[cfg_attr(feature = "wasm", tsify(type = "LightingSceneCell[]"))]
    pub items: Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>,
}

/// One page of immutable board-compiled layer scenes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LightingCompiledScenesPage {
    pub topology_revision: u32,
    pub total_count: u16,
    #[cfg_attr(feature = "wasm", tsify(type = "LightingSceneCell[]"))]
    pub items: Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>,
}

impl MaxSize for LightingCompiledScenesPage {
    const POSTCARD_MAX_SIZE: usize = u32::POSTCARD_MAX_SIZE
        + u16::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>();
}

wire_type! {
    pub struct LightingLayerCondition {
        pub layer: u8,
        pub active: bool,
    }
}

wire_type! {
    pub enum LightingChargeCondition {
        Any,
        Charging,
        Discharging,
        Unknown,
    }
}

wire_type! {
    pub struct LightingBatteryCondition {
        pub node: LightingNodeId,
        pub min_level: Option<u8>,
        pub max_level: Option<u8>,
        pub charge: LightingChargeCondition,
    }
}

wire_type! {
    /// A conjunction of optional layer and battery predicates.
    pub struct LightingConditionSet {
        pub layer: Option<LightingLayerCondition>,
        pub battery: Option<LightingBatteryCondition>,
    }
}

wire_type! {
    /// One immutable conditional effect compiled from keyboard configuration.
    pub struct LightingConditionalSceneCell {
        pub conditions: LightingConditionSet,
        pub led_id: LightingLedId,
        pub effect: LightingEffect,
    }
}

wire_type! {
    /// Key/layer controls that gate the configured lighting presentation.
    pub struct LightingControls {
        pub output_toggle_user_action: Option<u8>,
        pub wake_layer: Option<u8>,
    }
}

wire_type! {
    /// Persistent policy selected by the board's configured cycle action.
    pub enum LightingOutputMode {
        AlwaysOn,
        AlwaysOff,
        PoweredOnly,
    }
}

wire_type! {
    /// Power source used by split renderers in `PoweredOnly` mode.
    pub enum LightingPoweredOnlyScope {
        Authority,
        Local,
    }
}

wire_type! {
    /// Configured status LED and its mode-specific effects.
    pub struct LightingOutputModeIndicator {
        pub led_id: LightingLedId,
        pub always_on: LightingEffect,
        pub always_off: LightingEffect,
        pub powered_only: LightingEffect,
    }
}

wire_type! {
    /// Authoritative output policy plus the inputs that determine whether the
    /// LEDs are physically enabled right now.
    pub struct LightingOutputModeState {
        pub mode: LightingOutputMode,
        pub powered: bool,
        pub wake_active: bool,
        pub effective_enabled: bool,
        pub powered_only_scope: LightingPoweredOnlyScope,
        pub cycle_user_action: Option<u8>,
        pub wake_layer: Option<u8>,
        pub indicator: Option<LightingOutputModeIndicator>,
    }
}

wire_type! {
    /// Current selection of the firmware's animated extension band. Indices
    /// address the name lists served by `GetLightingExtensionNames`.
    pub struct LightingExtensionState {
        pub effect: u8,
        pub palette: u8,
        pub value: u8,
        pub speed: u8,
    }
}

wire_type! {
    /// Extension-effects discovery: name-list sizes plus the live selection,
    /// revision-pinned like every other lighting mutation surface.
    pub struct LightingExtension {
        pub revision: u32,
        pub effect_count: u8,
        pub palette_count: u8,
        pub state: LightingExtensionState,
    }
}

wire_type! {
    /// Which extension name list a page request addresses.
    pub enum LightingExtensionNameKind {
        Effects,
        Palettes,
    }
}

wire_type! {
    pub struct LightingExtensionNamesRequest {
        pub kind: LightingExtensionNameKind,
        pub offset: u8,
    }
}

/// One page of extension effect or palette names. Names are static for a
/// firmware build, so pages carry no revision pin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LightingExtensionNamesPage {
    pub total: u8,
    #[cfg_attr(feature = "wasm", tsify(type = "string[]"))]
    pub items: Vec<String<LIGHTING_EXTENSION_NAME_SIZE>, LIGHTING_EXTENSION_NAME_CHUNK>,
}

impl MaxSize for LightingExtensionNamesPage {
    const POSTCARD_MAX_SIZE: usize = u8::POSTCARD_MAX_SIZE
        + crate::varint_max_size(LIGHTING_EXTENSION_NAME_CHUNK)
        + LIGHTING_EXTENSION_NAME_CHUNK * crate::heapless_vec_max_size::<u8, LIGHTING_EXTENSION_NAME_SIZE>();
}

wire_type! {
    pub struct SetLightingExtensionStateRequest {
        pub expected_revision: u32,
        pub state: LightingExtensionState,
    }
}

wire_type! {
    pub struct LightingConditionalSceneStatus {
        pub topology_revision: u32,
        pub cell_len: u16,
        pub chunk_capacity: u8,
        pub controls: LightingControls,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct LightingConditionalScenesPage {
    pub topology_revision: u32,
    pub total_count: u16,
    #[cfg_attr(feature = "wasm", tsify(type = "LightingConditionalSceneCell[]"))]
    pub items: Vec<LightingConditionalSceneCell, LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE>,
}

impl MaxSize for LightingConditionalScenesPage {
    const POSTCARD_MAX_SIZE: usize = u32::POSTCARD_MAX_SIZE
        + u16::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<LightingConditionalSceneCell, LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE>();
}

impl MaxSize for LightingScenesPage {
    const POSTCARD_MAX_SIZE: usize = u32::POSTCARD_MAX_SIZE
        + u16::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>();
}

wire_type! {
    pub struct SetLightingSceneCellRequest {
        pub expected_revision: u32,
        pub cell: LightingSceneCell,
    }
}

wire_type! {
    pub struct UnsetLightingSceneCellRequest {
        pub expected_revision: u32,
        pub layer: u8,
        pub led_id: LightingLedId,
    }
}

wire_type! {
    pub struct SetLightingLayerPolicyRequest {
        pub expected_revision: u32,
        pub policy: LightingLayerPolicy,
    }
}

wire_type! {
    /// Begin an atomic, multi-packet scene-table replacement.
    pub struct BeginLightingSceneReplaceRequest {
        pub expected_revision: u32,
        pub cell_count: u16,
    }
}

wire_type! {
    /// Opaque scene transaction token allocated by the firmware.
    pub struct LightingSceneTransaction {
        pub id: u32,
        pub cell_count: u16,
    }
}

/// One ordered scene transaction chunk. Chunks are applied only by commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
#[cfg_attr(feature = "wasm", derive(tsify::Tsify))]
#[cfg_attr(feature = "wasm", tsify(into_wasm_abi, from_wasm_abi))]
pub struct PutLightingSceneChunkRequest {
    pub transaction_id: u32,
    pub offset: u16,
    #[cfg_attr(feature = "wasm", tsify(type = "LightingSceneCell[]"))]
    pub cells: Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>,
}

impl MaxSize for PutLightingSceneChunkRequest {
    const POSTCARD_MAX_SIZE: usize = u32::POSTCARD_MAX_SIZE
        + u16::POSTCARD_MAX_SIZE
        + crate::heapless_vec_max_size::<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>();
}

wire_type! {
    pub struct CommitLightingSceneReplaceRequest {
        pub transaction_id: u32,
    }
}

wire_type! {
    pub struct AbortLightingSceneReplaceRequest {
        pub transaction_id: u32,
    }
}

wire_type! {
    /// Lighting-domain rejection carried inside Rynk's outer protocol result.
    pub enum LightingError {
        Unsupported,
        InvalidRequest,
        InvalidEffect,
        InvalidTtl,
        TopologyRevisionConflict { expected: u32, current: u32 },
        StateRevisionConflict { expected: u32, current: u32 },
        UnknownLed { led_id: LightingLedId },
        OverlayFull { capacity: u16 },
        TransactionBusy,
        InvalidTransaction,
        TransactionExpired,
        TransactionIncomplete { expected: u16, received: u16 },
        // Appended after the first lighting ICD; new variants only surface
        // from the new scene endpoints, so older hosts never decode them.
        UnknownLayer { layer: u8 },
        SceneFull { capacity: u16 },
    }
}

/// Detailed lighting result nested inside Rynk's transport/protocol result.
pub type LightingResult<T> = Result<T, LightingError>;
pub type LightingCapabilitiesResult = LightingResult<LightingCapabilities>;
pub type LightingStateResult = LightingResult<LightingState>;
pub type LightingKeysPageResult = LightingResult<LightingKeysPage>;
pub type LightingPhysicalKeysPageResult = LightingResult<LightingPhysicalKeysPage>;
pub type LightingLedsPageResult = LightingResult<LightingLedsPage>;
pub type LightingZonesPageResult = LightingResult<LightingZonesPage>;
pub type LightingZoneMembershipsPageResult = LightingResult<LightingZoneMembershipsPage>;
pub type LightingOutputsPageResult = LightingResult<LightingOutputsPage>;
pub type LightingRoutesPageResult = LightingResult<LightingRoutesPage>;
pub type LightingOverlayPageResult = LightingResult<LightingOverlayPage>;
pub type LightingOverlayTransactionResult = LightingResult<LightingOverlayTransaction>;
pub type LightingSceneStatusResult = LightingResult<LightingSceneStatus>;
pub type LightingScenesPageResult = LightingResult<LightingScenesPage>;
pub type LightingCompiledSceneStatusResult = LightingResult<LightingCompiledSceneStatus>;
pub type LightingCompiledScenesPageResult = LightingResult<LightingCompiledScenesPage>;
pub type LightingConditionalSceneStatusResult = LightingResult<LightingConditionalSceneStatus>;
pub type LightingConditionalScenesPageResult = LightingResult<LightingConditionalScenesPage>;
pub type LightingOutputModeStateResult = LightingResult<LightingOutputModeState>;
pub type LightingExtensionResult = LightingResult<LightingExtension>;
pub type LightingExtensionNamesPageResult = LightingResult<LightingExtensionNamesPage>;
pub type LightingSceneTransactionResult = LightingResult<LightingSceneTransaction>;
pub type LightingUnitResult = LightingResult<()>;

wire_type! {
    /// Best-effort invalidation marker. Hosts recover current authoritative
    /// state with `GetLightingState`; events never carry a second state copy.
    pub struct LightingChanged;
}

const _: () = {
    use crate::protocol::rynk::RynkError;

    macro_rules! assert_endpoint_fits {
        ($req:ty, $resp:ty) => {
            core::assert!(<$req as MaxSize>::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);
            core::assert!(<Result<$resp, RynkError> as MaxSize>::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);
        };
    }

    assert_endpoint_fits!((), LightingCapabilitiesResult);
    assert_endpoint_fits!((), LightingStateResult);
    assert_endpoint_fits!(SetLightingStateRequest, LightingStateResult);
    assert_endpoint_fits!(LightingPageRequest, LightingKeysPageResult);
    assert_endpoint_fits!(LightingPageRequest, LightingPhysicalKeysPageResult);
    assert_endpoint_fits!(LightingPageRequest, LightingLedsPageResult);
    assert_endpoint_fits!(LightingPageRequest, LightingZonesPageResult);
    assert_endpoint_fits!(LightingPageRequest, LightingZoneMembershipsPageResult);
    assert_endpoint_fits!(LightingPageRequest, LightingOutputsPageResult);
    assert_endpoint_fits!(LightingPageRequest, LightingRoutesPageResult);
    assert_endpoint_fits!(LightingOverlayPageRequest, LightingOverlayPageResult);
    assert_endpoint_fits!(SetLightingOverlayRequest, LightingStateResult);
    assert_endpoint_fits!(UnsetLightingOverlayRequest, LightingStateResult);
    assert_endpoint_fits!(ClearLightingOverlayRequest, LightingStateResult);
    assert_endpoint_fits!(BeginLightingOverlayReplaceRequest, LightingOverlayTransactionResult);
    assert_endpoint_fits!(PutLightingOverlayChunkRequest, LightingUnitResult);
    assert_endpoint_fits!(CommitLightingOverlayReplaceRequest, LightingStateResult);
    assert_endpoint_fits!(AbortLightingOverlayReplaceRequest, LightingUnitResult);
    assert_endpoint_fits!((), LightingSceneStatusResult);
    assert_endpoint_fits!(LightingScenePageRequest, LightingScenesPageResult);
    assert_endpoint_fits!((), LightingCompiledSceneStatusResult);
    assert_endpoint_fits!(LightingPageRequest, LightingCompiledScenesPageResult);
    assert_endpoint_fits!((), LightingConditionalSceneStatusResult);
    assert_endpoint_fits!(LightingPageRequest, LightingConditionalScenesPageResult);
    assert_endpoint_fits!((), LightingOutputModeStateResult);
    assert_endpoint_fits!((), LightingExtensionResult);
    assert_endpoint_fits!(LightingExtensionNamesRequest, LightingExtensionNamesPageResult);
    assert_endpoint_fits!(SetLightingExtensionStateRequest, LightingStateResult);
    assert_endpoint_fits!(SetLightingSceneCellRequest, LightingStateResult);
    assert_endpoint_fits!(UnsetLightingSceneCellRequest, LightingStateResult);
    assert_endpoint_fits!(SetLightingLayerPolicyRequest, LightingStateResult);
    assert_endpoint_fits!(BeginLightingSceneReplaceRequest, LightingSceneTransactionResult);
    assert_endpoint_fits!(PutLightingSceneChunkRequest, LightingUnitResult);
    assert_endpoint_fits!(CommitLightingSceneReplaceRequest, LightingStateResult);
    assert_endpoint_fits!(AbortLightingSceneReplaceRequest, LightingUnitResult);
    core::assert!(LightingChanged::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::rynk::tests::{assert_max_size_bound, round_trip};

    fn cell(id: u16) -> LightingOverlayCell {
        LightingOverlayCell {
            led_id: LightingLedId(id),
            effect: LightingEffect::Blink {
                color: LightingRgb8 { r: 1, g: 2, b: 3 },
                period_ms: u32::MAX,
                phase_ms: u32::MAX,
                duty: 100,
            },
            ttl_ms: Some(u32::MAX),
        }
    }

    #[test]
    fn geometry_and_key_association_round_trip() {
        round_trip(&LightingLed {
            id: LightingLedId(42),
            key: Some(LightingMatrixPosition { row: 3, col: 7 }),
            position: Some(LightingPoint3 { x: -128, y: 256, z: 64 }),
            zone_start: 2,
            zone_len: 3,
        });
        round_trip(&LightingLed {
            id: LightingLedId(1000),
            key: None,
            position: None,
            zone_start: 0,
            zone_len: 0,
        });
    }

    #[test]
    fn maximum_overlay_chunk_and_page_respect_bound() {
        let mut cells = Vec::new();
        for id in 0..LIGHTING_OVERLAY_CHUNK_SIZE as u16 {
            cells.push(cell(id)).unwrap();
        }
        let request = PutLightingOverlayChunkRequest {
            transaction_id: u32::MAX,
            offset: u16::MAX,
            cells: cells.clone(),
        };
        round_trip(&request);
        assert_max_size_bound(&request);
        assert!(PutLightingOverlayChunkRequest::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);

        let page = LightingOverlayPage {
            revision: u32::MAX,
            total_count: u16::MAX,
            items: cells,
        };
        round_trip(&page);
        assert_max_size_bound(&page);
        assert!(LightingOverlayPage::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);
    }

    #[test]
    fn maximum_zone_page_respects_bound() {
        let mut items = Vec::new();
        for id in 0..LIGHTING_PAGE_SIZE as u8 {
            let mut name = String::new();
            for _ in 0..LIGHTING_ZONE_NAME_SIZE {
                name.push('x').unwrap();
            }
            items
                .push(LightingZone {
                    id: LightingZoneId(id),
                    name,
                })
                .unwrap();
        }
        let page = LightingZonesPage {
            topology_revision: u32::MAX,
            total_count: u16::MAX,
            items,
        };
        round_trip(&page);
        assert_max_size_bound(&page);
    }

    fn scene_cell(layer: u8, id: u16) -> LightingSceneCell {
        LightingSceneCell {
            layer,
            led_id: LightingLedId(id),
            effect: LightingEffect::Breathe {
                color: LightingRgb8 { r: 4, g: 5, b: 6 },
                period_ms: u32::MAX,
                phase_ms: u32::MAX,
                step_ms: u16::MAX - 1,
            },
        }
    }

    #[test]
    fn scene_types_round_trip() {
        round_trip(&LightingLayerPolicy::EffectiveOnly);
        round_trip(&LightingLayerPolicy::ActiveStack);
        round_trip(&scene_cell(3, 42));
        round_trip(&LightingSceneStatus {
            revision: u32::MAX,
            capacity: 256,
            scene_len: 12,
            policy: LightingLayerPolicy::ActiveStack,
            chunk_capacity: LIGHTING_SCENE_CHUNK_SIZE as u8,
        });
        round_trip(&LightingCompiledSceneStatus {
            topology_revision: u32::MAX,
            scene_len: 12,
            policy: LightingLayerPolicy::EffectiveOnly,
            chunk_capacity: LIGHTING_SCENE_CHUNK_SIZE as u8,
        });
        round_trip(&LightingSceneTransaction {
            id: u32::MAX,
            cell_count: u16::MAX,
        });
        round_trip(&LightingError::UnknownLayer { layer: 9 });
        round_trip(&LightingError::SceneFull { capacity: 256 });
    }

    #[test]
    fn maximum_scene_chunk_and_page_respect_bounds() {
        let mut cells = Vec::new();
        for id in 0..LIGHTING_SCENE_CHUNK_SIZE as u16 {
            cells.push(scene_cell(u8::MAX, id)).unwrap();
        }
        let request = PutLightingSceneChunkRequest {
            transaction_id: u32::MAX,
            offset: u16::MAX,
            cells: cells.clone(),
        };
        round_trip(&request);
        assert_max_size_bound(&request);
        assert!(PutLightingSceneChunkRequest::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);

        let page = LightingScenesPage {
            revision: u32::MAX,
            total_count: u16::MAX,
            items: cells.clone(),
        };
        round_trip(&page);
        assert_max_size_bound(&page);
        assert!(LightingScenesPage::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);

        let compiled_page = LightingCompiledScenesPage {
            topology_revision: u32::MAX,
            total_count: u16::MAX,
            items: cells,
        };
        round_trip(&compiled_page);
        assert_max_size_bound(&compiled_page);
        assert!(LightingCompiledScenesPage::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);
    }

    #[test]
    fn conditional_scene_page_round_trips_at_capacity() {
        let mut items = Vec::new();
        for id in 0..LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE as u16 {
            items
                .push(LightingConditionalSceneCell {
                    conditions: LightingConditionSet {
                        layer: Some(LightingLayerCondition {
                            layer: u8::MAX,
                            active: true,
                        }),
                        battery: Some(LightingBatteryCondition {
                            node: LightingNodeId(u8::MAX),
                            min_level: Some(1),
                            max_level: Some(100),
                            charge: LightingChargeCondition::Charging,
                        }),
                    },
                    led_id: LightingLedId(id),
                    effect: LightingEffect::Solid {
                        color: LightingRgb8 {
                            r: u8::MAX,
                            g: u8::MAX,
                            b: u8::MAX,
                        },
                    },
                })
                .unwrap();
        }
        let page = LightingConditionalScenesPage {
            topology_revision: u32::MAX,
            total_count: u16::MAX,
            items,
        };
        round_trip(&page);
        assert_max_size_bound(&page);
        assert!(LightingConditionalScenesPage::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);
    }

    #[test]
    fn extension_types_round_trip() {
        round_trip(&LightingExtensionState {
            effect: 1,
            palette: 2,
            value: 3,
            speed: 4,
        });
        round_trip(&LightingExtension {
            revision: u32::MAX,
            effect_count: 6,
            palette_count: 16,
            state: LightingExtensionState {
                effect: 5,
                palette: 15,
                value: u8::MAX,
                speed: 0,
            },
        });
        round_trip(&LightingExtensionNamesRequest {
            kind: LightingExtensionNameKind::Effects,
            offset: 0,
        });
        round_trip(&LightingExtensionNamesRequest {
            kind: LightingExtensionNameKind::Palettes,
            offset: LIGHTING_EXTENSION_NAME_CHUNK as u8,
        });
        round_trip(&SetLightingExtensionStateRequest {
            expected_revision: u32::MAX,
            state: LightingExtensionState {
                effect: 0,
                palette: 1,
                value: 2,
                speed: 3,
            },
        });
    }

    #[test]
    fn maximum_extension_names_page_respects_bound() {
        let mut items = Vec::new();
        for _ in 0..LIGHTING_EXTENSION_NAME_CHUNK {
            let mut name = String::new();
            for _ in 0..LIGHTING_EXTENSION_NAME_SIZE {
                name.push('x').unwrap();
            }
            items.push(name).unwrap();
        }
        let page = LightingExtensionNamesPage { total: u8::MAX, items };
        round_trip(&page);
        assert_max_size_bound(&page);
        assert!(LightingExtensionNamesPage::POSTCARD_MAX_SIZE <= LIGHTING_PAYLOAD_SIZE);
    }

    #[test]
    fn scene_cell_validation_is_effect_validation() {
        assert_eq!(scene_cell(0, 1).validate(), Ok(()));
        let invalid = LightingSceneCell {
            layer: 0,
            led_id: LightingLedId(1),
            effect: LightingEffect::Blink {
                color: LightingRgb8 { r: 1, g: 2, b: 3 },
                period_ms: 0,
                phase_ms: 0,
                duty: 50,
            },
        };
        assert_eq!(invalid.validate(), Err(LightingError::InvalidEffect));
    }

    #[test]
    fn effect_and_ttl_validation_is_explicit() {
        let mut valid = cell(1);
        assert_eq!(valid.validate(), Ok(()));
        valid.ttl_ms = Some(0);
        assert_eq!(valid.validate(), Err(LightingError::InvalidTtl));
        valid.ttl_ms = None;
        valid.effect = LightingEffect::Breathe {
            color: LightingRgb8 { r: 1, g: 2, b: 3 },
            period_ms: 100,
            phase_ms: 0,
            step_ms: 100,
        };
        assert_eq!(valid.validate(), Err(LightingError::InvalidEffect));
    }
}
