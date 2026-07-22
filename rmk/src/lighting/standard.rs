//! Ready-to-use compositor engine for ordinary RMK lighting.
//!
//! Boards provide static layer scenes plus optional extension and status
//! sources. The engine supplies a controllable uniform background, TTL host
//! overlay, deterministic priority bands, output brightness, frame history,
//! `LightAction` handling, and protocol-independent commands/readback.

use core::cell::RefCell;
use core::fmt;
use core::num::NonZeroU32;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use rmk_types::action::LightAction;

use super::Rgb8;
use super::compositor::{
    Compositor, Contribution, ExtensionDescriptor, ExtensionState, LightingSource, LogicalFrame, RenderError,
    RenderInput as SourceRenderInput,
};
use super::context::{LightingContext, LightingContextProvider};
use super::effect::{BuiltinEffect, LightingEffect};
use super::output::BrightnessTransform;
use super::service::{CommandResult, Invalidation, LightingEngine, RenderInput, RenderOutcome};
use super::source::{
    LayerPolicy, LayerScenes, LightingControls, OutputMode, OverlayError, OverlayUpdate, SceneCell, SparseScene,
    TtlOverlay,
};
use super::topology::LedSlot;
use crate::RawMutex;

/// Cells per overlay readback page. Kept equal to the wire chunk size so
/// protocol adapters can forward pages without re-batching.
pub const OVERLAY_CHUNK_SIZE: usize = 8;

/// Cells per scene page/replacement chunk. Kept equal to the wire chunk size
/// so protocol adapters can forward chunks without re-batching.
pub const SCENE_CHUNK_SIZE: usize = 8;

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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum BackgroundMode {
    Solid,
    Breathe,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BackgroundState {
    pub enabled: bool,
    pub hue: u8,
    pub saturation: u8,
    pub value: u8,
    pub speed: u8,
    pub mode: BackgroundMode,
}

impl Default for BackgroundState {
    fn default() -> Self {
        Self {
            enabled: true,
            hue: 0,
            saturation: 0,
            value: 32,
            speed: 128,
            mode: BackgroundMode::Solid,
        }
    }
}

/// Atomic partial update of the designated background.
///
/// Protocol adapters use this instead of a `ReadState` followed by
/// `SetBackground`: the lighting engine remains the sole mutable owner and
/// concurrent callers cannot overwrite fields changed between two mailbox
/// requests.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct BackgroundPatch {
    pub enabled: Option<bool>,
    pub hue: Option<u8>,
    pub saturation: Option<u8>,
    pub value: Option<u8>,
    pub speed: Option<u8>,
    pub mode: Option<BackgroundMode>,
}

impl BackgroundPatch {
    pub const fn apply_to(self, state: &mut BackgroundState) {
        if let Some(enabled) = self.enabled {
            state.enabled = enabled;
        }
        if let Some(hue) = self.hue {
            state.hue = hue;
        }
        if let Some(saturation) = self.saturation {
            state.saturation = saturation;
        }
        if let Some(value) = self.value {
            state.value = value;
        }
        if let Some(speed) = self.speed {
            state.speed = speed;
        }
        if let Some(mode) = self.mode {
            state.mode = mode;
        }
    }
}

/// Built-in designated background controlled by RGB/Vial-compatible fields.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct UniformBackground<const N: usize> {
    state: BackgroundState,
}

impl<const N: usize> UniformBackground<N> {
    pub const fn new(state: BackgroundState) -> Self {
        Self { state }
    }

    pub const fn state(&self) -> BackgroundState {
        self.state
    }

    pub fn set_state(&mut self, state: BackgroundState) {
        self.state = state;
    }

    fn effect(&self) -> BuiltinEffect {
        let color = if self.state.enabled {
            hsv(self.state.hue, self.state.saturation, self.state.value)
        } else {
            Rgb8::BLACK
        };
        match self.state.mode {
            BackgroundMode::Solid => BuiltinEffect::Solid { color },
            BackgroundMode::Breathe => BuiltinEffect::Breathe {
                color,
                period_ms: 250 + ((u8::MAX - self.state.speed) as u32 * 3_750 / 255),
                phase_ms: 0,
                step_ms: 16,
            },
        }
    }
}

impl<Context, const N: usize> LightingSource<Rgb8, Context> for UniformBackground<N> {
    fn len(&self, _: &SourceRenderInput<'_, Context>) -> usize {
        N
    }

    fn slot(&self, index: usize, _: &SourceRenderInput<'_, Context>) -> LedSlot {
        LedSlot::from_index(index)
    }

    fn contribution(&mut self, _: usize, input: &SourceRenderInput<'_, Context>) -> Contribution<Rgb8> {
        Contribution::Opaque(self.effect().sample(input.now_ms))
    }
}

/// Zero-sized source used when a board does not need an extension band.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct EmptySource;

impl<C, Context> LightingSource<C, Context> for EmptySource {
    fn len(&self, _: &SourceRenderInput<'_, Context>) -> usize {
        0
    }

    fn slot(&self, _: usize, _: &SourceRenderInput<'_, Context>) -> LedSlot {
        unreachable!("EmptySource has no targets")
    }

    fn contribution(&mut self, _: usize, _: &SourceRenderInput<'_, Context>) -> Contribution<C> {
        unreachable!("EmptySource has no samples")
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OverlayCell {
    pub slot: LedSlot,
    pub effect: BuiltinEffect,
    /// Relative lifetime from command application. `None` persists until an
    /// explicit unset/clear or reboot.
    pub ttl_ms: Option<NonZeroU32>,
}

const EMPTY_OVERLAY_CELL: OverlayCell = OverlayCell {
    slot: LedSlot(0),
    effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
    ttl_ms: None,
};

/// Fixed-capacity, owned batch suitable for a bounded async mailbox.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OverlayBatch<const CAP: usize> {
    cells: [OverlayCell; CAP],
    len: usize,
}

impl<const CAP: usize> OverlayBatch<CAP> {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_OVERLAY_CELL; CAP],
            len: 0,
        }
    }

    pub fn push(&mut self, cell: OverlayCell) -> Result<(), OverlayError> {
        if self.len == CAP {
            return Err(OverlayError::TooManyEntries {
                supplied: self.len + 1,
                capacity: CAP,
            });
        }
        self.cells[self.len] = cell;
        self.len += 1;
        Ok(())
    }

    pub fn as_slice(&self) -> &[OverlayCell] {
        &self.cells[..self.len]
    }
}

impl<const CAP: usize> Default for OverlayBatch<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

/// One durable scene cell: an effect bound to a local slot on one layer.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SceneTableCell {
    pub layer: u8,
    pub slot: LedSlot,
    pub effect: BuiltinEffect,
}

const EMPTY_SCENE_CELL: SceneTableCell = SceneTableCell {
    layer: 0,
    slot: LedSlot(0),
    effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
};

/// Fixed-capacity, owned scene chunk suitable for a bounded async mailbox.
/// One chunk is both a page of readback and a replacement-staging step.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SceneChunk {
    cells: [SceneTableCell; SCENE_CHUNK_SIZE],
    len: usize,
}

impl Default for SceneChunk {
    fn default() -> Self {
        Self::new()
    }
}

impl SceneChunk {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_SCENE_CELL; SCENE_CHUNK_SIZE],
            len: 0,
        }
    }

    pub fn push(&mut self, cell: SceneTableCell) -> Result<(), StandardError> {
        if self.len == SCENE_CHUNK_SIZE {
            return Err(StandardError::InvalidSceneRequest);
        }
        self.cells[self.len] = cell;
        self.len += 1;
        Ok(())
    }

    pub fn as_slice(&self) -> &[SceneTableCell] {
        &self.cells[..self.len]
    }
}

/// Fixed-capacity runtime scene table with layer-aware composition.
///
/// Cells are unique per `(layer, slot)` and stored in insertion order; order
/// is irrelevant to rendering because layer precedence comes from the policy
/// and same-layer cells can never target the same slot.
#[derive(Copy, Clone, Debug)]
pub struct SceneTable<const CAP: usize> {
    cells: [SceneTableCell; CAP],
    len: usize,
    policy: LayerPolicy,
}

impl<const CAP: usize> SceneTable<CAP> {
    pub const fn new() -> Self {
        Self {
            cells: [EMPTY_SCENE_CELL; CAP],
            len: 0,
            policy: LayerPolicy::ActiveStack,
        }
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn policy(&self) -> LayerPolicy {
        self.policy
    }

    pub fn set_policy(&mut self, policy: LayerPolicy) -> bool {
        let changed = self.policy != policy;
        self.policy = policy;
        changed
    }

    pub fn as_slice(&self) -> &[SceneTableCell] {
        &self.cells[..self.len]
    }

    /// Insert or update the cell addressed by `(layer, slot)`.
    pub fn set(&mut self, cell: SceneTableCell) -> Result<(), StandardError> {
        if let Some(existing) = self.cells[..self.len]
            .iter_mut()
            .find(|existing| existing.layer == cell.layer && existing.slot == cell.slot)
        {
            *existing = cell;
            return Ok(());
        }
        if self.len == CAP {
            return Err(StandardError::SceneFull { capacity: CAP });
        }
        self.cells[self.len] = cell;
        self.len += 1;
        Ok(())
    }

    /// Remove the cell addressed by `(layer, slot)`.
    pub fn unset(&mut self, layer: u8, slot: LedSlot) -> bool {
        if let Some(index) = self.cells[..self.len]
            .iter()
            .position(|cell| cell.layer == layer && cell.slot == slot)
        {
            self.len -= 1;
            self.cells[index] = self.cells[self.len];
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    /// One readback page starting at `offset`, clamped at the table's end.
    pub fn page(&self, offset: u16) -> SceneChunk {
        let start = (offset as usize).min(self.len);
        let end = (start + SCENE_CHUNK_SIZE).min(self.len);
        let mut chunk = SceneChunk::new();
        for cell in &self.cells[start..end] {
            chunk.push(*cell).expect("page is chunk-bounded");
        }
        chunk
    }

    fn cell_for_layer(&self, layer: u8, wanted: &mut usize) -> Option<&SceneTableCell> {
        for cell in self.cells[..self.len].iter().filter(|cell| cell.layer == layer) {
            if *wanted == 0 {
                return Some(cell);
            }
            *wanted -= 1;
        }
        None
    }

    fn cell_at(&self, context: &LightingContext, mut wanted: usize) -> &SceneTableCell {
        let effective = context.layers.effective;
        match self.policy {
            LayerPolicy::EffectiveOnly => self.cell_for_layer(effective, &mut wanted),
            LayerPolicy::ActiveStack => {
                let default = context.layers.default;
                if let Some(cell) = self.cell_for_layer(default, &mut wanted) {
                    return cell;
                }
                for layer in 0..super::context::LayerState::CAPACITY {
                    if layer != default
                        && layer != effective
                        && context.layers.is_active(layer)
                        && let Some(cell) = self.cell_for_layer(layer, &mut wanted)
                    {
                        return cell;
                    }
                }
                if effective != default {
                    self.cell_for_layer(effective, &mut wanted)
                } else {
                    None
                }
            }
        }
        .expect("LightingSource index must be below len")
    }

    fn included_len(&self, context: &LightingContext) -> usize {
        let effective = context.layers.effective;
        self.cells[..self.len]
            .iter()
            .filter(|cell| match self.policy {
                LayerPolicy::EffectiveOnly => cell.layer == effective,
                LayerPolicy::ActiveStack => {
                    cell.layer == context.layers.default
                        || cell.layer == effective
                        || context.layers.is_active(cell.layer)
                }
            })
            .count()
    }
}

impl<const CAP: usize> Default for SceneTable<CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const CAP: usize> PartialEq for SceneTable<CAP> {
    fn eq(&self, other: &Self) -> bool {
        // Cells past `len` are stale storage, not state.
        self.policy == other.policy && self.as_slice() == other.as_slice()
    }
}

impl<const CAP: usize> Eq for SceneTable<CAP> {}

impl<Context, const CAP: usize> LightingSource<Rgb8, Context> for SceneTable<CAP>
where
    Context: LightingContextProvider,
{
    fn len(&self, input: &SourceRenderInput<'_, Context>) -> usize {
        self.included_len(input.context.lighting_context())
    }

    fn slot(&self, index: usize, input: &SourceRenderInput<'_, Context>) -> LedSlot {
        self.cell_at(input.context.lighting_context(), index).slot
    }

    fn contribution(&mut self, index: usize, input: &SourceRenderInput<'_, Context>) -> Contribution<Rgb8> {
        Contribution::Opaque(
            self.cell_at(input.context.lighting_context(), index)
                .effect
                .sample(input.now_ms),
        )
    }
}

/// One in-progress, chunk-staged atomic scene replacement.
///
/// The overlay replacement stages host-side because a whole overlay batch
/// fits one mailbox command. A scene table is an order of magnitude larger,
/// so its transaction stages inside the engine via bounded chunks instead of
/// forcing kilobyte-sized command payloads through every mailbox.
struct SceneReplace<const CAP: usize> {
    id: u32,
    expected_revision: u32,
    expected_count: u16,
    cells: [SceneTableCell; CAP],
    len: usize,
    last_activity_ms: u64,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StandardCommand<const OVERLAY_CAP: usize, const SCENE_CAP: usize = 0> {
    SetOutputEnabled(bool),
    SetOutputBrightness(u8),
    SetBackground(BackgroundState),
    PatchBackground(BackgroundPatch),
    SetStateIfRevision {
        expected_revision: u32,
        state: StandardMutableState,
    },
    SetOverlay(OverlayCell),
    SetOverlayIfRevision {
        expected_revision: u32,
        cell: OverlayCell,
    },
    UnsetOverlay(LedSlot),
    UnsetOverlayIfRevision {
        expected_revision: u32,
        slot: LedSlot,
    },
    ClearOverlay,
    ClearOverlayIfRevision {
        expected_revision: u32,
    },
    ReplaceOverlay(OverlayBatch<OVERLAY_CAP>),
    ReplaceOverlayIfRevision {
        expected_revision: u32,
        batch: OverlayBatch<OVERLAY_CAP>,
    },
    /// Atomically export all state needed by a renderer replica. The larger
    /// snapshot moves through the referenced slot so it does not inflate
    /// every element of the bounded command queue.
    ExportReplica(&'static StandardReplicaSlot<OVERLAY_CAP, SCENE_CAP>),
    /// Atomically install a snapshot previously placed in the referenced
    /// slot. Intended for a renderer replica, not a second authority.
    ApplyReplica(&'static StandardReplicaSlot<OVERLAY_CAP, SCENE_CAP>),
    ReadState,
    /// Descriptor and current selection of the extension source, if any.
    ReadExtension,
    SetExtensionIfRevision {
        expected_revision: u32,
        state: ExtensionState,
    },
    /// One atomically sampled page of the transient overlay.
    ReadOverlay {
        offset: u16,
    },
    SetSceneCellIfRevision {
        expected_revision: u32,
        cell: SceneTableCell,
    },
    UnsetSceneCellIfRevision {
        expected_revision: u32,
        layer: u8,
        slot: LedSlot,
    },
    SetLayerPolicyIfRevision {
        expected_revision: u32,
        policy: LayerPolicy,
    },
    /// Reserve the engine's single scene-staging transaction. The expected
    /// revision is recorded here and enforced atomically at commit.
    BeginSceneReplace {
        expected_revision: u32,
        cell_count: u16,
    },
    PutSceneChunk {
        transaction_id: u32,
        offset: u16,
        cells: SceneChunk,
    },
    CommitSceneReplace {
        transaction_id: u32,
    },
    AbortSceneReplace {
        transaction_id: u32,
    },
    /// One page of the stored scene table starting at `offset`.
    ReadScenes {
        offset: u16,
    },
    /// One page of the immutable board-compiled layer scenes.
    ReadCompiledScenes {
        offset: u16,
    },
}

/// Mutable standard state excluding the transient overlay contents and the
/// engine-owned optimistic-concurrency revision.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StandardMutableState {
    pub output_enabled: bool,
    pub output_brightness: u8,
    pub background: BackgroundState,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StandardState {
    pub revision: u32,
    pub output_enabled: bool,
    pub output_mode: OutputMode,
    pub powered: bool,
    pub wake_active: bool,
    pub output_brightness: u8,
    pub background: BackgroundState,
    pub overlay_len: usize,
    pub scene_len: usize,
    pub scene_policy: LayerPolicy,
}

/// One page of transient overlay cells with remaining TTLs.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OverlayPage {
    pub revision: u32,
    pub total: u16,
    pub cells: OverlayBatch<OVERLAY_CHUNK_SIZE>,
}

/// One page of stored scene cells with the revision it was read under.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ScenePage {
    pub revision: u32,
    pub total: u16,
    pub cells: SceneChunk,
}

/// Extension-source readback: selectable content plus current selection.
/// `descriptor`/`state` are `None` when the engine's extension band is not
/// user-selectable (e.g. `EmptySource`).
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExtensionPage {
    pub revision: u32,
    pub descriptor: Option<ExtensionDescriptor>,
    pub state: Option<ExtensionState>,
}

/// One page of immutable board-compiled layer scenes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CompiledScenePage {
    pub total: u16,
    pub policy: LayerPolicy,
    pub cells: SceneChunk,
}

/// Protocol-independent readback for [`StandardCommand`]s. Most commands
/// answer with authoritative [`StandardState`]; scene reads and transaction
/// reservation carry their own shapes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StandardReply {
    State(StandardState),
    OverlayPage(OverlayPage),
    ScenesPage(ScenePage),
    CompiledScenesPage(CompiledScenePage),
    SceneTransaction { id: u32, cell_count: u16 },
    Extension(ExtensionPage),
}

impl StandardReply {
    /// The state readback carried by state-shaped replies.
    pub const fn state(self) -> Option<StandardState> {
        match self {
            Self::State(state) => Some(state),
            _ => None,
        }
    }
}

/// Complete declarative state needed by a standard-engine renderer replica.
///
/// `sample_time_ms` is the authority's animation clock at snapshot time.
/// Applying the snapshot anchors the replica's local monotonic clock to that
/// value; subsequent animation frames are sampled locally without link
/// traffic. Overlay TTLs are remaining lifetimes at the same instant.
///
/// The runtime scene table travels with the snapshot: replicas must render
/// host-configured per-layer scenes exactly like the authority, and they
/// never receive the incremental scene mutation commands themselves.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StandardReplicaState<const OVERLAY_CAP: usize, const SCENE_CAP: usize = 0> {
    pub revision: u32,
    pub mutable: StandardMutableState,
    pub output_mode: OutputMode,
    pub overlay: OverlayBatch<OVERLAY_CAP>,
    pub scenes: SceneTable<SCENE_CAP>,
    pub context: LightingContext,
    pub sample_time_ms: u64,
    /// Extension-source selection, carried so split renderer replicas track
    /// the authority's animated band. `None` when the authority has no
    /// selectable extension.
    pub extension: Option<ExtensionState>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ReplicaSlotError {
    Busy,
    Empty,
}

/// One statically allocated hand-off slot for a replica snapshot.
///
/// The lighting mailbox serializes access. Keeping the large snapshot out of
/// [`StandardCommand`] prevents command-channel capacity from multiplying its
/// RAM cost on small MCUs.
pub struct StandardReplicaSlot<const OVERLAY_CAP: usize, const SCENE_CAP: usize = 0> {
    value: BlockingMutex<RawMutex, RefCell<Option<StandardReplicaState<OVERLAY_CAP, SCENE_CAP>>>>,
}

impl<const OVERLAY_CAP: usize, const SCENE_CAP: usize> StandardReplicaSlot<OVERLAY_CAP, SCENE_CAP> {
    pub const fn new() -> Self {
        Self {
            value: BlockingMutex::new(RefCell::new(None)),
        }
    }

    pub fn put(&self, state: StandardReplicaState<OVERLAY_CAP, SCENE_CAP>) -> Result<(), ReplicaSlotError> {
        self.value.lock(|value| {
            let mut value = value.borrow_mut();
            if value.is_some() {
                Err(ReplicaSlotError::Busy)
            } else {
                *value = Some(state);
                Ok(())
            }
        })
    }

    pub fn take(&self) -> Result<StandardReplicaState<OVERLAY_CAP, SCENE_CAP>, ReplicaSlotError> {
        self.value
            .lock(|value| value.borrow_mut().take().ok_or(ReplicaSlotError::Empty))
    }
}

impl<const OVERLAY_CAP: usize, const SCENE_CAP: usize> Default for StandardReplicaSlot<OVERLAY_CAP, SCENE_CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const OVERLAY_CAP: usize, const SCENE_CAP: usize> fmt::Debug for StandardReplicaSlot<OVERLAY_CAP, SCENE_CAP> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StandardReplicaSlot").finish_non_exhaustive()
    }
}

impl<const OVERLAY_CAP: usize, const SCENE_CAP: usize> PartialEq for StandardReplicaSlot<OVERLAY_CAP, SCENE_CAP> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self, other)
    }
}

impl<const OVERLAY_CAP: usize, const SCENE_CAP: usize> Eq for StandardReplicaSlot<OVERLAY_CAP, SCENE_CAP> {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StandardError {
    Render(RenderError),
    Overlay(OverlayError),
    DeadlineOverflow,
    ReplicaSlot(ReplicaSlotError),
    RevisionConflict {
        expected: u32,
        current: u32,
    },
    SceneFull {
        capacity: usize,
    },
    SceneSlotOutOfRange {
        slot: LedSlot,
    },
    /// Malformed scene request: bad chunk order, count overflow, or a
    /// duplicate `(layer, slot)` within one staged replacement.
    InvalidSceneRequest,
    /// The extension source declined the state (none installed, or an index
    /// out of its descriptor's range).
    ExtensionUnsupported,
    SceneTransactionBusy,
    InvalidSceneTransaction,
    SceneTransactionExpired,
    SceneTransactionIncomplete {
        expected: u16,
        received: u16,
    },
}

impl From<RenderError> for StandardError {
    fn from(value: RenderError) -> Self {
        Self::Render(value)
    }
}

impl From<OverlayError> for StandardError {
    fn from(value: OverlayError) -> Self {
        Self::Overlay(value)
    }
}

impl From<ReplicaSlotError> for StandardError {
    fn from(value: ReplicaSlotError) -> Self {
        Self::ReplicaSlot(value)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct AnimationClock {
    local_anchor_ms: u64,
    shared_anchor_ms: u64,
}

impl AnimationClock {
    const fn local() -> Self {
        Self {
            local_anchor_ms: 0,
            shared_anchor_ms: 0,
        }
    }

    fn sample_time(self, local_now_ms: u64) -> u64 {
        self.shared_anchor_ms
            .saturating_add(local_now_ms.saturating_sub(self.local_anchor_ms))
    }

    fn anchor(&mut self, local_now_ms: u64, shared_now_ms: u64) {
        self.local_anchor_ms = local_now_ms;
        self.shared_anchor_ms = shared_now_ms;
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StandardInput(pub LightAction);

impl From<LightAction> for StandardInput {
    fn from(value: LightAction) -> Self {
        Self(value)
    }
}

/// End-to-end standard engine. `Extension` composes above the designated
/// background and below layers; `Status` composes last. Either can be an
/// external stateful source, or [`EmptySource`].
///
/// Static board layer scenes and the runtime [`SceneTable`] share the layer
/// band: runtime cells apply after the static defaults, so a host-configured
/// cell overrides a board default for the same slot while the TTL overlay
/// still wins above both.
pub struct StandardLightingEngine<
    'scenes,
    Extension,
    Status,
    const N: usize,
    const OVERLAY_CAP: usize,
    const SCENE_CAP: usize = 0,
> {
    compositor: Compositor<Rgb8, N>,
    background: UniformBackground<N>,
    extension: Extension,
    layers: LayerScenes<'scenes, BuiltinEffect>,
    scenes: SceneTable<SCENE_CAP>,
    overlay: TtlOverlay<BuiltinEffect, OVERLAY_CAP>,
    status: Status,
    animation_clock: AnimationClock,
    scene_replace: Option<SceneReplace<SCENE_CAP>>,
    scene_next_transaction: u32,
    scene_expired_transaction: Option<u32>,
    scene_committed_transaction: Option<u32>,
    revision: u32,
    controls: LightingControls,
    output_mode: OutputMode,
    powered: bool,
    wake_active: bool,
    effective_output_enabled: bool,
    output_brightness: u8,
}

impl<'scenes, Extension, Status, const N: usize, const OVERLAY_CAP: usize, const SCENE_CAP: usize>
    StandardLightingEngine<'scenes, Extension, Status, N, OVERLAY_CAP, SCENE_CAP>
{
    pub fn new(
        background: BackgroundState,
        layers: LayerScenes<'scenes, BuiltinEffect>,
        extension: Extension,
        status: Status,
    ) -> Self {
        Self {
            compositor: Compositor::new(Rgb8::BLACK),
            background: UniformBackground::new(background),
            extension,
            layers,
            scenes: SceneTable::new(),
            overlay: TtlOverlay::new(),
            status,
            animation_clock: AnimationClock::local(),
            scene_replace: None,
            scene_next_transaction: 1,
            scene_expired_transaction: None,
            scene_committed_transaction: None,
            revision: 0,
            controls: LightingControls::default(),
            output_mode: OutputMode::AlwaysOn,
            powered: false,
            wake_active: false,
            effective_output_enabled: true,
            output_brightness: u8::MAX,
        }
    }

    /// Install the board's declarative lighting controls before the engine is
    /// served. The configured initial mode becomes authoritative on boot.
    pub const fn with_controls(mut self, controls: LightingControls) -> Self {
        self.output_mode = controls.initial_output_mode;
        self.effective_output_enabled = matches!(self.output_mode, OutputMode::AlwaysOn);
        self.controls = controls;
        self
    }

    pub const fn output_mode(&self) -> OutputMode {
        self.output_mode
    }

    pub const fn powered(&self) -> bool {
        self.powered
    }

    pub const fn wake_active(&self) -> bool {
        self.wake_active
    }

    const fn effective_output(&self) -> bool {
        self.wake_active
            || matches!(self.output_mode, OutputMode::AlwaysOn)
            || matches!(self.output_mode, OutputMode::PoweredOnly) && self.powered
    }

    pub fn state(&self) -> StandardState {
        StandardState {
            revision: self.revision,
            output_enabled: self.effective_output(),
            output_mode: self.output_mode,
            powered: self.powered,
            wake_active: self.wake_active,
            output_brightness: self.output_brightness,
            background: self.background.state(),
            overlay_len: self.overlay.active_len(),
            scene_len: self.scenes.len(),
            scene_policy: self.scenes.policy(),
        }
    }

    pub const fn scene_capacity() -> usize {
        SCENE_CAP
    }

    pub const fn scenes(&self) -> &SceneTable<SCENE_CAP> {
        &self.scenes
    }

    /// Install one persisted scene cell during startup, before the engine is
    /// serving commands. Does not advance the concurrency revision.
    pub fn install_scene_cell(&mut self, cell: SceneTableCell) -> Result<(), StandardError> {
        Self::check_scene_slot(cell.slot)?;
        self.scenes.set(cell)
    }

    /// Install the persisted layer policy during startup.
    pub fn install_scene_policy(&mut self, policy: LayerPolicy) {
        self.scenes.set_policy(policy);
    }

    pub const fn extension(&self) -> &Extension {
        &self.extension
    }

    pub fn extension_mut(&mut self) -> &mut Extension {
        &mut self.extension
    }

    pub const fn status(&self) -> &Status {
        &self.status
    }

    pub fn status_mut(&mut self) -> &mut Status {
        &mut self.status
    }

    fn apply_light_action(&mut self, action: LightAction) -> bool {
        const STEP: u8 = 17;
        let background = &mut self.background.state;
        match action {
            LightAction::BacklightOn => self.output_mode = OutputMode::AlwaysOn,
            LightAction::BacklightOff => self.output_mode = OutputMode::AlwaysOff,
            LightAction::BacklightToggle => {
                self.output_mode = if matches!(self.output_mode, OutputMode::AlwaysOff) {
                    OutputMode::AlwaysOn
                } else {
                    OutputMode::AlwaysOff
                }
            }
            LightAction::OutputModeCycle => self.output_mode = self.output_mode.next(),
            LightAction::BacklightDown => self.output_brightness = self.output_brightness.saturating_sub(STEP),
            LightAction::BacklightUp => self.output_brightness = self.output_brightness.saturating_add(STEP),
            LightAction::BacklightStep => self.output_brightness = self.output_brightness.wrapping_add(STEP),
            LightAction::BacklightToggleBreathing => {
                background.mode = match background.mode {
                    BackgroundMode::Solid => BackgroundMode::Breathe,
                    BackgroundMode::Breathe => BackgroundMode::Solid,
                }
            }
            LightAction::RgbTog => background.enabled = !background.enabled,
            LightAction::RgbModeForward | LightAction::RgbModeReverse => {
                background.mode = match background.mode {
                    BackgroundMode::Solid => BackgroundMode::Breathe,
                    BackgroundMode::Breathe => BackgroundMode::Solid,
                }
            }
            LightAction::RgbHui => background.hue = background.hue.wrapping_add(STEP),
            LightAction::RgbHud => background.hue = background.hue.wrapping_sub(STEP),
            LightAction::RgbSai => background.saturation = background.saturation.saturating_add(STEP),
            LightAction::RgbSad => background.saturation = background.saturation.saturating_sub(STEP),
            LightAction::RgbVai => background.value = background.value.saturating_add(STEP),
            LightAction::RgbVad => background.value = background.value.saturating_sub(STEP),
            LightAction::RgbSpi => background.speed = background.speed.saturating_add(STEP),
            LightAction::RgbSpd => background.speed = background.speed.saturating_sub(STEP),
            LightAction::RgbModePlain => background.mode = BackgroundMode::Solid,
            LightAction::RgbModeBreathe => background.mode = BackgroundMode::Breathe,
            // The standard engine advertises only Solid and Breathe. Other
            // named modes remain available to an extension source/command.
            _ => return false,
        }
        true
    }

    fn check_revision(&self, expected: u32) -> Result<(), StandardError> {
        if expected == self.revision {
            Ok(())
        } else {
            Err(StandardError::RevisionConflict {
                expected,
                current: self.revision,
            })
        }
    }

    fn advance_revision(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }

    fn set_mutable_state(&mut self, state: StandardMutableState) {
        self.output_mode = if state.output_enabled {
            OutputMode::AlwaysOn
        } else {
            OutputMode::AlwaysOff
        };
        self.output_brightness = state.output_brightness;
        self.background.set_state(state.background);
    }

    fn mutable_state(&self) -> StandardMutableState {
        StandardMutableState {
            output_enabled: self.effective_output(),
            output_brightness: self.output_brightness,
            background: self.background.state(),
        }
    }

    fn replica_state<Context: LightingContextProvider>(
        &self,
        local_now_ms: u64,
        context: &Context,
    ) -> Result<StandardReplicaState<OVERLAY_CAP, SCENE_CAP>, StandardError> {
        let sample_time_ms = self.animation_clock.sample_time(local_now_ms);
        let mut overlay = OverlayBatch::new();
        for update in self.overlay.active_updates() {
            let ttl_ms = match update.expires_ms {
                Some(expires_ms) if expires_ms > sample_time_ms => {
                    let remaining = expires_ms - sample_time_ms;
                    Some(NonZeroU32::new(remaining.min(u32::MAX as u64) as u32).unwrap())
                }
                Some(_) => continue,
                None => None,
            };
            overlay.push(OverlayCell {
                slot: update.slot,
                effect: update.effect,
                ttl_ms,
            })?;
        }
        Ok(StandardReplicaState {
            revision: self.revision,
            mutable: self.mutable_state(),
            output_mode: self.output_mode,
            overlay,
            scenes: self.scenes,
            context: *context.lighting_context(),
            sample_time_ms,
            // Filled by handle_command, where the LightingSource bound is
            // available on the Extension parameter.
            extension: None,
        })
    }

    fn replica_overlay(
        replica: &StandardReplicaState<OVERLAY_CAP, SCENE_CAP>,
    ) -> Result<TtlOverlay<BuiltinEffect, OVERLAY_CAP>, StandardError> {
        let mut updates = [OverlayUpdate {
            slot: LedSlot(0),
            effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
            expires_ms: None,
        }; OVERLAY_CAP];
        for (target, cell) in updates.iter_mut().zip(replica.overlay.as_slice().iter().copied()) {
            *target = OverlayUpdate {
                slot: cell.slot,
                effect: cell.effect,
                expires_ms: cell
                    .ttl_ms
                    .and_then(|ttl| replica.sample_time_ms.checked_add(ttl.get() as u64)),
            };
            if cell.ttl_ms.is_some() && target.expires_ms.is_none() {
                return Err(StandardError::DeadlineOverflow);
            }
        }
        let mut overlay = TtlOverlay::new();
        overlay.replace(replica.sample_time_ms, &updates[..replica.overlay.as_slice().len()])?;
        Ok(overlay)
    }

    fn apply_replica(
        &mut self,
        local_now_ms: u64,
        replica: StandardReplicaState<OVERLAY_CAP, SCENE_CAP>,
        overlay: TtlOverlay<BuiltinEffect, OVERLAY_CAP>,
    ) {
        self.overlay = overlay;
        self.set_mutable_state(replica.mutable);
        self.output_mode = replica.output_mode;
        self.scenes = replica.scenes;
        self.revision = replica.revision;
        self.animation_clock.anchor(local_now_ms, replica.sample_time_ms);
    }

    fn expires_at(now_ms: u64, ttl_ms: Option<NonZeroU32>) -> Result<Option<u64>, StandardError> {
        ttl_ms
            .map(|ttl| {
                now_ms
                    .checked_add(ttl.get() as u64)
                    .ok_or(StandardError::DeadlineOverflow)
            })
            .transpose()
    }

    fn overlay_update(now_ms: u64, cell: OverlayCell) -> Result<OverlayUpdate<BuiltinEffect>, StandardError> {
        Ok(OverlayUpdate {
            slot: cell.slot,
            effect: cell.effect,
            expires_ms: Self::expires_at(now_ms, cell.ttl_ms)?,
        })
    }

    fn check_scene_slot(slot: LedSlot) -> Result<(), StandardError> {
        if slot.index() < N {
            Ok(())
        } else {
            Err(StandardError::SceneSlotOutOfRange { slot })
        }
    }

    fn expire_scene_replace(&mut self, now_ms: u64) {
        if self
            .scene_replace
            .as_ref()
            .is_some_and(|replace| now_ms.saturating_sub(replace.last_activity_ms) >= SCENE_TRANSACTION_TIMEOUT_MS)
        {
            self.scene_expired_transaction = self.scene_replace.as_ref().map(|replace| replace.id);
            self.scene_replace = None;
        }
    }

    fn scene_transaction_error(&self, id: u32) -> StandardError {
        if self.scene_expired_transaction == Some(id) {
            StandardError::SceneTransactionExpired
        } else {
            StandardError::InvalidSceneTransaction
        }
    }

    fn begin_scene_replace(
        &mut self,
        now_ms: u64,
        expected_revision: u32,
        cell_count: u16,
    ) -> Result<u32, StandardError> {
        self.expire_scene_replace(now_ms);
        if self.scene_replace.is_some() {
            return Err(StandardError::SceneTransactionBusy);
        }
        if cell_count as usize > SCENE_CAP {
            return Err(StandardError::SceneFull { capacity: SCENE_CAP });
        }
        let id = self.scene_next_transaction;
        self.scene_next_transaction = self.scene_next_transaction.wrapping_add(1).max(1);
        self.scene_expired_transaction = None;
        self.scene_replace = Some(SceneReplace {
            id,
            expected_revision,
            expected_count: cell_count,
            cells: [EMPTY_SCENE_CELL; SCENE_CAP],
            len: 0,
            last_activity_ms: now_ms,
        });
        Ok(id)
    }

    fn put_scene_chunk(
        &mut self,
        now_ms: u64,
        transaction_id: u32,
        offset: u16,
        cells: &SceneChunk,
    ) -> Result<(), StandardError> {
        self.expire_scene_replace(now_ms);
        for cell in cells.as_slice() {
            Self::check_scene_slot(cell.slot)?;
        }
        let error = self.scene_transaction_error(transaction_id);
        let replace = self
            .scene_replace
            .as_mut()
            .filter(|replace| replace.id == transaction_id)
            .ok_or(error)?;
        if offset as usize != replace.len || replace.len + cells.as_slice().len() > replace.expected_count as usize {
            return Err(StandardError::InvalidSceneRequest);
        }
        for cell in cells.as_slice() {
            if replace.cells[..replace.len]
                .iter()
                .any(|staged| staged.layer == cell.layer && staged.slot == cell.slot)
            {
                return Err(StandardError::InvalidSceneRequest);
            }
            replace.cells[replace.len] = *cell;
            replace.len += 1;
        }
        replace.last_activity_ms = now_ms;
        Ok(())
    }

    /// Atomically publish a complete staged replacement. A repeated commit of
    /// the transaction most recently committed answers idempotently with
    /// `changed == false` so a retried commit over a lossy link converges.
    fn commit_scene_replace(&mut self, now_ms: u64, transaction_id: u32) -> Result<bool, StandardError> {
        self.expire_scene_replace(now_ms);
        if self.scene_committed_transaction == Some(transaction_id) && self.scene_replace.is_none() {
            return Ok(false);
        }
        let error = self.scene_transaction_error(transaction_id);
        let replace = self
            .scene_replace
            .as_ref()
            .filter(|replace| replace.id == transaction_id)
            .ok_or(error)?;
        if replace.len != replace.expected_count as usize {
            return Err(StandardError::SceneTransactionIncomplete {
                expected: replace.expected_count,
                received: replace.len as u16,
            });
        }
        self.check_revision(replace.expected_revision)?;
        let replace = self.scene_replace.take().expect("checked above");
        self.scenes.clear();
        for cell in &replace.cells[..replace.len] {
            self.scenes.set(*cell).expect("staged length is table-bounded");
        }
        self.scene_committed_transaction = Some(transaction_id);
        self.scene_expired_transaction = None;
        Ok(true)
    }

    fn abort_scene_replace(&mut self, now_ms: u64, transaction_id: u32) -> Result<(), StandardError> {
        self.expire_scene_replace(now_ms);
        if self
            .scene_replace
            .as_ref()
            .is_some_and(|replace| replace.id == transaction_id)
        {
            self.scene_replace = None;
            return Ok(());
        }
        Err(self.scene_transaction_error(transaction_id))
    }
}

impl<'scenes, Context, Extension, Status, const N: usize, const OVERLAY_CAP: usize, const SCENE_CAP: usize>
    LightingEngine<Context> for StandardLightingEngine<'scenes, Extension, Status, N, OVERLAY_CAP, SCENE_CAP>
where
    Context: LightingContextProvider,
    Extension: LightingSource<Rgb8, Context>,
    Status: LightingSource<Rgb8, Context>,
{
    type Frame = LogicalFrame<Rgb8, N>;
    type Input = StandardInput;
    type Command = StandardCommand<OVERLAY_CAP, SCENE_CAP>;
    type Reply = StandardReply;
    type Error = StandardError;

    fn on_input(&mut self, input: Self::Input, _snapshot: &Context) -> Result<Invalidation, Self::Error> {
        if self.extension.handle_light_action(input.0) || self.apply_light_action(input.0) {
            self.advance_revision();
            Ok(Invalidation::Render)
        } else {
            Ok(Invalidation::None)
        }
    }

    fn handle_command(
        &mut self,
        now_ms: u64,
        command: Self::Command,
        snapshot: &Context,
    ) -> Result<CommandResult<Self::Reply>, Self::Error> {
        let effect_now_ms = self.animation_clock.sample_time(now_ms);
        // Scene reads and transaction bookkeeping have non-state replies or
        // bespoke revision behavior; everything else falls through to the
        // uniform state-readback path below.
        match command {
            StandardCommand::ReadOverlay { offset } => {
                let old_len = self.overlay.active_len();
                self.overlay.prune_expired(effect_now_ms);
                let expired = self.overlay.active_len() != old_len;
                if expired {
                    self.advance_revision();
                }
                let start = (offset as usize).min(self.overlay.active_len());
                let mut cells = OverlayBatch::new();
                for update in self.overlay.active_updates().skip(start).take(OVERLAY_CHUNK_SIZE) {
                    let ttl_ms = update.expires_ms.map(|expires_ms| {
                        let remaining = expires_ms.saturating_sub(effect_now_ms).min(u32::MAX as u64) as u32;
                        NonZeroU32::new(remaining).expect("expired overlays were pruned")
                    });
                    cells
                        .push(OverlayCell {
                            slot: update.slot,
                            effect: update.effect,
                            ttl_ms,
                        })
                        .expect("overlay page is chunk-bounded");
                }
                return Ok(CommandResult::new(
                    StandardReply::OverlayPage(OverlayPage {
                        revision: self.revision,
                        total: self.overlay.active_len().min(u16::MAX as usize) as u16,
                        cells,
                    }),
                    if expired {
                        Invalidation::Render
                    } else {
                        Invalidation::None
                    },
                ));
            }
            StandardCommand::ReadScenes { offset } => {
                return Ok(CommandResult::unchanged(StandardReply::ScenesPage(ScenePage {
                    revision: self.revision,
                    total: self.scenes.len().min(u16::MAX as usize) as u16,
                    cells: self.scenes.page(offset),
                })));
            }
            StandardCommand::ReadCompiledScenes { offset } => {
                let mut cells = SceneChunk::new();
                for (layer, cell) in self.layers.cells().skip(offset as usize).take(SCENE_CHUNK_SIZE) {
                    cells
                        .push(SceneTableCell {
                            layer,
                            slot: cell.slot,
                            effect: cell.effect,
                        })
                        .expect("compiled scene page is chunk-bounded");
                }
                return Ok(CommandResult::unchanged(StandardReply::CompiledScenesPage(
                    CompiledScenePage {
                        total: self.layers.cell_count().min(u16::MAX as usize) as u16,
                        policy: self.layers.policy,
                        cells,
                    },
                )));
            }
            StandardCommand::BeginSceneReplace {
                expected_revision,
                cell_count,
            } => {
                let id = self.begin_scene_replace(now_ms, expected_revision, cell_count)?;
                return Ok(CommandResult::unchanged(StandardReply::SceneTransaction {
                    id,
                    cell_count,
                }));
            }
            StandardCommand::PutSceneChunk {
                transaction_id,
                offset,
                cells,
            } => {
                self.put_scene_chunk(now_ms, transaction_id, offset, &cells)?;
                return Ok(CommandResult::unchanged(StandardReply::State(self.state())));
            }
            StandardCommand::CommitSceneReplace { transaction_id } => {
                // A repeated commit of the last committed transaction is
                // answered idempotently without advancing the revision.
                let committed = self.commit_scene_replace(now_ms, transaction_id)?;
                if committed {
                    self.advance_revision();
                    return Ok(CommandResult::new(
                        StandardReply::State(self.state()),
                        Invalidation::Render,
                    ));
                }
                return Ok(CommandResult::unchanged(StandardReply::State(self.state())));
            }
            StandardCommand::AbortSceneReplace { transaction_id } => {
                self.abort_scene_replace(now_ms, transaction_id)?;
                return Ok(CommandResult::unchanged(StandardReply::State(self.state())));
            }
            _ => {}
        }

        let (mut invalidation, advances_revision) = match command {
            StandardCommand::SetOutputEnabled(enabled) => {
                self.output_mode = if enabled {
                    OutputMode::AlwaysOn
                } else {
                    OutputMode::AlwaysOff
                };
                (Invalidation::Render, true)
            }
            StandardCommand::SetOutputBrightness(level) => {
                self.output_brightness = level;
                (Invalidation::Render, true)
            }
            StandardCommand::SetBackground(state) => {
                self.background.set_state(state);
                (Invalidation::Render, true)
            }
            StandardCommand::PatchBackground(patch) => {
                patch.apply_to(&mut self.background.state);
                (Invalidation::Render, true)
            }
            StandardCommand::SetStateIfRevision {
                expected_revision,
                state,
            } => {
                self.check_revision(expected_revision)?;
                self.set_mutable_state(state);
                (Invalidation::Render, true)
            }
            StandardCommand::SetOverlay(cell) => {
                let update = Self::overlay_update(effect_now_ms, cell)?;
                self.overlay.set(effect_now_ms, update)?;
                (Invalidation::Render, true)
            }
            StandardCommand::SetOverlayIfRevision {
                expected_revision,
                cell,
            } => {
                self.check_revision(expected_revision)?;
                let update = Self::overlay_update(effect_now_ms, cell)?;
                self.overlay.set(effect_now_ms, update)?;
                (Invalidation::Render, true)
            }
            StandardCommand::UnsetOverlay(slot) => {
                let changed = self.overlay.unset(slot);
                (
                    if changed {
                        Invalidation::Render
                    } else {
                        Invalidation::None
                    },
                    true,
                )
            }
            StandardCommand::UnsetOverlayIfRevision {
                expected_revision,
                slot,
            } => {
                self.check_revision(expected_revision)?;
                let changed = self.overlay.unset(slot);
                (
                    if changed {
                        Invalidation::Render
                    } else {
                        Invalidation::None
                    },
                    true,
                )
            }
            StandardCommand::ClearOverlay => {
                let changed = self.overlay.active_len() != 0;
                self.overlay.clear();
                (
                    if changed {
                        Invalidation::Render
                    } else {
                        Invalidation::None
                    },
                    true,
                )
            }
            StandardCommand::ClearOverlayIfRevision { expected_revision } => {
                self.check_revision(expected_revision)?;
                let changed = self.overlay.active_len() != 0;
                self.overlay.clear();
                (
                    if changed {
                        Invalidation::Render
                    } else {
                        Invalidation::None
                    },
                    true,
                )
            }
            StandardCommand::ReplaceOverlay(batch) => {
                let mut updates = [OverlayUpdate {
                    slot: LedSlot(0),
                    effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
                    expires_ms: None,
                }; OVERLAY_CAP];
                for (target, cell) in updates.iter_mut().zip(batch.as_slice().iter().copied()) {
                    *target = Self::overlay_update(effect_now_ms, cell)?;
                }
                self.overlay
                    .replace(effect_now_ms, &updates[..batch.as_slice().len()])?;
                (Invalidation::Render, true)
            }
            StandardCommand::ReplaceOverlayIfRevision {
                expected_revision,
                batch,
            } => {
                self.check_revision(expected_revision)?;
                let mut updates = [OverlayUpdate {
                    slot: LedSlot(0),
                    effect: BuiltinEffect::Solid { color: Rgb8::BLACK },
                    expires_ms: None,
                }; OVERLAY_CAP];
                for (target, cell) in updates.iter_mut().zip(batch.as_slice().iter().copied()) {
                    *target = Self::overlay_update(effect_now_ms, cell)?;
                }
                self.overlay
                    .replace(effect_now_ms, &updates[..batch.as_slice().len()])?;
                (Invalidation::Render, true)
            }
            StandardCommand::ExportReplica(slot) => {
                let mut replica = self.replica_state(now_ms, snapshot)?;
                replica.extension = self.extension.extension_state();
                slot.put(replica)?;
                (Invalidation::None, false)
            }
            StandardCommand::ApplyReplica(slot) => {
                let replica = slot.take()?;
                let overlay = Self::replica_overlay(&replica)?;
                match (replica.extension, self.extension.extension_state()) {
                    (None, None) => {}
                    (Some(extension), Some(_)) if self.extension.apply_extension_state(extension) => {}
                    _ => return Err(StandardError::ExtensionUnsupported),
                }
                self.apply_replica(now_ms, replica, overlay);
                (Invalidation::Render, false)
            }
            StandardCommand::SetSceneCellIfRevision {
                expected_revision,
                cell,
            } => {
                self.check_revision(expected_revision)?;
                Self::check_scene_slot(cell.slot)?;
                self.scenes.set(cell)?;
                (Invalidation::Render, true)
            }
            StandardCommand::UnsetSceneCellIfRevision {
                expected_revision,
                layer,
                slot,
            } => {
                self.check_revision(expected_revision)?;
                let changed = self.scenes.unset(layer, slot);
                (
                    if changed {
                        Invalidation::Render
                    } else {
                        Invalidation::None
                    },
                    true,
                )
            }
            StandardCommand::SetLayerPolicyIfRevision {
                expected_revision,
                policy,
            } => {
                self.check_revision(expected_revision)?;
                let changed = self.scenes.set_policy(policy);
                (
                    if changed {
                        Invalidation::Render
                    } else {
                        Invalidation::None
                    },
                    true,
                )
            }
            StandardCommand::ReadExtension => {
                return Ok(CommandResult::unchanged(StandardReply::Extension(ExtensionPage {
                    revision: self.revision,
                    descriptor: self.extension.extension_descriptor(),
                    state: self.extension.extension_state(),
                })));
            }
            StandardCommand::SetExtensionIfRevision {
                expected_revision,
                state,
            } => {
                self.check_revision(expected_revision)?;
                if !self.extension.apply_extension_state(state) {
                    return Err(StandardError::ExtensionUnsupported);
                }
                (Invalidation::Render, true)
            }
            StandardCommand::ReadState => (Invalidation::None, false),
            StandardCommand::ReadOverlay { .. }
            | StandardCommand::ReadScenes { .. }
            | StandardCommand::ReadCompiledScenes { .. }
            | StandardCommand::BeginSceneReplace { .. }
            | StandardCommand::PutSceneChunk { .. }
            | StandardCommand::CommitSceneReplace { .. }
            | StandardCommand::AbortSceneReplace { .. } => unreachable!("handled above"),
        };
        if advances_revision {
            self.advance_revision();
            if invalidation == Invalidation::None {
                invalidation = Invalidation::StateChanged;
            }
        }
        Ok(CommandResult::new(StandardReply::State(self.state()), invalidation))
    }

    fn render(
        &mut self,
        input: RenderInput<'_, Context>,
        frame: &mut Self::Frame,
    ) -> Result<RenderOutcome, Self::Error> {
        let effect_now_ms = self.animation_clock.sample_time(input.now_ms);
        let overlay_len = self.overlay.active_len();
        self.overlay.prune_expired(effect_now_ms);
        let mut state_changed = self.overlay.active_len() != overlay_len;
        if state_changed {
            self.advance_revision();
        }
        let context = input.snapshot.lighting_context();
        let powered = context.powered;
        let wake_active = self
            .controls
            .wake_layer
            .is_some_and(|layer| context.layers.is_active(layer));
        let effective_output_enabled = wake_active
            || matches!(self.output_mode, OutputMode::AlwaysOn)
            || matches!(self.output_mode, OutputMode::PoweredOnly) && powered;
        if self.powered != powered
            || self.wake_active != wake_active
            || self.effective_output_enabled != effective_output_enabled
        {
            self.powered = powered;
            self.wake_active = wake_active;
            self.effective_output_enabled = effective_output_enabled;
            state_changed = true;
        }
        let indicator_cell = self.controls.output_mode_indicator.map(|indicator| SceneCell {
            slot: indicator.slot,
            effect: indicator.effect(self.output_mode),
        });
        let Self {
            compositor,
            background,
            extension,
            layers,
            scenes,
            overlay,
            status,
            animation_clock: _,
            controls: _,
            output_mode: _,
            powered: _,
            wake_active: _,
            effective_output_enabled,
            output_brightness,
            scene_replace: _,
            scene_next_transaction: _,
            scene_expired_transaction: _,
            scene_committed_transaction: _,
            revision: _,
        } = self;
        let mut transaction = compositor.begin(effect_now_ms, input.snapshot, Rgb8::BLACK, frame);
        transaction.apply(priority::BACKGROUND, background)?;
        transaction.apply(priority::EXTENSION, extension)?;
        transaction.apply(priority::LAYER, layers)?;
        // Same band, later call: runtime cells override static defaults.
        transaction.apply(priority::LAYER, scenes)?;
        transaction.apply(priority::HOST_OVERLAY, overlay)?;
        transaction.apply(priority::STATUS, status)?;
        if wake_active && let Some(indicator_cell) = indicator_cell.as_ref() {
            let mut indicator = SparseScene {
                cells: core::slice::from_ref(indicator_cell),
            };
            transaction.apply(priority::STATUS, &mut indicator)?;
        }
        let mut transform = BrightnessTransform::new(if *effective_output_enabled {
            *output_brightness
        } else {
            0
        });
        let result = transaction.finish_with(&mut transform);
        let next_wake_in_ms = result.next_wake_ms.map(|deadline| {
            let delay = deadline.saturating_sub(effect_now_ms).clamp(1, u32::MAX as u64);
            NonZeroU32::new(delay as u32).expect("clamped delay is nonzero")
        });
        Ok(RenderOutcome {
            changed: result.changed,
            state_changed,
            next_wake_in_ms,
        })
    }

    fn on_presented(&mut self, frame: &Self::Frame) {
        self.compositor.commit(frame);
    }
}

fn hsv(hue: u8, saturation: u8, value: u8) -> Rgb8 {
    if saturation == 0 {
        return Rgb8::new(value, value, value);
    }
    let region = hue / 43;
    let remainder = (hue - region * 43) as u16 * 6;
    let p = (value as u16 * (255 - saturation as u16) / 255) as u8;
    let q = (value as u16 * (255 - (saturation as u16 * remainder / 255)) / 255) as u8;
    let t = (value as u16 * (255 - (saturation as u16 * (255 - remainder) / 255)) / 255) as u8;
    match region {
        0 => Rgb8::new(value, t, p),
        1 => Rgb8::new(q, value, p),
        2 => Rgb8::new(p, value, t),
        3 => Rgb8::new(p, q, value),
        4 => Rgb8::new(t, p, value),
        _ => Rgb8::new(value, p, q),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::source::SparseScene;
    use crate::lighting::{LayerPolicy, LayerScene, LayerState, LightingContext, SceneCell};

    const RED: Rgb8 = Rgb8::new(200, 0, 0);
    const GREEN: Rgb8 = Rgb8::new(0, 200, 0);

    type Engine = StandardLightingEngine<'static, EmptySource, EmptySource, 2, 2>;

    #[derive(Copy, Clone)]
    struct ReplicaExtension {
        state: ExtensionState,
        accept: bool,
    }

    impl<Context> LightingSource<Rgb8, Context> for ReplicaExtension {
        fn len(&self, _: &SourceRenderInput<'_, Context>) -> usize {
            0
        }

        fn slot(&self, _: usize, _: &SourceRenderInput<'_, Context>) -> LedSlot {
            unreachable!("replica test extension has no targets")
        }

        fn contribution(&mut self, _: usize, _: &SourceRenderInput<'_, Context>) -> Contribution<Rgb8> {
            unreachable!("replica test extension has no samples")
        }

        fn extension_state(&self) -> Option<ExtensionState> {
            Some(self.state)
        }

        fn apply_extension_state(&mut self, state: ExtensionState) -> bool {
            if !self.accept {
                return false;
            }
            self.state = state;
            true
        }
    }

    static LAYER_CELLS: [SceneCell<BuiltinEffect>; 1] = [SceneCell {
        slot: LedSlot(0),
        effect: BuiltinEffect::Solid { color: RED },
    }];
    static LAYERS: [LayerScene<'static, BuiltinEffect>; 1] = [LayerScene {
        layer: 1,
        cells: &LAYER_CELLS,
    }];
    static STATUS_CELLS: [SceneCell<BuiltinEffect>; 1] = [SceneCell {
        slot: LedSlot(1),
        effect: BuiltinEffect::Solid { color: GREEN },
    }];

    fn engine() -> Engine {
        StandardLightingEngine::new(
            BackgroundState {
                value: 10,
                ..BackgroundState::default()
            },
            LayerScenes {
                scenes: &LAYERS,
                policy: LayerPolicy::ActiveStack,
            },
            EmptySource,
            EmptySource,
        )
    }

    fn replica_engine(accept: bool) -> StandardLightingEngine<'static, ReplicaExtension, EmptySource, 2, 2> {
        StandardLightingEngine::new(
            BackgroundState {
                value: 10,
                ..BackgroundState::default()
            },
            LayerScenes {
                scenes: &LAYERS,
                policy: LayerPolicy::ActiveStack,
            },
            ReplicaExtension {
                state: ExtensionState {
                    effect: 0,
                    palette: 0,
                    value: 10,
                    speed: 20,
                },
                accept,
            },
            EmptySource,
        )
    }

    fn context(layer: u8) -> LightingContext {
        LightingContext {
            layers: LayerState::new(layer, 0, 1 | (1 << layer)),
            indicators: Default::default(),
            powered: false,
        }
    }

    #[test]
    fn replica_snapshot_preserves_state_scenes_context_ttl_and_animation_phase() {
        type SceneEngine = StandardLightingEngine<'static, EmptySource, EmptySource, 2, 2, 4>;
        fn scene_engine() -> SceneEngine {
            SceneEngine::new(
                BackgroundState {
                    value: 10,
                    ..BackgroundState::default()
                },
                LayerScenes {
                    scenes: &LAYERS,
                    policy: LayerPolicy::ActiveStack,
                },
                EmptySource,
                EmptySource,
            )
        }
        let mut authority = scene_engine();
        let authority_context = context(3);
        authority
            .handle_command(
                100,
                StandardCommand::SetBackground(BackgroundState {
                    mode: BackgroundMode::Breathe,
                    speed: 200,
                    ..BackgroundState::default()
                }),
                &authority_context,
            )
            .unwrap();
        authority
            .handle_command(
                100,
                StandardCommand::SetOverlay(OverlayCell {
                    slot: LedSlot(1),
                    effect: BuiltinEffect::Solid { color: GREEN },
                    ttl_ms: NonZeroU32::new(100),
                }),
                &authority_context,
            )
            .unwrap();
        let scene_cell = SceneTableCell {
            layer: 3,
            slot: LedSlot(0),
            effect: BuiltinEffect::Solid { color: RED },
        };
        authority
            .handle_command(
                100,
                StandardCommand::SetSceneCellIfRevision {
                    expected_revision: 2,
                    cell: scene_cell,
                },
                &authority_context,
            )
            .unwrap();

        static EXPORT: StandardReplicaSlot<2, 4> = StandardReplicaSlot::new();
        authority
            .handle_command(120, StandardCommand::ExportReplica(&EXPORT), &authority_context)
            .unwrap();
        let snapshot = EXPORT.take().unwrap();
        assert_eq!(snapshot.context, authority_context);
        assert_eq!(snapshot.sample_time_ms, 120);
        assert_eq!(snapshot.overlay.as_slice()[0].ttl_ms, NonZeroU32::new(80));
        assert_eq!(snapshot.scenes.as_slice(), &[scene_cell]);

        let mut authority_frame = LogicalFrame::new(Rgb8::BLACK);
        authority
            .render(
                RenderInput {
                    now_ms: 136,
                    snapshot: &authority_context,
                },
                &mut authority_frame,
            )
            .unwrap();

        static APPLY: StandardReplicaSlot<2, 4> = StandardReplicaSlot::new();
        APPLY.put(snapshot).unwrap();
        let mut replica = scene_engine();
        replica
            .handle_command(1_000, StandardCommand::ApplyReplica(&APPLY), &authority_context)
            .unwrap();
        let mut replica_frame = LogicalFrame::new(Rgb8::BLACK);
        replica
            .render(
                RenderInput {
                    now_ms: 1_016,
                    snapshot: &snapshot.context,
                },
                &mut replica_frame,
            )
            .unwrap();

        assert_eq!(replica.state().revision, snapshot.revision);
        assert_eq!(replica.scenes().as_slice(), &[scene_cell]);
        assert_eq!(replica_frame, authority_frame);
    }

    #[test]
    fn replica_slot_rejects_overwrite_and_empty_take() {
        let slot = StandardReplicaSlot::<2>::new();
        let snapshot = StandardReplicaState {
            revision: 1,
            mutable: StandardMutableState {
                output_enabled: true,
                output_brightness: 42,
                background: BackgroundState::default(),
            },
            output_mode: OutputMode::AlwaysOn,
            overlay: OverlayBatch::new(),
            scenes: SceneTable::new(),
            context: context(0),
            sample_time_ms: 9,
            extension: None,
        };
        slot.put(snapshot).unwrap();
        assert_eq!(slot.put(snapshot), Err(ReplicaSlotError::Busy));
        assert_eq!(slot.take(), Ok(snapshot));
        assert_eq!(slot.take(), Err(ReplicaSlotError::Empty));
    }

    #[test]
    fn replica_application_is_atomic_across_common_and_extension_state() {
        let next_extension = ExtensionState {
            effect: 1,
            palette: 2,
            value: 30,
            speed: 40,
        };
        let snapshot = StandardReplicaState {
            revision: 9,
            mutable: StandardMutableState {
                output_enabled: false,
                output_brightness: 42,
                background: BackgroundState {
                    value: 99,
                    ..BackgroundState::default()
                },
            },
            output_mode: OutputMode::AlwaysOff,
            overlay: OverlayBatch::new(),
            scenes: SceneTable::new(),
            context: context(0),
            sample_time_ms: 50,
            extension: Some(next_extension),
        };

        let mut declining = replica_engine(false);
        let before = declining.state();
        let before_extension = declining.extension().state;
        static DECLINING_SLOT: StandardReplicaSlot<2> = StandardReplicaSlot::new();
        DECLINING_SLOT.put(snapshot).unwrap();
        assert_eq!(
            declining.handle_command(100, StandardCommand::ApplyReplica(&DECLINING_SLOT), &context(0),),
            Err(StandardError::ExtensionUnsupported)
        );
        assert_eq!(declining.state(), before);
        assert_eq!(declining.extension().state, before_extension);

        let mut accepting = replica_engine(true);
        static ACCEPTING_SLOT: StandardReplicaSlot<2> = StandardReplicaSlot::new();
        ACCEPTING_SLOT.put(snapshot).unwrap();
        accepting
            .handle_command(100, StandardCommand::ApplyReplica(&ACCEPTING_SLOT), &context(0))
            .unwrap();
        assert_eq!(accepting.state().revision, 9);
        assert_eq!(accepting.state().output_brightness, 42);
        assert_eq!(accepting.extension().state, next_extension);
    }

    #[test]
    fn invalid_replica_common_state_does_not_mutate_extension() {
        let mut overlay = OverlayBatch::new();
        for _ in 0..2 {
            overlay
                .push(OverlayCell {
                    slot: LedSlot(0),
                    effect: BuiltinEffect::Solid { color: RED },
                    ttl_ms: None,
                })
                .unwrap();
        }
        let snapshot = StandardReplicaState {
            revision: 9,
            mutable: StandardMutableState {
                output_enabled: true,
                output_brightness: 42,
                background: BackgroundState::default(),
            },
            output_mode: OutputMode::AlwaysOn,
            overlay,
            scenes: SceneTable::new(),
            context: context(0),
            sample_time_ms: 50,
            extension: Some(ExtensionState {
                effect: 1,
                palette: 2,
                value: 30,
                speed: 40,
            }),
        };
        let mut engine = replica_engine(true);
        let before = engine.extension().state;
        static INVALID_SLOT: StandardReplicaSlot<2> = StandardReplicaSlot::new();
        INVALID_SLOT.put(snapshot).unwrap();
        assert_eq!(
            engine.handle_command(100, StandardCommand::ApplyReplica(&INVALID_SLOT), &context(0),),
            Err(StandardError::Overlay(OverlayError::DuplicateSlot { slot: LedSlot(0) }))
        );
        assert_eq!(engine.extension().state, before);
        assert_eq!(engine.state().revision, 0);
    }

    #[test]
    fn output_mode_cycles_and_wake_layer_temporarily_overrides_policy() {
        let mut engine = engine().with_controls(LightingControls {
            output_toggle_user_action: None,
            output_mode_cycle_user_action: Some(13),
            wake_layer: Some(2),
            initial_output_mode: OutputMode::PoweredOnly,
            powered_only_scope: super::super::source::PoweredOnlyScope::Local,
            output_mode_indicator: Some(super::super::source::OutputModeIndicator {
                slot: LedSlot(1),
                always_on: BuiltinEffect::Solid { color: GREEN },
                always_off: BuiltinEffect::Solid { color: RED },
                powered_only: BuiltinEffect::Solid {
                    color: Rgb8::new(0, 0, 200),
                },
            }),
        });
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let battery_context = context(0);
        engine
            .render(
                RenderInput {
                    now_ms: 0,
                    snapshot: &battery_context,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(frame.as_slice(), &[Rgb8::BLACK, Rgb8::BLACK]);
        assert!(!engine.state().output_enabled);

        let mut powered_context = battery_context;
        powered_context.powered = true;
        engine
            .render(
                RenderInput {
                    now_ms: 1,
                    snapshot: &powered_context,
                },
                &mut frame,
            )
            .unwrap();
        assert!(engine.state().output_enabled);

        engine
            .on_input(StandardInput(LightAction::OutputModeCycle), &powered_context)
            .unwrap();
        assert_eq!(engine.output_mode(), OutputMode::AlwaysOn);
        engine
            .on_input(StandardInput(LightAction::OutputModeCycle), &powered_context)
            .unwrap();
        assert_eq!(engine.output_mode(), OutputMode::AlwaysOff);

        let magic_context = context(2);
        engine
            .render(
                RenderInput {
                    now_ms: 2,
                    snapshot: &magic_context,
                },
                &mut frame,
            )
            .unwrap();
        assert!(engine.state().wake_active);
        assert!(engine.state().output_enabled);
        assert_eq!(frame.as_slice()[1], RED);

        engine
            .render(
                RenderInput {
                    now_ms: 3,
                    snapshot: &battery_context,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(frame.as_slice(), &[Rgb8::BLACK, Rgb8::BLACK]);
        assert_eq!(engine.output_mode(), OutputMode::AlwaysOff);
    }

    #[test]
    fn background_patch_preserves_unmentioned_fields() {
        let mut engine = engine();
        let snapshot = context(0);
        let before = engine.state();
        let reply = engine
            .handle_command(
                0,
                StandardCommand::PatchBackground(BackgroundPatch {
                    value: Some(77),
                    ..BackgroundPatch::default()
                }),
                &snapshot,
            )
            .unwrap()
            .reply
            .state()
            .unwrap();

        assert_eq!(reply.background.value, 77);
        assert_eq!(reply.background.enabled, before.background.enabled);
        assert_eq!(reply.background.hue, before.background.hue);
        assert_eq!(reply.background.saturation, before.background.saturation);
        assert_eq!(reply.background.speed, before.background.speed);
        assert_eq!(reply.background.mode, before.background.mode);
        assert_eq!(reply.output_enabled, before.output_enabled);
        assert_eq!(reply.output_brightness, before.output_brightness);
        assert_eq!(reply.revision, before.revision + 1);
    }

    #[test]
    fn revision_checks_are_atomic_and_cover_inputs_and_expiry() {
        let mut engine = engine();
        let snapshot = context(0);
        assert_eq!(engine.state().revision, 0);

        engine
            .handle_command(
                10,
                StandardCommand::PatchBackground(BackgroundPatch {
                    value: Some(77),
                    ..BackgroundPatch::default()
                }),
                &snapshot,
            )
            .unwrap();
        assert_eq!(engine.state().revision, 1);

        let desired = StandardMutableState {
            output_enabled: false,
            output_brightness: 12,
            background: BackgroundState {
                value: 99,
                ..BackgroundState::default()
            },
        };
        assert_eq!(
            engine.handle_command(
                10,
                StandardCommand::SetStateIfRevision {
                    expected_revision: 0,
                    state: desired,
                },
                &snapshot,
            ),
            Err(StandardError::RevisionConflict {
                expected: 0,
                current: 1,
            })
        );
        assert!(engine.state().output_enabled, "a stale update is all-or-nothing");

        let reply = engine
            .handle_command(
                10,
                StandardCommand::SetStateIfRevision {
                    expected_revision: 1,
                    state: desired,
                },
                &snapshot,
            )
            .unwrap()
            .reply
            .state()
            .unwrap();
        assert_eq!(reply.revision, 2);
        assert!(!reply.output_enabled);

        engine
            .on_input(StandardInput(LightAction::BacklightOn), &snapshot)
            .unwrap();
        assert_eq!(engine.state().revision, 3);
        assert_eq!(
            engine.on_input(StandardInput(LightAction::RgbModeRainbow), &snapshot),
            Ok(Invalidation::None)
        );
        assert_eq!(engine.state().revision, 3);

        engine
            .handle_command(
                10,
                StandardCommand::SetOverlayIfRevision {
                    expected_revision: 3,
                    cell: OverlayCell {
                        slot: LedSlot(0),
                        effect: BuiltinEffect::Solid { color: GREEN },
                        ttl_ms: NonZeroU32::new(1),
                    },
                },
                &snapshot,
            )
            .unwrap();
        assert_eq!(engine.state().revision, 4);

        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let first_expiry = engine
            .render(
                RenderInput {
                    now_ms: 11,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert!(first_expiry.state_changed);
        assert_eq!(engine.state().revision, 5, "TTL expiry is authoritative state");
        let second_expiry = engine
            .render(
                RenderInput {
                    now_ms: 12,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert!(!second_expiry.state_changed);
        assert_eq!(engine.state().revision, 5, "expiry advances exactly once");
    }

    #[test]
    fn overlay_pages_sample_remaining_ttl_and_prune_with_revision_advance() {
        let mut engine = engine();
        let snapshot = context(0);
        engine
            .handle_command(
                100,
                StandardCommand::SetOverlay(OverlayCell {
                    slot: LedSlot(0),
                    effect: BuiltinEffect::Solid { color: RED },
                    ttl_ms: None,
                }),
                &snapshot,
            )
            .unwrap();
        engine
            .handle_command(
                100,
                StandardCommand::SetOverlay(OverlayCell {
                    slot: LedSlot(1),
                    effect: BuiltinEffect::Solid { color: GREEN },
                    ttl_ms: NonZeroU32::new(50),
                }),
                &snapshot,
            )
            .unwrap();

        let sampled = engine
            .handle_command(120, StandardCommand::ReadOverlay { offset: 0 }, &snapshot)
            .unwrap();
        assert_eq!(sampled.invalidation, Invalidation::None);
        let StandardReply::OverlayPage(page) = sampled.reply else {
            panic!("expected overlay page")
        };
        assert_eq!(page.revision, 2);
        assert_eq!(page.total, 2);
        assert_eq!(page.cells.as_slice()[0].ttl_ms, None);
        assert_eq!(page.cells.as_slice()[1].ttl_ms, NonZeroU32::new(30));

        let pruned = engine
            .handle_command(150, StandardCommand::ReadOverlay { offset: 0 }, &snapshot)
            .unwrap();
        assert_eq!(pruned.invalidation, Invalidation::Render);
        let StandardReply::OverlayPage(page) = pruned.reply else {
            panic!("expected overlay page")
        };
        assert_eq!(page.revision, 3);
        assert_eq!(page.total, 1);
        assert_eq!(page.cells.as_slice()[0].slot, LedSlot(0));
    }

    #[test]
    fn compiled_scene_pages_preserve_layer_and_are_separate_from_runtime_scenes() {
        let mut engine = scene_engine();
        set_cell(&mut engine, 0, scene_cell(7, 1, GREEN));

        let reply = engine
            .handle_command(0, StandardCommand::ReadCompiledScenes { offset: 0 }, &context(0))
            .unwrap()
            .reply;
        let StandardReply::CompiledScenesPage(page) = reply else {
            panic!("expected compiled scene page")
        };
        assert_eq!(page.total, 1);
        assert_eq!(page.policy, LayerPolicy::ActiveStack);
        assert_eq!(page.cells.as_slice(), &[scene_cell(1, 0, RED)]);

        let runtime = scenes_page(&mut engine, 0);
        assert_eq!(runtime.total, 1);
        assert_eq!(runtime.cells.as_slice(), &[scene_cell(7, 1, GREEN)]);

        let mut empty: Engine = StandardLightingEngine::new(
            BackgroundState::default(),
            LayerScenes {
                scenes: &[],
                policy: LayerPolicy::ActiveStack,
            },
            EmptySource,
            EmptySource,
        );
        let StandardReply::CompiledScenesPage(empty_page) = empty
            .handle_command(0, StandardCommand::ReadCompiledScenes { offset: 0 }, &context(0))
            .unwrap()
            .reply
        else {
            panic!("expected compiled scene page")
        };
        assert_eq!(empty_page.total, 0);
        assert_eq!(empty_page.policy, LayerPolicy::ActiveStack);
        assert!(empty_page.cells.as_slice().is_empty());
    }

    #[test]
    fn disabling_background_does_not_disable_layer_or_status_sources() {
        type StatusEngine = StandardLightingEngine<'static, EmptySource, SparseScene<'static, BuiltinEffect>, 2, 2>;
        let mut engine = StatusEngine::new(
            BackgroundState {
                value: 10,
                ..BackgroundState::default()
            },
            LayerScenes {
                scenes: &LAYERS,
                policy: LayerPolicy::ActiveStack,
            },
            EmptySource,
            SparseScene { cells: &STATUS_CELLS },
        );
        let snapshot = context(1);
        let before = engine.state();
        engine
            .handle_command(
                0,
                StandardCommand::PatchBackground(BackgroundPatch {
                    enabled: Some(false),
                    ..BackgroundPatch::default()
                }),
                &snapshot,
            )
            .unwrap();

        assert!(engine.state().output_enabled);
        assert_eq!(engine.state().output_brightness, before.output_brightness);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        engine
            .render(
                RenderInput {
                    now_ms: 0,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(frame.as_slice(), &[RED, GREEN]);
    }

    #[test]
    fn standard_engine_composes_background_layer_and_expiring_overlay() {
        let mut engine = engine();
        let snapshot = context(1);
        engine
            .handle_command(
                100,
                StandardCommand::SetOverlay(OverlayCell {
                    slot: LedSlot(1),
                    effect: BuiltinEffect::Solid { color: GREEN },
                    ttl_ms: NonZeroU32::new(10),
                }),
                &snapshot,
            )
            .unwrap();
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let outcome = engine
            .render(
                RenderInput {
                    now_ms: 100,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(frame.as_slice(), &[RED, GREEN]);
        assert_eq!(outcome.next_wake_in_ms, NonZeroU32::new(10));
        <Engine as LightingEngine<LightingContext>>::on_presented(&mut engine, &frame);

        engine
            .render(
                RenderInput {
                    now_ms: 110,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(frame.as_slice(), &[RED, Rgb8::new(10, 10, 10)]);
        assert_eq!(engine.state().overlay_len, 0);
    }

    #[test]
    fn expired_overlay_capacity_is_reclaimed_and_light_actions_are_scoped() {
        let mut engine = engine();
        let snapshot = context(0);
        for slot in [LedSlot(0), LedSlot(1)] {
            engine
                .handle_command(
                    0,
                    StandardCommand::SetOverlay(OverlayCell {
                        slot,
                        effect: BuiltinEffect::Solid { color: GREEN },
                        ttl_ms: NonZeroU32::new(1),
                    }),
                    &snapshot,
                )
                .unwrap();
        }
        assert!(
            engine
                .handle_command(
                    1,
                    StandardCommand::SetOverlay(OverlayCell {
                        slot: LedSlot(0),
                        effect: BuiltinEffect::Solid { color: RED },
                        ttl_ms: None,
                    }),
                    &snapshot,
                )
                .is_ok()
        );

        let before = engine.state();
        assert_eq!(
            engine.on_input(StandardInput(LightAction::RgbModeRainbow), &snapshot),
            Ok(Invalidation::None)
        );
        assert_eq!(engine.state(), before);
        assert_eq!(
            engine.on_input(StandardInput(LightAction::BacklightOff), &snapshot),
            Ok(Invalidation::Render)
        );
        assert!(!engine.state().output_enabled);
    }

    const BLUE: Rgb8 = Rgb8::new(0, 0, 200);

    type SceneEngine = StandardLightingEngine<'static, EmptySource, EmptySource, 2, 2, 4>;

    fn scene_engine() -> SceneEngine {
        StandardLightingEngine::new(
            BackgroundState {
                value: 10,
                ..BackgroundState::default()
            },
            LayerScenes {
                scenes: &LAYERS,
                policy: LayerPolicy::ActiveStack,
            },
            EmptySource,
            EmptySource,
        )
    }

    fn scene_cell(layer: u8, slot: u16, color: Rgb8) -> SceneTableCell {
        SceneTableCell {
            layer,
            slot: LedSlot(slot),
            effect: BuiltinEffect::Solid { color },
        }
    }

    fn set_cell(engine: &mut SceneEngine, revision: u32, cell: SceneTableCell) -> StandardState {
        engine
            .handle_command(
                0,
                StandardCommand::SetSceneCellIfRevision {
                    expected_revision: revision,
                    cell,
                },
                &context(0),
            )
            .unwrap()
            .reply
            .state()
            .unwrap()
    }

    fn scenes_page(engine: &mut SceneEngine, offset: u16) -> ScenePage {
        match engine
            .handle_command(0, StandardCommand::ReadScenes { offset }, &context(0))
            .unwrap()
            .reply
        {
            StandardReply::ScenesPage(page) => page,
            other => panic!("expected page, got {other:?}"),
        }
    }

    #[test]
    fn scene_crud_is_revision_checked_and_keyed_by_layer_and_slot() {
        let mut engine = scene_engine();
        let snapshot = context(0);

        let state = set_cell(&mut engine, 0, scene_cell(1, 0, GREEN));
        assert_eq!(state.revision, 1);
        assert_eq!(state.scene_len, 1);

        // Same (layer, slot) updates in place; a new pair appends.
        let state = set_cell(&mut engine, 1, scene_cell(1, 0, BLUE));
        assert_eq!(state.scene_len, 1);
        let state = set_cell(&mut engine, 2, scene_cell(2, 0, RED));
        assert_eq!(state.scene_len, 2);

        assert_eq!(
            engine.handle_command(
                0,
                StandardCommand::SetSceneCellIfRevision {
                    expected_revision: 0,
                    cell: scene_cell(0, 1, RED),
                },
                &snapshot,
            ),
            Err(StandardError::RevisionConflict {
                expected: 0,
                current: 3
            })
        );
        assert_eq!(engine.state().scene_len, 2, "a stale write is all-or-nothing");

        // Out-of-range slots never reach the table.
        assert_eq!(
            engine.handle_command(
                0,
                StandardCommand::SetSceneCellIfRevision {
                    expected_revision: 3,
                    cell: scene_cell(0, 9, RED),
                },
                &snapshot,
            ),
            Err(StandardError::SceneSlotOutOfRange { slot: LedSlot(9) })
        );

        // Unset of a missing cell keeps the revision moving but not the frame.
        let result = engine
            .handle_command(
                0,
                StandardCommand::UnsetSceneCellIfRevision {
                    expected_revision: 3,
                    layer: 7,
                    slot: LedSlot(0),
                },
                &snapshot,
            )
            .unwrap();
        assert_eq!(result.invalidation, Invalidation::StateChanged);
        let state = result.reply.state().unwrap();
        assert_eq!(state.scene_len, 2);
        assert_eq!(state.revision, 4);

        let state = engine
            .handle_command(
                0,
                StandardCommand::UnsetSceneCellIfRevision {
                    expected_revision: 4,
                    layer: 1,
                    slot: LedSlot(0),
                },
                &snapshot,
            )
            .unwrap()
            .reply
            .state()
            .unwrap();
        assert_eq!(state.scene_len, 1);

        // Capacity is enforced with the table's own limit.
        for (revision, (layer, slot)) in [(5u32, (5u8, 0u16)), (6, (5, 1)), (7, (6, 0))] {
            set_cell(&mut engine, revision, scene_cell(layer, slot, RED));
        }
        assert_eq!(
            engine.handle_command(
                0,
                StandardCommand::SetSceneCellIfRevision {
                    expected_revision: 8,
                    cell: scene_cell(9, 1, RED),
                },
                &snapshot,
            ),
            Err(StandardError::SceneFull { capacity: 4 })
        );
    }

    #[test]
    fn scene_pages_echo_revision_and_clamp() {
        let mut engine = scene_engine();
        set_cell(&mut engine, 0, scene_cell(0, 0, RED));
        set_cell(&mut engine, 1, scene_cell(0, 1, GREEN));

        let page = scenes_page(&mut engine, 0);
        assert_eq!(page.revision, 2);
        assert_eq!(page.total, 2);
        assert_eq!(page.cells.as_slice().len(), 2);
        let tail = scenes_page(&mut engine, 99);
        assert_eq!(tail.total, 2);
        assert!(tail.cells.as_slice().is_empty());
    }

    #[test]
    fn runtime_scene_overrides_static_layer_and_yields_to_overlay() {
        let mut engine = scene_engine();
        let snapshot = context(1);

        // Static layer 1 paints slot 0 RED; runtime cell overrides to BLUE.
        set_cell(&mut engine, 0, scene_cell(1, 0, BLUE));
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        engine
            .render(
                RenderInput {
                    now_ms: 0,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(frame.as_slice()[0], BLUE);

        // The TTL overlay still wins above runtime scenes.
        engine
            .handle_command(
                0,
                StandardCommand::SetOverlay(OverlayCell {
                    slot: LedSlot(0),
                    effect: BuiltinEffect::Solid { color: GREEN },
                    ttl_ms: None,
                }),
                &snapshot,
            )
            .unwrap();
        engine
            .render(
                RenderInput {
                    now_ms: 1,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(frame.as_slice()[0], GREEN);
    }

    #[test]
    fn layer_policy_switches_between_effective_only_and_active_stack() {
        let mut engine = scene_engine();
        // Base layer paints slot 1; effective layer paints slot 0.
        set_cell(&mut engine, 0, scene_cell(0, 1, GREEN));
        set_cell(&mut engine, 1, scene_cell(1, 0, BLUE));
        let snapshot = context(1);

        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        engine
            .render(
                RenderInput {
                    now_ms: 0,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(frame.as_slice(), &[BLUE, GREEN], "ActiveStack falls through sparsely");

        let state = engine
            .handle_command(
                0,
                StandardCommand::SetLayerPolicyIfRevision {
                    expected_revision: 2,
                    policy: LayerPolicy::EffectiveOnly,
                },
                &snapshot,
            )
            .unwrap()
            .reply
            .state()
            .unwrap();
        assert_eq!(state.scene_policy, LayerPolicy::EffectiveOnly);

        let background = Rgb8::new(10, 10, 10);
        engine
            .render(
                RenderInput {
                    now_ms: 1,
                    snapshot: &snapshot,
                },
                &mut frame,
            )
            .unwrap();
        assert_eq!(
            frame.as_slice(),
            &[BLUE, background],
            "EffectiveOnly hides base-layer cells"
        );
    }

    #[test]
    fn scene_replace_is_chunked_ordered_atomic_and_idempotent() {
        let mut engine = scene_engine();
        let snapshot = context(0);
        set_cell(&mut engine, 0, scene_cell(3, 0, RED));

        let (id, _) = match engine
            .handle_command(
                10,
                StandardCommand::BeginSceneReplace {
                    expected_revision: 1,
                    cell_count: 2,
                },
                &snapshot,
            )
            .unwrap()
            .reply
        {
            StandardReply::SceneTransaction { id, cell_count } => (id, cell_count),
            other => panic!("expected transaction, got {other:?}"),
        };

        // A second begin while staged is busy; out-of-order chunks rejected.
        assert_eq!(
            engine.handle_command(
                11,
                StandardCommand::BeginSceneReplace {
                    expected_revision: 1,
                    cell_count: 0,
                },
                &snapshot,
            ),
            Err(StandardError::SceneTransactionBusy)
        );
        let mut chunk = SceneChunk::new();
        chunk.push(scene_cell(0, 0, GREEN)).unwrap();
        assert_eq!(
            engine.handle_command(
                12,
                StandardCommand::PutSceneChunk {
                    transaction_id: id,
                    offset: 1,
                    cells: chunk,
                },
                &snapshot,
            ),
            Err(StandardError::InvalidSceneRequest)
        );
        assert_eq!(
            engine.handle_command(
                13,
                StandardCommand::CommitSceneReplace { transaction_id: id },
                &snapshot,
            ),
            Err(StandardError::SceneTransactionIncomplete {
                expected: 2,
                received: 0
            })
        );

        let mut chunk = SceneChunk::new();
        chunk.push(scene_cell(0, 0, GREEN)).unwrap();
        chunk.push(scene_cell(0, 1, BLUE)).unwrap();
        engine
            .handle_command(
                14,
                StandardCommand::PutSceneChunk {
                    transaction_id: id,
                    offset: 0,
                    cells: chunk,
                },
                &snapshot,
            )
            .unwrap();

        let committed = engine
            .handle_command(
                15,
                StandardCommand::CommitSceneReplace { transaction_id: id },
                &snapshot,
            )
            .unwrap();
        assert_eq!(committed.invalidation, Invalidation::Render);
        let state = committed.reply.state().unwrap();
        assert_eq!(state.scene_len, 2, "replacement drops the previous table");
        assert_eq!(state.revision, 2);

        // Retried commit answers idempotently without another revision bump.
        let retried = engine
            .handle_command(
                16,
                StandardCommand::CommitSceneReplace { transaction_id: id },
                &snapshot,
            )
            .unwrap();
        assert_eq!(retried.invalidation, Invalidation::None);
        assert_eq!(retried.reply.state().unwrap().revision, 2);

        // A conflicting revision at commit leaves the table untouched.
        let stale = match engine
            .handle_command(
                20,
                StandardCommand::BeginSceneReplace {
                    expected_revision: 0,
                    cell_count: 0,
                },
                &snapshot,
            )
            .unwrap()
            .reply
        {
            StandardReply::SceneTransaction { id, .. } => id,
            other => panic!("expected transaction, got {other:?}"),
        };
        assert_eq!(
            engine.handle_command(
                21,
                StandardCommand::CommitSceneReplace { transaction_id: stale },
                &snapshot,
            ),
            Err(StandardError::RevisionConflict {
                expected: 0,
                current: 2
            })
        );
        assert_eq!(engine.state().scene_len, 2);
        engine
            .handle_command(
                22,
                StandardCommand::AbortSceneReplace { transaction_id: stale },
                &snapshot,
            )
            .unwrap();
    }

    #[test]
    fn scene_transaction_expires_after_inactivity() {
        let mut engine = scene_engine();
        let snapshot = context(0);
        let id = match engine
            .handle_command(
                0,
                StandardCommand::BeginSceneReplace {
                    expected_revision: 0,
                    cell_count: 0,
                },
                &snapshot,
            )
            .unwrap()
            .reply
        {
            StandardReply::SceneTransaction { id, .. } => id,
            other => panic!("expected transaction, got {other:?}"),
        };
        assert_eq!(
            engine.handle_command(
                SCENE_TRANSACTION_TIMEOUT_MS,
                StandardCommand::CommitSceneReplace { transaction_id: id },
                &snapshot,
            ),
            Err(StandardError::SceneTransactionExpired)
        );
        assert_eq!(
            engine.handle_command(
                SCENE_TRANSACTION_TIMEOUT_MS,
                StandardCommand::AbortSceneReplace { transaction_id: 999 },
                &snapshot,
            ),
            Err(StandardError::InvalidSceneTransaction)
        );
    }

    #[test]
    fn extension_source_claims_light_actions_before_background() {
        #[derive(Default)]
        struct ClaimingSource;

        impl<C, Context> LightingSource<C, Context> for ClaimingSource {
            fn len(&self, _: &SourceRenderInput<'_, Context>) -> usize {
                0
            }

            fn slot(&self, _: usize, _: &SourceRenderInput<'_, Context>) -> LedSlot {
                unreachable!("ClaimingSource has no targets")
            }

            fn contribution(&mut self, _: usize, _: &SourceRenderInput<'_, Context>) -> Contribution<C> {
                unreachable!("ClaimingSource has no samples")
            }

            fn handle_light_action(&mut self, action: LightAction) -> bool {
                matches!(action, LightAction::RgbHui)
            }
        }

        let mut engine: StandardLightingEngine<'static, ClaimingSource, EmptySource, 2, 2> =
            StandardLightingEngine::new(
                BackgroundState {
                    value: 10,
                    ..BackgroundState::default()
                },
                LayerScenes {
                    scenes: &LAYERS,
                    policy: LayerPolicy::ActiveStack,
                },
                ClaimingSource,
                EmptySource,
            );
        let snapshot = context(0);
        let hue_before = engine.state().background.hue;

        assert_eq!(
            engine.on_input(StandardInput(LightAction::RgbHui), &snapshot),
            Ok(Invalidation::Render)
        );
        assert_eq!(engine.state().revision, 1);
        assert_eq!(
            engine.state().background.hue,
            hue_before,
            "a claimed action must bypass the built-in background handling"
        );

        engine.on_input(StandardInput(LightAction::RgbHud), &snapshot).unwrap();
        assert_ne!(
            engine.state().background.hue,
            hue_before,
            "unclaimed actions still fall through to the background"
        );
    }
}
