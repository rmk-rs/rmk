//! Type-erased Rynk lighting binding and standard-engine mailbox bridge.
//!
//! `RynkService` deliberately stays non-generic so existing USB/BLE/UART
//! orchestration does not acquire lighting-engine type parameters. A small
//! protocol mailbox erases those parameters; [`StandardRynkLightingAdapter`]
//! is the only bridge from that mailbox to the standard engine's authoritative
//! mailbox.

use core::cell::{Cell, RefCell};
use core::num::NonZeroU32;

use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::channel::Channel;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use heapless::{String, Vec};
use rmk_types::protocol::rynk::{
    LIGHTING_EXTENSION_NAME_CHUNK, LIGHTING_EXTENSION_NAME_SIZE, LIGHTING_OVERLAY_CHUNK_SIZE,
    LIGHTING_SCENE_CHUNK_SIZE, LightingBackgroundMode, LightingBackgroundState, LightingCompiledScenesPage,
    LightingConditionalSceneCell as WireConditionalSceneCell, LightingControls as WireLightingControls, LightingError,
    LightingExtension, LightingExtensionNameKind, LightingExtensionNamesPage,
    LightingExtensionState as WireExtensionState, LightingLayerPolicy, LightingMutableState,
    LightingOutputMode as WireLightingOutputMode, LightingOutputModeIndicator as WireLightingOutputModeIndicator,
    LightingOutputModeState, LightingOverlayCell, LightingOverlayPage, LightingResult, LightingRgb8, LightingSceneCell,
    LightingSceneTransaction, LightingScenesPage, LightingState,
};

use crate::RawMutex;
use crate::core_traits::Runnable;
use crate::lighting::{
    BackgroundMode, BackgroundState, BuiltinEffect, ConditionalSceneCell, LayerPolicy, LedId, LightingControls,
    LightingMailbox, LightingRouting, LightingTopology, OVERLAY_CHUNK_SIZE, OverlayBatch, OverlayCell, OverlayError,
    Rgb8, SceneChunk, SceneTableCell, StandardCommand, StandardError, StandardLightingEngine, StandardMutableState,
    StandardReply, StandardState,
};

const _: () = core::assert!(
    crate::lighting::SCENE_CHUNK_SIZE == LIGHTING_SCENE_CHUNK_SIZE,
    "engine scene chunk must match the wire chunk so adapters forward chunks unmodified"
);
const _: () = core::assert!(
    OVERLAY_CHUNK_SIZE == LIGHTING_OVERLAY_CHUNK_SIZE,
    "engine overlay page must match the wire chunk so adapters forward pages unmodified"
);

/// Maximum number of cells staged by one Rynk overlay replacement.
///
/// A concrete engine may advertise a smaller capacity. Keeping the protocol
/// staging bound independent of the engine's const generic avoids making the
/// host service and every transport generic.
pub const RYNK_LIGHTING_TRANSACTION_CAPACITY: usize = 64;

const RYNK_LIGHTING_COMMAND_CAPACITY: usize = 4;

#[derive(Clone, Copy)]
pub struct RynkLightingDescriptor<'a> {
    pub topology_revision: u32,
    pub topology: LightingTopology<'a>,
    pub routing: LightingRouting<'a>,
}

/// Concrete, type-erased lighting attachment carried by [`super::RynkService`].
#[derive(Clone, Copy)]
pub struct RynkLightingController<'a> {
    pub(super) descriptor: RynkLightingDescriptor<'a>,
    pub(super) overlay_capacity: u16,
    /// Advertised runtime scene-cell capacity. `0` means the board did not
    /// wire a scene table; hosts gate on it and every scene endpoint rejects
    /// with `Unsupported`.
    pub(super) scene_capacity: u16,
    pub(super) conditional_scenes: &'a [ConditionalSceneCell<BuiltinEffect>],
    pub(super) controls: LightingControls,
    /// Whether the board wired a host-selectable extension source; gates the
    /// `EXTENSION_EFFECTS` capability bit and the extension endpoints.
    pub(super) extension_effects: bool,
    mailbox: &'a RynkLightingMailbox,
}

impl<'a> RynkLightingController<'a> {
    pub const fn new(
        mailbox: &'a RynkLightingMailbox,
        descriptor: RynkLightingDescriptor<'a>,
        overlay_capacity: u16,
    ) -> Self {
        let staged_capacity = RYNK_LIGHTING_TRANSACTION_CAPACITY as u16;
        Self {
            descriptor,
            // The single advertised limit must be valid for both incremental
            // updates and atomic replacement.
            overlay_capacity: if overlay_capacity < staged_capacity {
                overlay_capacity
            } else {
                staged_capacity
            },
            scene_capacity: 0,
            conditional_scenes: &[],
            controls: LightingControls {
                output_toggle_user_action: None,
                output_mode_cycle_user_action: None,
                wake_layer: None,
                initial_output_mode: crate::lighting::OutputMode::AlwaysOn,
                powered_only_scope: crate::lighting::PoweredOnlyScope::Authority,
                output_mode_indicator: None,
            },
            extension_effects: false,
            mailbox,
        }
    }

    /// Advertise the engine's host-selectable animated extension source.
    /// Boards whose extension band is not user-selectable skip this call.
    pub const fn with_extension_effects(mut self) -> Self {
        self.extension_effects = true;
        self
    }

    /// Advertise runtime scene support. Pass the engine's scene capacity;
    /// boards without a scene table simply skip this call.
    pub const fn with_scene_capacity(mut self, scene_capacity: u16) -> Self {
        self.scene_capacity = scene_capacity;
        self
    }

    /// Advertise immutable conditional scenes compiled from board config.
    pub const fn with_conditional_scenes(mut self, scenes: &'a [ConditionalSceneCell<BuiltinEffect>]) -> Self {
        self.conditional_scenes = scenes;
        self
    }

    pub const fn with_controls(mut self, controls: LightingControls) -> Self {
        self.controls = controls;
        self
    }

    pub(super) const fn controls_to_wire(&self) -> WireLightingControls {
        WireLightingControls {
            output_toggle_user_action: self.controls.output_toggle_user_action,
            wake_layer: self.controls.wake_layer,
        }
    }

    pub(super) fn output_mode_to_wire(&self, state: StandardState) -> LightingOutputModeState {
        LightingOutputModeState {
            mode: match state.output_mode {
                crate::lighting::OutputMode::AlwaysOn => WireLightingOutputMode::AlwaysOn,
                crate::lighting::OutputMode::AlwaysOff => WireLightingOutputMode::AlwaysOff,
                crate::lighting::OutputMode::PoweredOnly => WireLightingOutputMode::PoweredOnly,
            },
            powered: state.powered,
            wake_active: state.wake_active,
            effective_enabled: state.output_enabled,
            powered_only_scope: match self.controls.powered_only_scope {
                crate::lighting::PoweredOnlyScope::Authority => {
                    rmk_types::protocol::rynk::LightingPoweredOnlyScope::Authority
                }
                crate::lighting::PoweredOnlyScope::Local => rmk_types::protocol::rynk::LightingPoweredOnlyScope::Local,
            },
            cycle_user_action: self.controls.output_mode_cycle_user_action,
            wake_layer: self.controls.wake_layer,
            indicator: self.controls.output_mode_indicator.and_then(|indicator| {
                self.descriptor
                    .topology
                    .led(indicator.slot)
                    .map(|led| WireLightingOutputModeIndicator {
                        led_id: rmk_types::protocol::rynk::LightingLedId(led.id.0),
                        always_on: effect_to_wire(indicator.always_on),
                        always_off: effect_to_wire(indicator.always_off),
                        powered_only: effect_to_wire(indicator.powered_only),
                    })
            }),
        }
    }

    pub const fn descriptor(&self) -> RynkLightingDescriptor<'a> {
        self.descriptor
    }

    pub const fn overlay_capacity(&self) -> u16 {
        self.overlay_capacity
    }

    pub const fn scene_capacity(&self) -> u16 {
        self.scene_capacity
    }

    pub(super) fn conditional_scene_cell_to_wire(
        &self,
        cell: ConditionalSceneCell<BuiltinEffect>,
    ) -> Option<WireConditionalSceneCell> {
        let led = self.descriptor.topology.led(cell.slot)?;
        Some(WireConditionalSceneCell {
            conditions: condition_set_to_wire(cell.conditions),
            led_id: rmk_types::protocol::rynk::LightingLedId(led.id.0),
            effect: effect_to_wire(cell.effect),
        })
    }

    pub(super) async fn request(&self, command: RynkLightingCommand) -> LightingResult<RynkLightingReadback> {
        self.mailbox.request(command).await
    }

    /// Request expecting authoritative state readback.
    pub(super) async fn request_state(&self, command: RynkLightingCommand) -> LightingResult<LightingState> {
        expect_state(self.mailbox.request(command).await)
    }

    pub(super) async fn replace_overlay(
        &self,
        expected_revision: u32,
        cells: &Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY>,
    ) -> LightingResult<LightingState> {
        expect_state(self.mailbox.request_replace(expected_revision, cells).await)
    }
}

/// Typed readback carried by the protocol mailbox. Most commands answer with
/// wire state; scene reads and transaction reservation have their own shapes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RynkLightingReadback {
    State(LightingState),
    OutputMode(StandardState),
    OverlayPage(LightingOverlayPage),
    SceneStatus {
        revision: u32,
        scene_len: u16,
        policy: LightingLayerPolicy,
    },
    ScenesPage(LightingScenesPage),
    CompiledSceneStatus {
        scene_len: u16,
        policy: LightingLayerPolicy,
    },
    CompiledScenesPage(LightingCompiledScenesPage),
    SceneTransaction(LightingSceneTransaction),
    Extension(LightingExtension),
    ExtensionNamesPage(LightingExtensionNamesPage),
    Unit,
}

fn expect_state(result: LightingResult<RynkLightingReadback>) -> LightingResult<LightingState> {
    match result? {
        RynkLightingReadback::State(state) => Ok(state),
        _ => {
            debug_assert!(false, "adapter answered a state command with a non-state readback");
            Err(LightingError::InvalidRequest)
        }
    }
}

/// Fixed protocol-facing mailbox. Its adapter forwards every request to the
/// standard engine mailbox; it never owns renderer or compositor state.
pub struct RynkLightingMailbox {
    requests: Channel<RawMutex, MailboxRequest, RYNK_LIGHTING_COMMAND_CAPACITY>,
    response: Signal<RawMutex, MailboxResponse>,
    caller: Mutex<RawMutex, ()>,
    next_id: BlockingMutex<RawMutex, Cell<u32>>,
    replacement: BlockingMutex<RawMutex, RefCell<Option<StagedReplacement>>>,
    replacement_ready: Signal<RawMutex, ()>,
    replacement_available: Signal<RawMutex, ()>,
}

struct StagedReplacement {
    id: u32,
    cells: Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY>,
}

pub(in crate::host::rynk) struct MailboxRequest {
    pub(in crate::host::rynk) id: u32,
    pub(in crate::host::rynk) command: RynkLightingCommand,
}

struct MailboxResponse {
    id: u32,
    result: LightingResult<RynkLightingReadback>,
}

impl RynkLightingMailbox {
    pub const fn new() -> Self {
        Self {
            requests: Channel::new(),
            response: Signal::new(),
            caller: Mutex::new(()),
            next_id: BlockingMutex::new(Cell::new(0)),
            replacement: BlockingMutex::new(RefCell::new(None)),
            replacement_ready: Signal::new(),
            replacement_available: Signal::new(),
        }
    }

    async fn request(&self, command: RynkLightingCommand) -> LightingResult<RynkLightingReadback> {
        let _caller = self.caller.lock().await;
        let id = self.allocate_id();
        self.send_and_wait(id, command).await
    }

    async fn request_replace(
        &self,
        expected_revision: u32,
        cells: &Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY>,
    ) -> LightingResult<RynkLightingReadback> {
        let _caller = self.caller.lock().await;
        while self.replacement.lock(|replacement| replacement.borrow().is_some()) {
            self.replacement_available.wait().await;
        }

        let id = self.allocate_id();
        // Enqueue first. After this await, staging and its ready signal are
        // synchronous, so cancellation cannot leave an orphaned stage without
        // a matching command. The adapter may receive the token first and wait.
        self.requests
            .send(MailboxRequest {
                id,
                command: RynkLightingCommand::ReplaceOverlay { expected_revision },
            })
            .await;
        self.replacement.lock(|replacement| {
            let previous = replacement.borrow_mut().replace(StagedReplacement {
                id,
                cells: cells.clone(),
            });
            debug_assert!(previous.is_none(), "replacement slot was checked before enqueue");
        });
        self.replacement_ready.signal(());
        self.wait_for_reply(id).await
    }

    fn allocate_id(&self) -> u32 {
        self.next_id.lock(|next| {
            let id = next.get();
            next.set(id.wrapping_add(1));
            id
        })
    }

    async fn send_and_wait(&self, id: u32, command: RynkLightingCommand) -> LightingResult<RynkLightingReadback> {
        self.requests.send(MailboxRequest { id, command }).await;
        self.wait_for_reply(id).await
    }

    async fn wait_for_reply(&self, id: u32) -> LightingResult<RynkLightingReadback> {
        loop {
            let response = self.response.wait().await;
            if response.id == id {
                return response.result;
            }
        }
    }

    pub(in crate::host::rynk) async fn receive(&self) -> MailboxRequest {
        self.requests.receive().await
    }

    pub(in crate::host::rynk) fn reply(&self, id: u32, result: LightingResult<RynkLightingReadback>) {
        self.response.signal(MailboxResponse { id, result });
    }

    pub(in crate::host::rynk) async fn take_replacement(
        &self,
        id: u32,
    ) -> Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY> {
        loop {
            let staged = self.replacement.lock(|replacement| {
                let mut replacement = replacement.borrow_mut();
                if replacement.as_ref().is_some_and(|staged| staged.id == id) {
                    replacement.take()
                } else {
                    None
                }
            });
            if let Some(staged) = staged {
                self.replacement_available.signal(());
                return staged.cells;
            }
            self.replacement_ready.wait().await;
        }
    }
}

impl Default for RynkLightingMailbox {
    fn default() -> Self {
        Self::new()
    }
}

pub(super) enum RynkLightingCommand {
    ReadState,
    ReadOutputMode,
    ReadOverlay {
        expected_revision: u32,
        offset: u16,
    },
    ReadSceneStatus,
    ReadScenes {
        expected_revision: u32,
        offset: u16,
    },
    ReadCompiledSceneStatus,
    ReadCompiledScenes {
        offset: u16,
    },
    ReadExtension,
    ReadExtensionNames {
        kind: LightingExtensionNameKind,
        offset: u8,
    },
    SetExtensionState {
        expected_revision: u32,
        state: WireExtensionState,
    },
    SetSceneCell {
        expected_revision: u32,
        cell: LightingSceneCell,
    },
    UnsetSceneCell {
        expected_revision: u32,
        layer: u8,
        led_id: rmk_types::protocol::rynk::LightingLedId,
    },
    SetLayerPolicy {
        expected_revision: u32,
        policy: LightingLayerPolicy,
    },
    BeginSceneReplace {
        expected_revision: u32,
        cell_count: u16,
    },
    PutSceneChunk {
        transaction_id: u32,
        offset: u16,
        cells: Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE>,
    },
    CommitSceneReplace {
        transaction_id: u32,
    },
    AbortSceneReplace {
        transaction_id: u32,
    },
    SetState {
        expected_revision: u32,
        state: LightingMutableState,
    },
    SetOverlay {
        expected_revision: u32,
        cell: LightingOverlayCell,
    },
    UnsetOverlay {
        expected_revision: u32,
        led_id: rmk_types::protocol::rynk::LightingLedId,
    },
    ClearOverlay {
        expected_revision: u32,
    },
    ReplaceOverlay {
        expected_revision: u32,
    },
}

/// Bridges type-erased Rynk commands into one concrete standard lighting
/// mailbox. Boards spawn this alongside `LightingProcessor`.
pub struct StandardRynkLightingAdapter<
    'a,
    const OVERLAY_CAPACITY: usize,
    const CORE_COMMAND_CAPACITY: usize,
    const SCENE_CAP: usize = 0,
> {
    protocol: &'a RynkLightingMailbox,
    core: &'a LightingMailbox<
        StandardCommand<OVERLAY_CAPACITY, SCENE_CAP>,
        StandardReply,
        StandardError,
        CORE_COMMAND_CAPACITY,
    >,
    topology: LightingTopology<'a>,
}

impl<'a, const OVERLAY_CAPACITY: usize, const CORE_COMMAND_CAPACITY: usize, const SCENE_CAP: usize>
    StandardRynkLightingAdapter<'a, OVERLAY_CAPACITY, CORE_COMMAND_CAPACITY, SCENE_CAP>
{
    pub const fn new(
        protocol: &'a RynkLightingMailbox,
        core: &'a LightingMailbox<
            StandardCommand<OVERLAY_CAPACITY, SCENE_CAP>,
            StandardReply,
            StandardError,
            CORE_COMMAND_CAPACITY,
        >,
        topology: LightingTopology<'a>,
    ) -> Self {
        Self {
            protocol,
            core,
            topology,
        }
    }

    /// Process one protocol command. Exposed for board-specific executors and
    /// deterministic tests; [`Runnable`] simply repeats it forever.
    pub async fn process_next(&mut self) {
        let request = self.protocol.receive().await;
        let result = self.dispatch(request.id, request.command).await;
        self.protocol.reply(request.id, result);
    }

    async fn request_core(
        &self,
        command: StandardCommand<OVERLAY_CAPACITY, SCENE_CAP>,
    ) -> LightingResult<StandardReply> {
        self.core
            .request(command)
            .await
            .map_err(|error| map_standard_error(error, OVERLAY_CAPACITY))
    }

    async fn request_core_state(
        &self,
        command: StandardCommand<OVERLAY_CAPACITY, SCENE_CAP>,
    ) -> LightingResult<StandardState> {
        match self.request_core(command).await? {
            StandardReply::State(state) => Ok(state),
            _ => Err(LightingError::InvalidRequest),
        }
    }

    /// Extension readback shared by discovery and name paging. `None`
    /// descriptor/state (e.g. `EmptySource`) is not selectable → `Unsupported`.
    async fn request_extension_page(&self) -> LightingResult<crate::lighting::standard::ExtensionPage> {
        match self.request_core(StandardCommand::ReadExtension).await? {
            StandardReply::Extension(page) => Ok(page),
            _ => Err(LightingError::InvalidRequest),
        }
    }

    /// Run a scene mutation, then persist the whole authoritative scene
    /// configuration. Persisting by readback keeps the engine the single
    /// source of truth instead of mirroring its insertion algorithm here.
    async fn scene_mutation(
        &self,
        command: StandardCommand<OVERLAY_CAPACITY, SCENE_CAP>,
    ) -> LightingResult<StandardState> {
        let state = self.request_core_state(command).await?;
        self.persist_scenes(&state).await;
        Ok(state)
    }

    #[cfg(feature = "storage")]
    async fn persist_scenes(&self, state: &StandardState) {
        use crate::channel::FLASH_CHANNEL;
        use crate::storage::FlashOperationMessage;

        let mut total = state.scene_len.min(u16::MAX as usize) as u16;
        let mut offset: u16 = 0;
        let mut shard: u8 = 0;
        while offset < total {
            let Ok(StandardReply::ScenesPage(page)) = self.request_core(StandardCommand::ReadScenes { offset }).await
            else {
                return;
            };
            let cells = page.cells.as_slice();
            if cells.is_empty() {
                break;
            }
            let mut wire_cells: Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE> = Vec::new();
            for cell in cells {
                let Some(wire) = self.scene_cell_to_wire(*cell) else {
                    return;
                };
                let _ = wire_cells.push(wire);
            }
            FLASH_CHANNEL
                .send(FlashOperationMessage::LightingSceneShard {
                    index: shard,
                    cells: wire_cells,
                })
                .await;
            offset += cells.len() as u16;
            shard = shard.saturating_add(1);
            total = page.total;
        }
        FLASH_CHANNEL
            .send(FlashOperationMessage::LightingSceneTable {
                len: offset,
                policy: policy_to_wire(state.scene_policy),
            })
            .await;
    }

    #[cfg(not(feature = "storage"))]
    async fn persist_scenes(&self, _state: &StandardState) {}

    async fn dispatch(&self, request_id: u32, command: RynkLightingCommand) -> LightingResult<RynkLightingReadback> {
        let core_command = match command {
            RynkLightingCommand::ReadState => StandardCommand::ReadState,
            RynkLightingCommand::ReadOutputMode => {
                let state = self.request_core_state(StandardCommand::ReadState).await?;
                return Ok(RynkLightingReadback::OutputMode(state));
            }
            RynkLightingCommand::ReadOverlay {
                expected_revision,
                offset,
            } => {
                let page = match self.request_core(StandardCommand::ReadOverlay { offset }).await? {
                    StandardReply::OverlayPage(page) => page,
                    _ => return Err(LightingError::InvalidRequest),
                };
                if page.revision != expected_revision {
                    return Err(LightingError::StateRevisionConflict {
                        expected: expected_revision,
                        current: page.revision,
                    });
                }
                let mut items: Vec<LightingOverlayCell, LIGHTING_OVERLAY_CHUNK_SIZE> = Vec::new();
                for cell in page.cells.as_slice() {
                    items
                        .push(self.overlay_cell_to_wire(*cell).ok_or(LightingError::InvalidRequest)?)
                        .map_err(|_| LightingError::InvalidRequest)?;
                }
                return Ok(RynkLightingReadback::OverlayPage(LightingOverlayPage {
                    revision: page.revision,
                    total_count: page.total,
                    items,
                }));
            }
            RynkLightingCommand::ReadSceneStatus => {
                let state = self.request_core_state(StandardCommand::ReadState).await?;
                return Ok(RynkLightingReadback::SceneStatus {
                    revision: state.revision,
                    scene_len: state.scene_len.min(u16::MAX as usize) as u16,
                    policy: policy_to_wire(state.scene_policy),
                });
            }
            RynkLightingCommand::ReadScenes {
                expected_revision,
                offset,
            } => {
                let page = match self.request_core(StandardCommand::ReadScenes { offset }).await? {
                    StandardReply::ScenesPage(page) => page,
                    _ => return Err(LightingError::InvalidRequest),
                };
                // The page is read atomically by the engine; pinning only has
                // to compare the revision it was served under.
                if page.revision != expected_revision {
                    return Err(LightingError::StateRevisionConflict {
                        expected: expected_revision,
                        current: page.revision,
                    });
                }
                let mut items: Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE> = Vec::new();
                for cell in page.cells.as_slice() {
                    let wire = self.scene_cell_to_wire(*cell).ok_or(LightingError::InvalidRequest)?;
                    items.push(wire).map_err(|_| LightingError::InvalidRequest)?;
                }
                return Ok(RynkLightingReadback::ScenesPage(LightingScenesPage {
                    revision: page.revision,
                    total_count: page.total,
                    items,
                }));
            }
            RynkLightingCommand::ReadCompiledSceneStatus => {
                let page = match self
                    .request_core(StandardCommand::ReadCompiledScenes { offset: 0 })
                    .await?
                {
                    StandardReply::CompiledScenesPage(page) => page,
                    _ => return Err(LightingError::InvalidRequest),
                };
                return Ok(RynkLightingReadback::CompiledSceneStatus {
                    scene_len: page.total,
                    policy: policy_to_wire(page.policy),
                });
            }
            RynkLightingCommand::ReadCompiledScenes { offset } => {
                let page = match self
                    .request_core(StandardCommand::ReadCompiledScenes { offset })
                    .await?
                {
                    StandardReply::CompiledScenesPage(page) => page,
                    _ => return Err(LightingError::InvalidRequest),
                };
                let mut items: Vec<LightingSceneCell, LIGHTING_SCENE_CHUNK_SIZE> = Vec::new();
                for cell in page.cells.as_slice() {
                    items
                        .push(self.scene_cell_to_wire(*cell).ok_or(LightingError::InvalidRequest)?)
                        .map_err(|_| LightingError::InvalidRequest)?;
                }
                return Ok(RynkLightingReadback::CompiledScenesPage(LightingCompiledScenesPage {
                    topology_revision: 0,
                    total_count: page.total,
                    items,
                }));
            }
            RynkLightingCommand::ReadExtension => {
                let page = self.request_extension_page().await?;
                let (Some(descriptor), Some(state)) = (page.descriptor, page.state) else {
                    return Err(LightingError::Unsupported);
                };
                if descriptor.effects.len() > u8::MAX as usize || descriptor.palettes.len() > u8::MAX as usize {
                    return Err(LightingError::InvalidRequest);
                }
                return Ok(RynkLightingReadback::Extension(LightingExtension {
                    revision: page.revision,
                    effect_count: descriptor.effects.len() as u8,
                    palette_count: descriptor.palettes.len() as u8,
                    state: WireExtensionState {
                        effect: state.effect,
                        palette: state.palette,
                        value: state.value,
                        speed: state.speed,
                    },
                }));
            }
            RynkLightingCommand::ReadExtensionNames { kind, offset } => {
                let page = self.request_extension_page().await?;
                let Some(descriptor) = page.descriptor else {
                    return Err(LightingError::Unsupported);
                };
                if descriptor.effects.len() > u8::MAX as usize || descriptor.palettes.len() > u8::MAX as usize {
                    return Err(LightingError::InvalidRequest);
                }
                let names = match kind {
                    LightingExtensionNameKind::Effects => descriptor.effects,
                    LightingExtensionNameKind::Palettes => descriptor.palettes,
                };
                let start = (offset as usize).min(names.len());
                let end = (start + LIGHTING_EXTENSION_NAME_CHUNK).min(names.len());
                let mut items: Vec<String<LIGHTING_EXTENSION_NAME_SIZE>, LIGHTING_EXTENSION_NAME_CHUNK> = Vec::new();
                for name in &names[start..end] {
                    items.push(super::truncated(name)).expect("page is bounded");
                }
                return Ok(RynkLightingReadback::ExtensionNamesPage(LightingExtensionNamesPage {
                    total: names.len() as u8,
                    items,
                }));
            }
            RynkLightingCommand::SetExtensionState {
                expected_revision,
                state,
            } => StandardCommand::SetExtensionIfRevision {
                expected_revision,
                state: crate::lighting::compositor::ExtensionState {
                    effect: state.effect,
                    palette: state.palette,
                    value: state.value,
                    speed: state.speed,
                },
            },
            RynkLightingCommand::SetSceneCell {
                expected_revision,
                cell,
            } => {
                let cell = self.scene_cell_from_wire(cell)?;
                let state = self
                    .scene_mutation(StandardCommand::SetSceneCellIfRevision {
                        expected_revision,
                        cell,
                    })
                    .await?;
                return Ok(RynkLightingReadback::State(state_to_wire(state)));
            }
            RynkLightingCommand::UnsetSceneCell {
                expected_revision,
                layer,
                led_id,
            } => {
                let slot = self
                    .topology
                    .slot(LedId(led_id.0))
                    .ok_or(LightingError::UnknownLed { led_id })?;
                let state = self
                    .scene_mutation(StandardCommand::UnsetSceneCellIfRevision {
                        expected_revision,
                        layer,
                        slot,
                    })
                    .await?;
                return Ok(RynkLightingReadback::State(state_to_wire(state)));
            }
            RynkLightingCommand::SetLayerPolicy {
                expected_revision,
                policy,
            } => {
                let state = self
                    .scene_mutation(StandardCommand::SetLayerPolicyIfRevision {
                        expected_revision,
                        policy: policy_from_wire(policy),
                    })
                    .await?;
                return Ok(RynkLightingReadback::State(state_to_wire(state)));
            }
            RynkLightingCommand::BeginSceneReplace {
                expected_revision,
                cell_count,
            } => {
                let reply = self
                    .request_core(StandardCommand::BeginSceneReplace {
                        expected_revision,
                        cell_count,
                    })
                    .await?;
                return match reply {
                    StandardReply::SceneTransaction { id, cell_count } => {
                        Ok(RynkLightingReadback::SceneTransaction(LightingSceneTransaction {
                            id,
                            cell_count,
                        }))
                    }
                    _ => Err(LightingError::InvalidRequest),
                };
            }
            RynkLightingCommand::PutSceneChunk {
                transaction_id,
                offset,
                cells,
            } => {
                let mut chunk = SceneChunk::new();
                for cell in &cells {
                    chunk
                        .push(self.scene_cell_from_wire(*cell)?)
                        .map_err(|error| map_standard_error(error, OVERLAY_CAPACITY))?;
                }
                self.request_core_state(StandardCommand::PutSceneChunk {
                    transaction_id,
                    offset,
                    cells: chunk,
                })
                .await?;
                return Ok(RynkLightingReadback::Unit);
            }
            RynkLightingCommand::CommitSceneReplace { transaction_id } => {
                let state = self
                    .scene_mutation(StandardCommand::CommitSceneReplace { transaction_id })
                    .await?;
                return Ok(RynkLightingReadback::State(state_to_wire(state)));
            }
            RynkLightingCommand::AbortSceneReplace { transaction_id } => {
                self.request_core_state(StandardCommand::AbortSceneReplace { transaction_id })
                    .await?;
                return Ok(RynkLightingReadback::Unit);
            }
            RynkLightingCommand::SetState {
                expected_revision,
                state,
            } => StandardCommand::SetStateIfRevision {
                expected_revision,
                state: mutable_state_from_wire(state),
            },
            RynkLightingCommand::SetOverlay {
                expected_revision,
                cell,
            } => StandardCommand::SetOverlayIfRevision {
                expected_revision,
                cell: self.overlay_cell(cell)?,
            },
            RynkLightingCommand::UnsetOverlay {
                expected_revision,
                led_id,
            } => StandardCommand::UnsetOverlayIfRevision {
                expected_revision,
                slot: self
                    .topology
                    .slot(LedId(led_id.0))
                    .ok_or(LightingError::UnknownLed { led_id })?,
            },
            RynkLightingCommand::ClearOverlay { expected_revision } => {
                StandardCommand::ClearOverlayIfRevision { expected_revision }
            }
            RynkLightingCommand::ReplaceOverlay { expected_revision } => {
                let cells = self.protocol.take_replacement(request_id).await;
                let mut batch = OverlayBatch::new();
                for cell in cells {
                    let cell = self.overlay_cell(cell)?;
                    if batch.as_slice().iter().any(|existing| existing.slot == cell.slot) {
                        return Err(LightingError::InvalidRequest);
                    }
                    batch
                        .push(cell)
                        .map_err(|error| map_overlay_error(error, OVERLAY_CAPACITY))?;
                }
                StandardCommand::ReplaceOverlayIfRevision {
                    expected_revision,
                    batch,
                }
            }
        };

        self.request_core_state(core_command)
            .await
            .map(state_to_wire)
            .map(RynkLightingReadback::State)
    }

    fn overlay_cell(&self, cell: LightingOverlayCell) -> LightingResult<OverlayCell> {
        cell.validate()?;
        let slot = self
            .topology
            .slot(LedId(cell.led_id.0))
            .ok_or(LightingError::UnknownLed { led_id: cell.led_id })?;
        Ok(OverlayCell {
            slot,
            effect: effect_from_wire(cell.effect),
            ttl_ms: cell.ttl_ms.and_then(NonZeroU32::new),
        })
    }

    fn overlay_cell_to_wire(&self, cell: OverlayCell) -> Option<LightingOverlayCell> {
        let led = self.topology.led(cell.slot)?;
        Some(LightingOverlayCell {
            led_id: rmk_types::protocol::rynk::LightingLedId(led.id.0),
            effect: effect_to_wire(cell.effect),
            ttl_ms: cell.ttl_ms.map(NonZeroU32::get),
        })
    }

    fn scene_cell_from_wire(&self, cell: LightingSceneCell) -> LightingResult<SceneTableCell> {
        cell.validate()?;
        let slot = self
            .topology
            .slot(LedId(cell.led_id.0))
            .ok_or(LightingError::UnknownLed { led_id: cell.led_id })?;
        Ok(SceneTableCell {
            layer: cell.layer,
            slot,
            effect: effect_from_wire(cell.effect),
        })
    }

    fn scene_cell_to_wire(&self, cell: SceneTableCell) -> Option<LightingSceneCell> {
        let led = self.topology.led(cell.slot)?;
        Some(LightingSceneCell {
            layer: cell.layer,
            led_id: rmk_types::protocol::rynk::LightingLedId(led.id.0),
            effect: effect_to_wire(cell.effect),
        })
    }
}

/// Install persisted scene configuration into a standard engine at startup,
/// before the engine begins serving commands. Cells whose stable LED id no
/// longer resolves against the current topology are skipped rather than
/// failing the boot.
pub fn install_lighting_scenes<Extension, Status, const N: usize, const OVERLAY_CAP: usize, const SCENE_CAP: usize>(
    engine: &mut StandardLightingEngine<'_, Extension, Status, N, OVERLAY_CAP, SCENE_CAP>,
    topology: &LightingTopology<'_>,
    cells: &[LightingSceneCell],
    policy: Option<LightingLayerPolicy>,
) {
    if let Some(policy) = policy {
        engine.install_scene_policy(policy_from_wire(policy));
    }
    for cell in cells {
        if cell.validate().is_err() {
            continue;
        }
        let Some(slot) = topology.slot(LedId(cell.led_id.0)) else {
            continue;
        };
        let _ = engine.install_scene_cell(SceneTableCell {
            layer: cell.layer,
            slot,
            effect: effect_from_wire(cell.effect),
        });
    }
}

impl<const OVERLAY_CAPACITY: usize, const CORE_COMMAND_CAPACITY: usize, const SCENE_CAP: usize> Runnable
    for StandardRynkLightingAdapter<'_, OVERLAY_CAPACITY, CORE_COMMAND_CAPACITY, SCENE_CAP>
{
    async fn run(&mut self) -> ! {
        loop {
            self.process_next().await;
        }
    }
}

fn mutable_state_from_wire(state: LightingMutableState) -> StandardMutableState {
    StandardMutableState {
        output_enabled: state.output_enabled,
        output_brightness: state.output_brightness,
        background: BackgroundState {
            enabled: state.background.enabled,
            hue: state.background.hue,
            saturation: state.background.saturation,
            value: state.background.value,
            speed: state.background.speed,
            mode: match state.background.mode {
                LightingBackgroundMode::Solid => BackgroundMode::Solid,
                LightingBackgroundMode::Breathe => BackgroundMode::Breathe,
            },
        },
    }
}

pub(super) fn state_to_wire(state: StandardState) -> LightingState {
    LightingState {
        revision: state.revision,
        output_enabled: state.output_enabled,
        output_brightness: state.output_brightness,
        background: LightingBackgroundState {
            enabled: state.background.enabled,
            hue: state.background.hue,
            saturation: state.background.saturation,
            value: state.background.value,
            speed: state.background.speed,
            mode: match state.background.mode {
                BackgroundMode::Solid => LightingBackgroundMode::Solid,
                BackgroundMode::Breathe => LightingBackgroundMode::Breathe,
            },
        },
        overlay_len: state.overlay_len.min(u16::MAX as usize) as u16,
    }
}

fn effect_from_wire(effect: rmk_types::protocol::rynk::LightingEffect) -> BuiltinEffect {
    use rmk_types::protocol::rynk::LightingEffect;

    match effect {
        LightingEffect::Solid { color } => BuiltinEffect::Solid {
            color: rgb_from_wire(color),
        },
        LightingEffect::Blink {
            color,
            period_ms,
            phase_ms,
            duty,
        } => BuiltinEffect::Blink {
            color: rgb_from_wire(color),
            period_ms,
            phase_ms,
            duty,
        },
        LightingEffect::Breathe {
            color,
            period_ms,
            phase_ms,
            step_ms,
        } => BuiltinEffect::Breathe {
            color: rgb_from_wire(color),
            period_ms,
            phase_ms,
            step_ms,
        },
    }
}

const fn rgb_from_wire(color: LightingRgb8) -> Rgb8 {
    Rgb8::new(color.r, color.g, color.b)
}

const fn rgb_to_wire(color: Rgb8) -> LightingRgb8 {
    LightingRgb8 {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

fn effect_to_wire(effect: BuiltinEffect) -> rmk_types::protocol::rynk::LightingEffect {
    use rmk_types::protocol::rynk::LightingEffect;

    match effect {
        BuiltinEffect::Solid { color } => LightingEffect::Solid {
            color: rgb_to_wire(color),
        },
        BuiltinEffect::Blink {
            color,
            period_ms,
            phase_ms,
            duty,
        } => LightingEffect::Blink {
            color: rgb_to_wire(color),
            period_ms,
            phase_ms,
            duty,
        },
        BuiltinEffect::Breathe {
            color,
            period_ms,
            phase_ms,
            step_ms,
        } => LightingEffect::Breathe {
            color: rgb_to_wire(color),
            period_ms,
            phase_ms,
            step_ms,
        },
    }
}

fn condition_set_to_wire(conditions: crate::lighting::ConditionSet) -> rmk_types::protocol::rynk::LightingConditionSet {
    use rmk_types::protocol::rynk::{
        LightingBatteryCondition, LightingChargeCondition, LightingConditionSet, LightingLayerCondition, LightingNodeId,
    };

    use crate::lighting::ChargeCondition;

    LightingConditionSet {
        layer: conditions.layer.map(|condition| LightingLayerCondition {
            layer: condition.layer,
            active: condition.active,
        }),
        battery: conditions.battery.map(|condition| LightingBatteryCondition {
            node: LightingNodeId(condition.node),
            min_level: condition.min_level,
            max_level: condition.max_level,
            charge: match condition.charge {
                ChargeCondition::Any => LightingChargeCondition::Any,
                ChargeCondition::Charging => LightingChargeCondition::Charging,
                ChargeCondition::Discharging => LightingChargeCondition::Discharging,
                ChargeCondition::Unknown => LightingChargeCondition::Unknown,
            },
        }),
    }
}

pub(super) const fn policy_from_wire(policy: LightingLayerPolicy) -> LayerPolicy {
    match policy {
        LightingLayerPolicy::EffectiveOnly => LayerPolicy::EffectiveOnly,
        LightingLayerPolicy::ActiveStack => LayerPolicy::ActiveStack,
    }
}

pub(super) const fn policy_to_wire(policy: LayerPolicy) -> LightingLayerPolicy {
    match policy {
        LayerPolicy::EffectiveOnly => LightingLayerPolicy::EffectiveOnly,
        LayerPolicy::ActiveStack => LightingLayerPolicy::ActiveStack,
    }
}

fn map_standard_error(error: StandardError, capacity: usize) -> LightingError {
    match error {
        StandardError::RevisionConflict { expected, current } => {
            LightingError::StateRevisionConflict { expected, current }
        }
        StandardError::Overlay(error) => map_overlay_error(error, capacity),
        StandardError::DeadlineOverflow => LightingError::InvalidTtl,
        StandardError::ReplicaSlot(_) => LightingError::Unsupported,
        StandardError::Render(_) => LightingError::Unsupported,
        StandardError::SceneFull { capacity } => LightingError::SceneFull {
            capacity: capacity.min(u16::MAX as usize) as u16,
        },
        StandardError::SceneSlotOutOfRange { .. } | StandardError::InvalidSceneRequest => LightingError::InvalidRequest,
        StandardError::SceneTransactionBusy => LightingError::TransactionBusy,
        StandardError::InvalidSceneTransaction => LightingError::InvalidTransaction,
        StandardError::SceneTransactionExpired => LightingError::TransactionExpired,
        StandardError::SceneTransactionIncomplete { expected, received } => {
            LightingError::TransactionIncomplete { expected, received }
        }
        StandardError::ExtensionUnsupported => LightingError::Unsupported,
    }
}

fn map_overlay_error(error: OverlayError, capacity: usize) -> LightingError {
    match error {
        OverlayError::Full | OverlayError::TooManyEntries { .. } => LightingError::OverlayFull {
            capacity: capacity.min(u16::MAX as usize) as u16,
        },
        OverlayError::DuplicateSlot { .. } => LightingError::InvalidRequest,
    }
}

#[cfg(test)]
mod tests {
    use embassy_futures::join::join;
    use embassy_futures::select::{Either, select};
    use embassy_futures::yield_now;
    use rmk_types::protocol::rynk::{
        LightingBackgroundMode, LightingBackgroundState, LightingEffect, LightingLedId, LightingOverlayCell,
        LightingRgb8,
    };

    use super::*;
    use crate::lighting::{LedMetadata, MatrixSize, ZoneSpan};
    use crate::physical_layout::PhysicalLayout;
    use crate::test_support::test_block_on as block_on;

    static TEST_LEDS: [LedMetadata; 1] = [LedMetadata {
        id: LedId(10),
        key: None,
        position: None,
        zones: ZoneSpan::new(0, 0),
    }];

    fn topology() -> LightingTopology<'static> {
        LightingTopology {
            matrix: MatrixSize::new(0, 0),
            keys: &[],
            physical_layout: PhysicalLayout::EMPTY,
            leds: &TEST_LEDS,
            zones: &[],
            zone_memberships: &[],
        }
    }

    fn cell(id: u16) -> LightingOverlayCell {
        LightingOverlayCell {
            led_id: LightingLedId(id),
            effect: LightingEffect::Solid {
                color: LightingRgb8 { r: 1, g: 2, b: 3 },
            },
            ttl_ms: None,
        }
    }

    fn state(revision: u32) -> LightingState {
        LightingState {
            revision,
            output_enabled: true,
            output_brightness: 255,
            background: LightingBackgroundState {
                enabled: true,
                hue: 0,
                saturation: 0,
                value: 1,
                speed: 0,
                mode: LightingBackgroundMode::Solid,
            },
            overlay_len: 1,
        }
    }

    #[test]
    fn cancelled_replacement_cannot_be_overwritten_by_the_next_caller() {
        let mailbox = RynkLightingMailbox::new();
        let mut first = Vec::new();
        first.push(cell(10)).unwrap();

        let cancelled = block_on(select(mailbox.request_replace(0, &first), async {
            yield_now().await;
        }));
        assert!(matches!(cancelled, Either::Second(())));

        let mut second = Vec::new();
        second.push(cell(20)).unwrap();
        let (reply, ()) = block_on(join(mailbox.request_replace(1, &second), async {
            let abandoned = mailbox.receive().await;
            assert!(matches!(
                abandoned.command,
                RynkLightingCommand::ReplaceOverlay { expected_revision: 0 }
            ));
            let abandoned_cells = mailbox.take_replacement(abandoned.id).await;
            assert_eq!(abandoned_cells.as_slice(), &[cell(10)]);
            mailbox.reply(abandoned.id, Ok(RynkLightingReadback::State(state(1))));

            let current = mailbox.receive().await;
            assert!(matches!(
                current.command,
                RynkLightingCommand::ReplaceOverlay { expected_revision: 1 }
            ));
            let current_cells = mailbox.take_replacement(current.id).await;
            assert_eq!(current_cells.as_slice(), &[cell(20)]);
            mailbox.reply(current.id, Ok(RynkLightingReadback::State(state(2))));
        }));
        assert_eq!(reply, Ok(RynkLightingReadback::State(state(2))));
    }

    #[test]
    fn replacement_rejects_duplicate_stable_ids_before_reaching_the_core() {
        let protocol = RynkLightingMailbox::new();
        let core = LightingMailbox::<StandardCommand<2>, StandardReply, StandardError, 1>::new();
        let mut adapter = StandardRynkLightingAdapter::new(&protocol, &core, topology());
        let mut cells = Vec::new();
        cells.push(cell(10)).unwrap();
        cells.push(cell(10)).unwrap();

        let (reply, ()) = block_on(join(protocol.request_replace(0, &cells), adapter.process_next()));
        assert_eq!(reply, Err(LightingError::InvalidRequest));
    }

    static TEST_EFFECT_NAMES: &[&str] = &["Gradient", "Flow", "ABCDEFGHIJKLMNOé tail"];
    static TEST_PALETTE_NAMES: &[&str] = &["P0", "P1", "P2", "P3", "P4", "P5", "P6", "P7", "P8", "P9"];
    static TOO_MANY_EFFECT_NAMES: [&str; 256] = ["Effect"; 256];

    /// Zero-target source whose only job is serving the extension hooks.
    struct TestExtensionSource {
        state: crate::lighting::compositor::ExtensionState,
    }

    impl<Context> crate::lighting::compositor::LightingSource<Rgb8, Context> for TestExtensionSource {
        fn len(&self, _: &crate::lighting::compositor::RenderInput<'_, Context>) -> usize {
            0
        }

        fn slot(
            &self,
            _: usize,
            _: &crate::lighting::compositor::RenderInput<'_, Context>,
        ) -> crate::lighting::LedSlot {
            unreachable!("test extension source has no targets")
        }

        fn contribution(
            &mut self,
            _: usize,
            _: &crate::lighting::compositor::RenderInput<'_, Context>,
        ) -> crate::lighting::compositor::Contribution<Rgb8> {
            unreachable!("test extension source has no samples")
        }

        fn extension_descriptor(&self) -> Option<crate::lighting::compositor::ExtensionDescriptor> {
            Some(crate::lighting::compositor::ExtensionDescriptor {
                effects: TEST_EFFECT_NAMES,
                palettes: TEST_PALETTE_NAMES,
            })
        }

        fn extension_state(&self) -> Option<crate::lighting::compositor::ExtensionState> {
            Some(self.state)
        }

        fn apply_extension_state(&mut self, state: crate::lighting::compositor::ExtensionState) -> bool {
            self.state = state;
            true
        }
    }

    struct TooManyNamesSource(TestExtensionSource);

    impl<Context> crate::lighting::compositor::LightingSource<Rgb8, Context> for TooManyNamesSource {
        fn len(&self, _: &crate::lighting::compositor::RenderInput<'_, Context>) -> usize {
            0
        }

        fn slot(
            &self,
            _: usize,
            _: &crate::lighting::compositor::RenderInput<'_, Context>,
        ) -> crate::lighting::LedSlot {
            unreachable!("test extension source has no targets")
        }

        fn contribution(
            &mut self,
            _: usize,
            _: &crate::lighting::compositor::RenderInput<'_, Context>,
        ) -> crate::lighting::compositor::Contribution<Rgb8> {
            unreachable!("test extension source has no samples")
        }

        fn extension_descriptor(&self) -> Option<crate::lighting::compositor::ExtensionDescriptor> {
            Some(crate::lighting::compositor::ExtensionDescriptor {
                effects: &TOO_MANY_EFFECT_NAMES,
                palettes: TEST_PALETTE_NAMES,
            })
        }

        fn extension_state(&self) -> Option<crate::lighting::compositor::ExtensionState> {
            Some(self.0.state)
        }
    }

    /// Serve `client` against a live adapter + engine built around `extension`.
    fn run_extension_flow<Extension, T>(extension: Extension, client: impl AsyncFnOnce(&RynkLightingMailbox) -> T) -> T
    where
        Extension: crate::lighting::compositor::LightingSource<Rgb8, crate::lighting::LightingContext>,
    {
        use embassy_futures::select::{Either3, select3};

        use crate::lighting::{
            BackgroundState, EmptySource, LayerPolicy, LayerScenes, LightingContext, LightingEngine,
            StandardLightingEngine,
        };

        block_on(async {
            let protocol = RynkLightingMailbox::new();
            let core = LightingMailbox::<StandardCommand<2>, StandardReply, StandardError, 1>::new();
            let mut adapter = StandardRynkLightingAdapter::<2, 1>::new(&protocol, &core, topology());
            let mut engine: StandardLightingEngine<'static, Extension, EmptySource, 1, 2, 0> =
                StandardLightingEngine::new(
                    BackgroundState::default(),
                    LayerScenes {
                        scenes: &[],
                        policy: LayerPolicy::EffectiveOnly,
                    },
                    extension,
                    EmptySource,
                );

            let adapter_loop = async {
                loop {
                    adapter.process_next().await;
                }
            };
            let context = LightingContext::default();
            let engine_loop = async {
                loop {
                    let (id, command) = core.receive_request().await;
                    let result = engine.handle_command(0, command, &context).map(|outcome| outcome.reply);
                    core.publish_reply(id, result);
                }
            };
            match select3(client(&protocol), adapter_loop, engine_loop).await {
                Either3::First(value) => value,
                _ => panic!("service loops must not finish"),
            }
        })
    }

    #[test]
    fn extension_flow_reads_pages_truncates_and_guards_revision() {
        run_extension_flow(
            TestExtensionSource {
                state: crate::lighting::compositor::ExtensionState {
                    effect: 0,
                    palette: 1,
                    value: 128,
                    speed: 20,
                },
            },
            async |protocol| {
                let extension = match protocol.request(RynkLightingCommand::ReadExtension).await {
                    Ok(RynkLightingReadback::Extension(extension)) => extension,
                    other => panic!("expected extension readback, got {other:?}"),
                };
                assert_eq!(extension.revision, 0);
                assert_eq!(extension.effect_count, 3);
                assert_eq!(extension.palette_count, 10);
                assert_eq!(
                    extension.state,
                    WireExtensionState {
                        effect: 0,
                        palette: 1,
                        value: 128,
                        speed: 20,
                    }
                );

                // Overlong names are truncated to the wire size on a char
                // boundary: the multi-byte 'é' straddling byte 16 is dropped.
                let effects = match protocol
                    .request(RynkLightingCommand::ReadExtensionNames {
                        kind: LightingExtensionNameKind::Effects,
                        offset: 0,
                    })
                    .await
                {
                    Ok(RynkLightingReadback::ExtensionNamesPage(page)) => page,
                    other => panic!("expected names page, got {other:?}"),
                };
                assert_eq!(effects.total, 3);
                assert_eq!(effects.items.len(), 3);
                assert_eq!(effects.items[0].as_str(), "Gradient");
                assert_eq!(effects.items[2].as_str(), "ABCDEFGHIJKLMNO");

                // Ten palettes page as one full chunk plus a two-name tail;
                // an out-of-range offset yields an empty page, correct total.
                for (offset, expected) in [
                    (0u8, &TEST_PALETTE_NAMES[..8]),
                    (8, &TEST_PALETTE_NAMES[8..]),
                    (32, &[][..]),
                ] {
                    let page = match protocol
                        .request(RynkLightingCommand::ReadExtensionNames {
                            kind: LightingExtensionNameKind::Palettes,
                            offset,
                        })
                        .await
                    {
                        Ok(RynkLightingReadback::ExtensionNamesPage(page)) => page,
                        other => panic!("expected names page, got {other:?}"),
                    };
                    assert_eq!(page.total, 10);
                    assert_eq!(page.items.len(), expected.len());
                    for (item, name) in page.items.iter().zip(expected) {
                        assert_eq!(item.as_str(), *name);
                    }
                }

                let selected = WireExtensionState {
                    effect: 2,
                    palette: 9,
                    value: 7,
                    speed: 3,
                };
                let state = match protocol
                    .request(RynkLightingCommand::SetExtensionState {
                        expected_revision: 0,
                        state: selected,
                    })
                    .await
                {
                    Ok(RynkLightingReadback::State(state)) => state,
                    other => panic!("expected state readback, got {other:?}"),
                };
                assert_eq!(state.revision, 1);

                let extension = match protocol.request(RynkLightingCommand::ReadExtension).await {
                    Ok(RynkLightingReadback::Extension(extension)) => extension,
                    other => panic!("expected extension readback, got {other:?}"),
                };
                assert_eq!(extension.revision, 1);
                assert_eq!(extension.state, selected);

                let stale = protocol
                    .request(RynkLightingCommand::SetExtensionState {
                        expected_revision: 0,
                        state: selected,
                    })
                    .await;
                assert_eq!(
                    stale,
                    Err(LightingError::StateRevisionConflict {
                        expected: 0,
                        current: 1,
                    })
                );
            },
        );
    }

    #[test]
    fn extension_commands_are_unsupported_with_an_empty_source() {
        run_extension_flow(crate::lighting::EmptySource, async |protocol| {
            let read = protocol.request(RynkLightingCommand::ReadExtension).await;
            assert_eq!(read, Err(LightingError::Unsupported));
            let names = protocol
                .request(RynkLightingCommand::ReadExtensionNames {
                    kind: LightingExtensionNameKind::Effects,
                    offset: 0,
                })
                .await;
            assert_eq!(names, Err(LightingError::Unsupported));
            let set = protocol
                .request(RynkLightingCommand::SetExtensionState {
                    expected_revision: 0,
                    state: WireExtensionState {
                        effect: 0,
                        palette: 0,
                        value: 0,
                        speed: 0,
                    },
                })
                .await;
            assert_eq!(set, Err(LightingError::Unsupported));
        });
    }

    #[test]
    fn extension_commands_reject_descriptors_that_exceed_wire_counts() {
        run_extension_flow(
            TooManyNamesSource(TestExtensionSource {
                state: crate::lighting::compositor::ExtensionState {
                    effect: 0,
                    palette: 0,
                    value: 128,
                    speed: 20,
                },
            }),
            async |protocol| {
                assert_eq!(
                    protocol.request(RynkLightingCommand::ReadExtension).await,
                    Err(LightingError::InvalidRequest)
                );
                assert_eq!(
                    protocol
                        .request(RynkLightingCommand::ReadExtensionNames {
                            kind: LightingExtensionNameKind::Effects,
                            offset: 248,
                        })
                        .await,
                    Err(LightingError::InvalidRequest)
                );
            },
        );
    }
}
