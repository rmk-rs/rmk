use core::num::NonZeroU32;

use rmk_types::action::LightAction;

#[allow(unused_imports)]
use super::*;
use crate::lighting::Rgb8;
use crate::lighting::compositor::{Compositor, LightingSource, LogicalFrame};
use crate::lighting::context::LightingContextProvider;
use crate::lighting::effect::BuiltinEffect;
use crate::lighting::output::BrightnessTransform;
use crate::lighting::service::{CommandResult, Invalidation, LightingEngine, RenderInput, RenderOutcome};
use crate::lighting::source::{
    BatteryStatusProvider, LayerPolicy, LayerScenes, LightingControls, OutputMode, OverlayUpdate, SceneCell,
    SparseScene, TtlOverlay,
};
use crate::lighting::topology::LedSlot;

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
    runtime_conditional_scenes: RuntimeConditionalSceneTable<SCENE_CAP>,
    battery_status: &'scenes dyn BatteryStatusProvider,
    overlay: TtlOverlay<BuiltinEffect, OVERLAY_CAP>,
    status: Status,
    animation_clock: AnimationClock,
    scene_replace: Option<SceneReplace<SCENE_CAP>>,
    scene_next_transaction: u32,
    scene_expired_transaction: Option<u32>,
    scene_committed_transaction: Option<u32>,
    runtime_conditional_scene_replace: Option<RuntimeConditionalSceneReplace<SCENE_CAP>>,
    runtime_conditional_scene_next_transaction: u32,
    runtime_conditional_scene_expired_transaction: Option<u32>,
    runtime_conditional_scene_committed_transaction: Option<u32>,
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
            runtime_conditional_scenes: RuntimeConditionalSceneTable::new(),
            battery_status: &NO_BATTERY_STATUS,
            overlay: TtlOverlay::new(),
            status,
            animation_clock: AnimationClock::local(),
            scene_replace: None,
            scene_next_transaction: 1,
            scene_expired_transaction: None,
            scene_committed_transaction: None,
            runtime_conditional_scene_replace: None,
            runtime_conditional_scene_next_transaction: 1,
            runtime_conditional_scene_expired_transaction: None,
            runtime_conditional_scene_committed_transaction: None,
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

    /// Supply live board battery state for runtime conditional predicates.
    pub const fn with_battery_status_provider(mut self, battery_status: &'scenes dyn BatteryStatusProvider) -> Self {
        self.battery_status = battery_status;
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
            runtime_conditional_scene_len: self.runtime_conditional_scenes.len(),
        }
    }

    pub const fn scene_capacity() -> usize {
        SCENE_CAP
    }

    pub const fn scenes(&self) -> &SceneTable<SCENE_CAP> {
        &self.scenes
    }

    pub const fn runtime_conditional_scenes(&self) -> &RuntimeConditionalSceneTable<SCENE_CAP> {
        &self.runtime_conditional_scenes
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

    /// Install one persisted runtime conditional rule during startup.
    pub fn install_runtime_conditional_scene_cell(
        &mut self,
        cell: RuntimeConditionalSceneCell,
    ) -> Result<(), StandardError> {
        Self::check_scene_slot(cell.slot)?;
        self.runtime_conditional_scenes.push(cell)
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
            runtime_conditional_scenes: self.runtime_conditional_scenes,
            context: *context.lighting_context(),
            sample_time_ms,
            // Filled by handle_command, where the LightingSource bound is
            // available on the Extension parameter.
            extension: None,
            extension_params: None,
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
        self.runtime_conditional_scenes = replica.runtime_conditional_scenes;
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

    fn expire_runtime_conditional_scene_replace(&mut self, now_ms: u64) {
        if self
            .runtime_conditional_scene_replace
            .as_ref()
            .is_some_and(|replace| now_ms.saturating_sub(replace.last_activity_ms) >= SCENE_TRANSACTION_TIMEOUT_MS)
        {
            self.runtime_conditional_scene_expired_transaction = self
                .runtime_conditional_scene_replace
                .as_ref()
                .map(|replace| replace.id);
            self.runtime_conditional_scene_replace = None;
        }
    }

    fn runtime_conditional_scene_transaction_error(&self, id: u32) -> StandardError {
        if self.runtime_conditional_scene_expired_transaction == Some(id) {
            StandardError::ConditionalSceneTransactionExpired
        } else {
            StandardError::InvalidConditionalSceneTransaction
        }
    }

    fn begin_runtime_conditional_scene_replace(
        &mut self,
        now_ms: u64,
        expected_revision: u32,
        cell_count: u16,
    ) -> Result<u32, StandardError> {
        self.expire_runtime_conditional_scene_replace(now_ms);
        if self.runtime_conditional_scene_replace.is_some() {
            return Err(StandardError::ConditionalSceneTransactionBusy);
        }
        if cell_count as usize > SCENE_CAP {
            return Err(StandardError::ConditionalSceneFull { capacity: SCENE_CAP });
        }
        let id = self.runtime_conditional_scene_next_transaction;
        self.runtime_conditional_scene_next_transaction =
            self.runtime_conditional_scene_next_transaction.wrapping_add(1).max(1);
        self.runtime_conditional_scene_expired_transaction = None;
        self.runtime_conditional_scene_replace = Some(RuntimeConditionalSceneReplace {
            id,
            expected_revision,
            expected_count: cell_count,
            cells: [EMPTY_CONDITIONAL_SCENE_CELL; SCENE_CAP],
            len: 0,
            last_activity_ms: now_ms,
        });
        Ok(id)
    }

    fn put_runtime_conditional_scene_chunk(
        &mut self,
        now_ms: u64,
        transaction_id: u32,
        offset: u16,
        cells: &RuntimeConditionalSceneChunk,
    ) -> Result<(), StandardError> {
        self.expire_runtime_conditional_scene_replace(now_ms);
        for cell in cells.as_slice() {
            Self::check_scene_slot(cell.slot)?;
        }
        let error = self.runtime_conditional_scene_transaction_error(transaction_id);
        let replace = self
            .runtime_conditional_scene_replace
            .as_mut()
            .filter(|replace| replace.id == transaction_id)
            .ok_or(error)?;
        if offset as usize != replace.len || replace.len + cells.as_slice().len() > replace.expected_count as usize {
            return Err(StandardError::InvalidConditionalSceneRequest);
        }
        for cell in cells.as_slice() {
            replace.cells[replace.len] = *cell;
            replace.len += 1;
        }
        replace.last_activity_ms = now_ms;
        Ok(())
    }

    fn commit_runtime_conditional_scene_replace(
        &mut self,
        now_ms: u64,
        transaction_id: u32,
    ) -> Result<bool, StandardError> {
        self.expire_runtime_conditional_scene_replace(now_ms);
        if self.runtime_conditional_scene_committed_transaction == Some(transaction_id)
            && self.runtime_conditional_scene_replace.is_none()
        {
            return Ok(false);
        }
        let error = self.runtime_conditional_scene_transaction_error(transaction_id);
        let replace = self
            .runtime_conditional_scene_replace
            .as_ref()
            .filter(|replace| replace.id == transaction_id)
            .ok_or(error)?;
        if replace.len != replace.expected_count as usize {
            return Err(StandardError::ConditionalSceneTransactionIncomplete {
                expected: replace.expected_count,
                received: replace.len as u16,
            });
        }
        self.check_revision(replace.expected_revision)?;
        let replace = self.runtime_conditional_scene_replace.take().expect("checked above");
        self.runtime_conditional_scenes.clear();
        for cell in &replace.cells[..replace.len] {
            self.runtime_conditional_scenes
                .push(*cell)
                .expect("staged length is table-bounded");
        }
        self.runtime_conditional_scene_committed_transaction = Some(transaction_id);
        self.runtime_conditional_scene_expired_transaction = None;
        Ok(true)
    }

    fn abort_runtime_conditional_scene_replace(
        &mut self,
        now_ms: u64,
        transaction_id: u32,
    ) -> Result<(), StandardError> {
        self.expire_runtime_conditional_scene_replace(now_ms);
        if self
            .runtime_conditional_scene_replace
            .as_ref()
            .is_some_and(|replace| replace.id == transaction_id)
        {
            self.runtime_conditional_scene_replace = None;
            return Ok(());
        }
        Err(self.runtime_conditional_scene_transaction_error(transaction_id))
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
            StandardCommand::ReadRuntimeConditionalScenes { offset } => {
                return Ok(CommandResult::unchanged(StandardReply::RuntimeConditionalScenesPage(
                    RuntimeConditionalScenePage {
                        revision: self.revision,
                        total: self.runtime_conditional_scenes.len().min(u16::MAX as usize) as u16,
                        cells: self.runtime_conditional_scenes.page(offset),
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
            StandardCommand::BeginRuntimeConditionalSceneReplace {
                expected_revision,
                cell_count,
            } => {
                let id = self.begin_runtime_conditional_scene_replace(now_ms, expected_revision, cell_count)?;
                return Ok(CommandResult::unchanged(
                    StandardReply::RuntimeConditionalSceneTransaction { id, cell_count },
                ));
            }
            StandardCommand::PutRuntimeConditionalSceneChunk {
                transaction_id,
                offset,
                cells,
            } => {
                self.put_runtime_conditional_scene_chunk(now_ms, transaction_id, offset, &cells)?;
                return Ok(CommandResult::unchanged(StandardReply::State(self.state())));
            }
            StandardCommand::CommitRuntimeConditionalSceneReplace { transaction_id } => {
                let committed = self.commit_runtime_conditional_scene_replace(now_ms, transaction_id)?;
                if committed {
                    self.advance_revision();
                    return Ok(CommandResult::new(
                        StandardReply::State(self.state()),
                        Invalidation::Render,
                    ));
                }
                return Ok(CommandResult::unchanged(StandardReply::State(self.state())));
            }
            StandardCommand::AbortRuntimeConditionalSceneReplace { transaction_id } => {
                self.abort_runtime_conditional_scene_replace(now_ms, transaction_id)?;
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
            StandardCommand::SetOutputModeIfRevision {
                expected_revision,
                mode,
            } => {
                self.check_revision(expected_revision)?;
                self.output_mode = mode;
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
                replica.extension_params = replica.extension.and_then(|state| {
                    // A source with no descriptor, an active effect the
                    // descriptor does not know, or an effect without
                    // parameters all export nothing to replicate.
                    let specs = extension_param_specs::<Rgb8, Context, _>(&self.extension, state.effect).ok()?;
                    let len = specs.len().min(EXTENSION_PARAM_CHUNK);
                    if len == 0 {
                        return None;
                    }
                    let mut values = [0u8; EXTENSION_PARAM_CHUNK];
                    for (value, (index, spec)) in values.iter_mut().zip(specs[..len].iter().enumerate()) {
                        *value = self
                            .extension
                            .extension_param(state.effect, index as u8)
                            .unwrap_or(spec.default);
                    }
                    Some(ExtensionReplicaParams {
                        effect: state.effect,
                        len: len as u8,
                        values,
                    })
                });
                slot.put(replica)?;
                (Invalidation::None, false)
            }
            StandardCommand::ApplyReplica(slot) => {
                let replica = slot.take()?;
                let overlay = Self::replica_overlay(&replica)?;
                // Parameter addresses and ranges are checked against the
                // static descriptor before anything is applied, so a
                // malformed snapshot cannot leave the selection updated and
                // the tuning behind -- the same guarantee the overlay gets.
                if let Some(params) = replica.extension_params {
                    for (index, value) in params.values().iter().copied().enumerate() {
                        check_extension_param::<Rgb8, Context, _>(&self.extension, params.effect, index as u8, value)?;
                    }
                }
                match (replica.extension, self.extension.extension_state()) {
                    (None, None) => {}
                    (Some(extension), Some(_)) if self.extension.apply_extension_state(extension) => {}
                    _ => return Err(StandardError::ExtensionUnsupported),
                }
                // Parameters follow the selection so the addressed effect is
                // already active. Values go through the same validated path a
                // host set uses; only the revision pin is skipped, matching
                // how replica extension state is applied.
                if let Some(params) = replica.extension_params {
                    for (index, value) in params.values().iter().copied().enumerate() {
                        apply_extension_param_checked::<Rgb8, Context, _>(
                            &mut self.extension,
                            params.effect,
                            index as u8,
                            value,
                        )?;
                    }
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
            StandardCommand::ReadExtensionParams { effect, offset } => {
                let specs = extension_param_specs::<Rgb8, Context, _>(&self.extension, effect)?;
                let start = (offset as usize).min(specs.len());
                let end = (start + EXTENSION_PARAM_CHUNK).min(specs.len());
                let mut items = [None; EXTENSION_PARAM_CHUNK];
                for (item, (index, spec)) in items.iter_mut().zip((start..end).zip(&specs[start..end])) {
                    let value = self
                        .extension
                        .extension_param(effect, index as u8)
                        .unwrap_or(spec.default);
                    *item = Some(ExtensionParamValue { spec: *spec, value });
                }
                return Ok(CommandResult::unchanged(StandardReply::ExtensionParams(
                    ExtensionParamsPage {
                        revision: self.revision,
                        // The wire caps one effect's parameter list at 255.
                        total: specs.len().min(u8::MAX as usize) as u8,
                        items,
                    },
                )));
            }
            StandardCommand::SetExtensionParamIfRevision {
                expected_revision,
                effect,
                index,
                value,
            } => {
                self.check_revision(expected_revision)?;
                apply_extension_param_checked::<Rgb8, Context, _>(&mut self.extension, effect, index, value)?;
                (Invalidation::Render, true)
            }
            StandardCommand::ReadState => (Invalidation::None, false),
            StandardCommand::ReadOverlay { .. }
            | StandardCommand::ReadScenes { .. }
            | StandardCommand::ReadCompiledScenes { .. }
            | StandardCommand::ReadRuntimeConditionalScenes { .. }
            | StandardCommand::BeginSceneReplace { .. }
            | StandardCommand::PutSceneChunk { .. }
            | StandardCommand::CommitSceneReplace { .. }
            | StandardCommand::AbortSceneReplace { .. } => unreachable!("handled above"),
            StandardCommand::BeginRuntimeConditionalSceneReplace { .. }
            | StandardCommand::PutRuntimeConditionalSceneChunk { .. }
            | StandardCommand::CommitRuntimeConditionalSceneReplace { .. }
            | StandardCommand::AbortRuntimeConditionalSceneReplace { .. } => unreachable!("handled above"),
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
            runtime_conditional_scenes,
            battery_status,
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
            runtime_conditional_scene_replace: _,
            runtime_conditional_scene_next_transaction: _,
            runtime_conditional_scene_expired_transaction: _,
            runtime_conditional_scene_committed_transaction: _,
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
        // Status band, after the compiled rules: a runtime conditional cell is
        // the host's replacement for a compiled one, so it must reach the same
        // altitude and win on the slots the two share.
        let mut runtime_conditional_source = RuntimeConditionalSource {
            table: runtime_conditional_scenes,
            batteries: *battery_status,
        };
        transaction.apply(priority::STATUS, &mut runtime_conditional_source)?;
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

pub(super) fn hsv(hue: u8, saturation: u8, value: u8) -> Rgb8 {
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
