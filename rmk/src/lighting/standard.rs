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

use super::compositor::{
    Compositor, Contribution, LightingSource, LogicalFrame, RenderError, RenderInput as SourceRenderInput,
};
use super::context::LightingContextProvider;
use super::effect::{BuiltinEffect, LightingEffect};
use super::output::BrightnessTransform;
use super::service::{CommandResult, Invalidation, LightingEngine, RenderInput, RenderOutcome};
use super::source::{LayerScenes, OverlayError, OverlayUpdate, TtlOverlay};
use super::topology::LedSlot;
use super::{LightingContext, Rgb8};
use crate::RawMutex;

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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StandardCommand<const OVERLAY_CAP: usize> {
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
    ExportReplica(&'static StandardReplicaSlot<OVERLAY_CAP>),
    /// Atomically install a snapshot previously placed in the referenced
    /// slot. Intended for a renderer replica, not a second authority.
    ApplyReplica(&'static StandardReplicaSlot<OVERLAY_CAP>),
    ReadState,
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
    pub output_brightness: u8,
    pub background: BackgroundState,
    pub overlay_len: usize,
}

/// Complete declarative state needed by a standard-engine renderer replica.
///
/// `sample_time_ms` is the authority's animation clock at snapshot time.
/// Applying the snapshot anchors the replica's local monotonic clock to that
/// value; subsequent animation frames are sampled locally without link
/// traffic. Overlay TTLs are remaining lifetimes at the same instant.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct StandardReplicaState<const OVERLAY_CAP: usize> {
    pub revision: u32,
    pub mutable: StandardMutableState,
    pub overlay: OverlayBatch<OVERLAY_CAP>,
    pub context: LightingContext,
    pub sample_time_ms: u64,
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
pub struct StandardReplicaSlot<const OVERLAY_CAP: usize> {
    value: BlockingMutex<RawMutex, RefCell<Option<StandardReplicaState<OVERLAY_CAP>>>>,
}

impl<const OVERLAY_CAP: usize> StandardReplicaSlot<OVERLAY_CAP> {
    pub const fn new() -> Self {
        Self {
            value: BlockingMutex::new(RefCell::new(None)),
        }
    }

    pub fn put(&self, state: StandardReplicaState<OVERLAY_CAP>) -> Result<(), ReplicaSlotError> {
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

    pub fn take(&self) -> Result<StandardReplicaState<OVERLAY_CAP>, ReplicaSlotError> {
        self.value
            .lock(|value| value.borrow_mut().take().ok_or(ReplicaSlotError::Empty))
    }
}

impl<const OVERLAY_CAP: usize> Default for StandardReplicaSlot<OVERLAY_CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const OVERLAY_CAP: usize> fmt::Debug for StandardReplicaSlot<OVERLAY_CAP> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StandardReplicaSlot").finish_non_exhaustive()
    }
}

impl<const OVERLAY_CAP: usize> PartialEq for StandardReplicaSlot<OVERLAY_CAP> {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self, other)
    }
}

impl<const OVERLAY_CAP: usize> Eq for StandardReplicaSlot<OVERLAY_CAP> {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StandardError {
    Render(RenderError),
    Overlay(OverlayError),
    DeadlineOverflow,
    ReplicaSlot(ReplicaSlotError),
    RevisionConflict { expected: u32, current: u32 },
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
pub struct StandardLightingEngine<'scenes, Extension, Status, const N: usize, const OVERLAY_CAP: usize> {
    compositor: Compositor<Rgb8, N>,
    background: UniformBackground<N>,
    extension: Extension,
    layers: LayerScenes<'scenes, BuiltinEffect>,
    overlay: TtlOverlay<BuiltinEffect, OVERLAY_CAP>,
    status: Status,
    animation_clock: AnimationClock,
    revision: u32,
    output_enabled: bool,
    output_brightness: u8,
}

impl<'scenes, Extension, Status, const N: usize, const OVERLAY_CAP: usize>
    StandardLightingEngine<'scenes, Extension, Status, N, OVERLAY_CAP>
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
            overlay: TtlOverlay::new(),
            status,
            animation_clock: AnimationClock::local(),
            revision: 0,
            output_enabled: true,
            output_brightness: u8::MAX,
        }
    }

    pub fn state(&self) -> StandardState {
        StandardState {
            revision: self.revision,
            output_enabled: self.output_enabled,
            output_brightness: self.output_brightness,
            background: self.background.state(),
            overlay_len: self.overlay.active_len(),
        }
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
            LightAction::BacklightOn => self.output_enabled = true,
            LightAction::BacklightOff => self.output_enabled = false,
            LightAction::BacklightToggle => self.output_enabled = !self.output_enabled,
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
        self.output_enabled = state.output_enabled;
        self.output_brightness = state.output_brightness;
        self.background.set_state(state.background);
    }

    fn mutable_state(&self) -> StandardMutableState {
        StandardMutableState {
            output_enabled: self.output_enabled,
            output_brightness: self.output_brightness,
            background: self.background.state(),
        }
    }

    fn replica_state<Context: LightingContextProvider>(
        &self,
        local_now_ms: u64,
        context: &Context,
    ) -> Result<StandardReplicaState<OVERLAY_CAP>, StandardError> {
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
            overlay,
            context: *context.lighting_context(),
            sample_time_ms,
        })
    }

    fn apply_replica(
        &mut self,
        local_now_ms: u64,
        replica: StandardReplicaState<OVERLAY_CAP>,
    ) -> Result<(), StandardError> {
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
        self.overlay
            .replace(replica.sample_time_ms, &updates[..replica.overlay.as_slice().len()])?;
        self.set_mutable_state(replica.mutable);
        self.revision = replica.revision;
        self.animation_clock.anchor(local_now_ms, replica.sample_time_ms);
        Ok(())
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
}

impl<'scenes, Context, Extension, Status, const N: usize, const OVERLAY_CAP: usize> LightingEngine<Context>
    for StandardLightingEngine<'scenes, Extension, Status, N, OVERLAY_CAP>
where
    Context: LightingContextProvider,
    Extension: LightingSource<Rgb8, Context>,
    Status: LightingSource<Rgb8, Context>,
{
    type Frame = LogicalFrame<Rgb8, N>;
    type Input = StandardInput;
    type Command = StandardCommand<OVERLAY_CAP>;
    type Reply = StandardState;
    type Error = StandardError;

    fn on_input(&mut self, input: Self::Input, _snapshot: &Context) -> Result<Invalidation, Self::Error> {
        if self.apply_light_action(input.0) {
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
        let (mut invalidation, advances_revision) = match command {
            StandardCommand::SetOutputEnabled(enabled) => {
                self.output_enabled = enabled;
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
                slot.put(self.replica_state(now_ms, snapshot)?)?;
                (Invalidation::None, false)
            }
            StandardCommand::ApplyReplica(slot) => {
                self.apply_replica(now_ms, slot.take()?)?;
                (Invalidation::Render, false)
            }
            StandardCommand::ReadState => (Invalidation::None, false),
        };
        if advances_revision {
            self.advance_revision();
            if invalidation == Invalidation::None {
                invalidation = Invalidation::StateChanged;
            }
        }
        Ok(CommandResult::new(self.state(), invalidation))
    }

    fn render(
        &mut self,
        input: RenderInput<'_, Context>,
        frame: &mut Self::Frame,
    ) -> Result<RenderOutcome, Self::Error> {
        let effect_now_ms = self.animation_clock.sample_time(input.now_ms);
        let overlay_len = self.overlay.active_len();
        self.overlay.prune_expired(effect_now_ms);
        let state_changed = self.overlay.active_len() != overlay_len;
        if state_changed {
            self.advance_revision();
        }
        let Self {
            compositor,
            background,
            extension,
            layers,
            overlay,
            status,
            animation_clock: _,
            output_enabled,
            output_brightness,
            revision: _,
        } = self;
        let mut transaction = compositor.begin(effect_now_ms, input.snapshot, Rgb8::BLACK, frame);
        transaction.apply(priority::BACKGROUND, background)?;
        transaction.apply(priority::EXTENSION, extension)?;
        transaction.apply(priority::LAYER, layers)?;
        transaction.apply(priority::HOST_OVERLAY, overlay)?;
        transaction.apply(priority::STATUS, status)?;
        let mut transform = BrightnessTransform::new(if *output_enabled { *output_brightness } else { 0 });
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

    fn context(layer: u8) -> LightingContext {
        LightingContext {
            layers: LayerState::new(layer, 0, 1 | (1 << layer)),
            indicators: Default::default(),
        }
    }

    #[test]
    fn replica_snapshot_preserves_state_context_ttl_and_animation_phase() {
        let mut authority = engine();
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

        static EXPORT: StandardReplicaSlot<2> = StandardReplicaSlot::new();
        authority
            .handle_command(120, StandardCommand::ExportReplica(&EXPORT), &authority_context)
            .unwrap();
        let snapshot = EXPORT.take().unwrap();
        assert_eq!(snapshot.context, authority_context);
        assert_eq!(snapshot.sample_time_ms, 120);
        assert_eq!(snapshot.overlay.as_slice()[0].ttl_ms, NonZeroU32::new(80));

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

        static APPLY: StandardReplicaSlot<2> = StandardReplicaSlot::new();
        APPLY.put(snapshot).unwrap();
        let mut replica = engine();
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
            overlay: OverlayBatch::new(),
            context: context(0),
            sample_time_ms: 9,
        };
        slot.put(snapshot).unwrap();
        assert_eq!(slot.put(snapshot), Err(ReplicaSlotError::Busy));
        assert_eq!(slot.take(), Ok(snapshot));
        assert_eq!(slot.take(), Err(ReplicaSlotError::Empty));
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
            .reply;

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
            .reply;
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
}
