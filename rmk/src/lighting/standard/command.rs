use core::cell::RefCell;
use core::fmt;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;

#[allow(unused_imports)]
use super::*;
use crate::RawMutex;
use crate::lighting::compositor::{
    ExtensionDescriptor, ExtensionLayerState, ExtensionParamSpec, ExtensionState, LightingSource, RenderError,
};
use crate::lighting::context::LightingContext;
use crate::lighting::source::{LayerPolicy, OutputMode, OverlayError};
use crate::lighting::topology::LedSlot;

/// One in-progress, chunk-staged atomic scene replacement.
///
/// The overlay replacement stages host-side because a whole overlay batch
/// fits one mailbox command. A scene table is an order of magnitude larger,
/// so its transaction stages inside the engine via bounded chunks instead of
/// forcing kilobyte-sized command payloads through every mailbox.
pub(super) struct SceneReplace<const CAP: usize> {
    pub(super) id: u32,
    pub(super) expected_revision: u32,
    pub(super) expected_count: u16,
    pub(super) cells: [SceneTableCell; CAP],
    pub(super) len: usize,
    pub(super) last_activity_ms: u64,
}

pub(super) struct RuntimeConditionalSceneReplace<const CAP: usize> {
    pub(super) id: u32,
    pub(super) expected_revision: u32,
    pub(super) expected_count: u16,
    pub(super) table: RuntimeConditionalSceneTable<CAP>,
    pub(super) last_activity_ms: u64,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StandardCommand<const OVERLAY_CAP: usize, const SCENE_CAP: usize = 0> {
    SetOutputEnabled(bool),
    SetOutputModeIfRevision {
        expected_revision: u32,
        mode: OutputMode,
    },
    /// Replace the wake-layer mask; see [`LightingControls::wake_layers`].
    SetWakeLayersIfRevision {
        expected_revision: u32,
        layers: u64,
    },
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
    ReadExtensionLayers,
    SetExtensionLayersIfRevision {
        expected_revision: u32,
        state: ExtensionLayerState,
    },
    /// One page of the addressed effect's parameter specs paired with the
    /// source's live values.
    ReadExtensionParams {
        effect: u8,
        offset: u8,
    },
    SetExtensionParamIfRevision {
        expected_revision: u32,
        effect: u8,
        index: u8,
        value: u8,
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
    ReadRuntimeConditionalScenes {
        offset: u16,
    },
    BeginRuntimeConditionalSceneReplace {
        expected_revision: u32,
        cell_count: u16,
    },
    PutRuntimeConditionalSceneChunk {
        transaction_id: u32,
        offset: u16,
        cells: RuntimeConditionalSceneChunk,
    },
    CommitRuntimeConditionalSceneReplace {
        transaction_id: u32,
    },
    AbortRuntimeConditionalSceneReplace {
        transaction_id: u32,
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
    pub runtime_conditional_scene_len: usize,
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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExtensionLayersPage {
    pub revision: u32,
    pub state: Option<ExtensionLayerState>,
}

/// Number of extension parameters carried by one [`ExtensionParamsPage`].
pub const EXTENSION_PARAM_CHUNK: usize = 8;

/// One extension parameter: its static spec plus the source's live value.
/// The value falls back to the spec's default when the source does not
/// report one.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExtensionParamValue {
    pub spec: ExtensionParamSpec,
    pub value: u8,
}

/// One page of an extension effect's parameters. Pinned to the engine
/// revision like every other mutable readback.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExtensionParamsPage {
    pub revision: u32,
    /// Total parameters the addressed effect advertises.
    pub total: u8,
    /// Rows starting at the request's offset; trailing entries are `None`.
    pub items: [Option<ExtensionParamValue>; EXTENSION_PARAM_CHUNK],
}

impl ExtensionParamsPage {
    /// The populated rows, in order.
    pub fn items(&self) -> impl Iterator<Item = ExtensionParamValue> + '_ {
        self.items.iter().copied().flatten()
    }
}

/// The active effect's live parameter values, carried in a replica snapshot
/// so a split renderer tracks the authority's tuning.
///
/// Only the active effect travels: it is the only one a replica renders, and
/// bounding the row at [`EXTENSION_PARAM_CHUNK`] keeps the snapshot `Copy`
/// with a fixed RAM cost. Parameters of inactive effects ride along when
/// that effect becomes active, since export always reads live values.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ExtensionReplicaParams {
    pub effect: u8,
    pub len: u8,
    pub values: [u8; EXTENSION_PARAM_CHUNK],
}

impl ExtensionReplicaParams {
    /// The populated values, in parameter order.
    pub fn values(&self) -> &[u8] {
        &self.values[..(self.len as usize).min(EXTENSION_PARAM_CHUNK)]
    }
}

/// The parameter specs an extension source advertises for `effect`.
///
/// A free function because the callers live in the engine's `LightingEngine`
/// impl, where `Context` is in scope but no inherent impl carries the
/// [`LightingSource`] bound.
pub(super) fn extension_param_specs<C, Context, S>(
    source: &S,
    effect: u8,
) -> Result<&'static [ExtensionParamSpec], StandardError>
where
    S: LightingSource<C, Context> + ?Sized,
{
    source
        .extension_descriptor()
        .and_then(|descriptor| descriptor.effect_params(effect))
        .ok_or(StandardError::ExtensionUnsupported)
}

/// Check one parameter's address and its spec's range without applying it.
///
/// Split from the apply so replica application can validate a whole batch
/// before mutating anything, the way the overlay is validated first.
pub(super) fn check_extension_param<C, Context, S>(
    source: &S,
    effect: u8,
    index: u8,
    value: u8,
) -> Result<(), StandardError>
where
    S: LightingSource<C, Context> + ?Sized,
{
    let specs = extension_param_specs::<C, Context, S>(source, effect)?;
    let spec = specs.get(index as usize).ok_or(StandardError::ExtensionUnsupported)?;
    if value < spec.min || value > spec.max {
        return Err(StandardError::ExtensionUnsupported);
    }
    Ok(())
}

/// Validate one parameter against its spec, then offer it to the source.
///
/// Shared by the host-facing set command and replica application so both
/// reject the same addresses and the same out-of-range values.
pub(super) fn apply_extension_param_checked<C, Context, S>(
    source: &mut S,
    effect: u8,
    index: u8,
    value: u8,
) -> Result<(), StandardError>
where
    S: LightingSource<C, Context> + ?Sized,
{
    check_extension_param::<C, Context, S>(source, effect, index, value)?;
    if !source.apply_extension_param(effect, index, value) {
        return Err(StandardError::ExtensionUnsupported);
    }
    Ok(())
}

/// One page of immutable board-compiled layer scenes.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CompiledScenePage {
    pub total: u16,
    pub policy: LayerPolicy,
    pub cells: SceneChunk,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RuntimeConditionalScenePage {
    pub revision: u32,
    pub total: u16,
    pub cells: RuntimeConditionalSceneChunk,
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
    ExtensionLayers(ExtensionLayersPage),
    ExtensionParams(ExtensionParamsPage),
    RuntimeConditionalScenesPage(RuntimeConditionalScenePage),
    RuntimeConditionalSceneTransaction { id: u32, cell_count: u16 },
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
    pub runtime_conditional_scenes: RuntimeConditionalSceneTable<SCENE_CAP>,
    pub context: LightingContext,
    pub sample_time_ms: u64,
    /// Extension-source selection, carried so split renderer replicas track
    /// the authority's animated band. `None` when the authority has no
    /// selectable extension.
    pub extension: Option<ExtensionState>,
    /// Optional second effect rendered over `extension`.
    pub extension_layers: Option<ExtensionLayerState>,
    /// The active effect's live parameter values. `None` when the authority
    /// has no selectable extension or the active effect has no parameters.
    pub extension_params: Option<ExtensionReplicaParams>,
    /// Overlay effect parameters when it has a distinct parameter row.
    pub extension_overlay_params: Option<ExtensionReplicaParams>,
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
    ConditionalSceneFull {
        capacity: usize,
    },
    InvalidConditionalSceneRequest,
    ConditionalSceneTransactionBusy,
    InvalidConditionalSceneTransaction,
    ConditionalSceneTransactionExpired,
    ConditionalSceneTransactionIncomplete {
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
