//! Native Rynk lighting handlers and bounded replacement transactions.

use embassy_time::Instant;
use heapless::{String, Vec};
use rmk_types::protocol::rynk::command::{
    ClearLightingOverlay, GetLightingCapabilities, GetLightingCompiledSceneStatus, GetLightingCompiledScenes,
    GetLightingConditionalSceneStatus, GetLightingConditionalScenes, GetLightingExtension, GetLightingExtensionNames,
    GetLightingKeys, GetLightingLeds, GetLightingOutputMode, GetLightingOutputs, GetLightingOverlay,
    GetLightingPhysicalKeys, GetLightingRoutes, GetLightingSceneStatus, GetLightingScenes, GetLightingState,
    GetLightingZoneMemberships, GetLightingZones, SetLightingExtensionState, SetLightingLayerPolicy,
    SetLightingOverlay, SetLightingSceneCell, SetLightingState, UnsetLightingOverlay, UnsetLightingSceneCell,
};
use rmk_types::protocol::rynk::{
    AbortLightingOverlayReplaceRequest, AbortLightingSceneReplaceRequest, BeginLightingOverlayReplaceRequest,
    BeginLightingSceneReplaceRequest, ClearLightingOverlayRequest, CommitLightingOverlayReplaceRequest,
    CommitLightingSceneReplaceRequest, LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE, LIGHTING_PAGE_SIZE,
    LIGHTING_SCENE_CHUNK_SIZE, LIGHTING_ZONE_NAME_SIZE, LightingCapabilities, LightingCapabilitiesResult,
    LightingCompiledSceneStatus, LightingCompiledSceneStatusResult, LightingCompiledScenesPageResult,
    LightingConditionalSceneCell, LightingConditionalSceneStatus, LightingConditionalSceneStatusResult,
    LightingConditionalScenesPage, LightingConditionalScenesPageResult, LightingEffectFlags, LightingError,
    LightingExtensionNamesPageResult, LightingExtensionNamesRequest, LightingExtensionResult, LightingFeatureFlags,
    LightingKeysPage, LightingKeysPageResult, LightingLed, LightingLedId, LightingLedsPage, LightingLedsPageResult,
    LightingMatrixPosition, LightingOutput, LightingOutputCapabilities, LightingOutputCoverage,
    LightingOutputModeStateResult, LightingOutputsPage, LightingOutputsPageResult, LightingOverlayCell,
    LightingOverlayPageRequest, LightingOverlayPageResult, LightingOverlayTransaction,
    LightingOverlayTransactionResult, LightingPageRequest, LightingPhysicalKey, LightingPhysicalKeysPage,
    LightingPhysicalKeysPageResult, LightingPoint3, LightingResult, LightingRoute, LightingRoutesPage,
    LightingRoutesPageResult, LightingScenePageRequest, LightingSceneStatus, LightingSceneStatusResult,
    LightingSceneTransactionResult, LightingScenesPageResult, LightingState, LightingStateResult, LightingUnitResult,
    LightingZone, LightingZoneId, LightingZoneMembershipsPage, LightingZoneMembershipsPageResult, LightingZonesPage,
    LightingZonesPageResult, PutLightingOverlayChunkRequest, PutLightingSceneChunkRequest, RynkError, RynkMessage,
    SetLightingExtensionStateRequest, SetLightingLayerPolicyRequest, SetLightingOverlayRequest,
    SetLightingSceneCellRequest, SetLightingStateRequest, UnsetLightingOverlayRequest, UnsetLightingSceneCellRequest,
};

use super::super::lighting::{
    RYNK_LIGHTING_TRANSACTION_CAPACITY, RynkLightingCommand, RynkLightingController, RynkLightingReadback,
};
use super::super::{RynkService, RynkSession};
use super::Handle;
use crate::lighting::OutputCoverage;

const TRANSACTION_TIMEOUT_MS: u64 = 5_000;

fn controller<'a>(service: &RynkService<'a>) -> LightingResult<RynkLightingController<'a>> {
    service.lighting.ok_or(LightingError::Unsupported)
}

/// Scene endpoints require a board that wired a runtime scene table; a
/// controller advertising `scene_capacity == 0` has none.
fn scene_controller<'a>(service: &RynkService<'a>) -> LightingResult<RynkLightingController<'a>> {
    let controller = controller(service)?;
    if controller.scene_capacity == 0 {
        return Err(LightingError::Unsupported);
    }
    Ok(controller)
}

/// Extension endpoints require a board that advertised a host-selectable
/// extension source via `with_extension_effects`.
fn extension_controller<'a>(service: &RynkService<'a>) -> LightingResult<RynkLightingController<'a>> {
    let controller = controller(service)?;
    if !controller.extension_effects {
        return Err(LightingError::Unsupported);
    }
    Ok(controller)
}

impl Handle<GetLightingCapabilities> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LightingCapabilitiesResult, RynkError> {
        Ok(controller(self).map(capabilities))
    }
}

impl Handle<GetLightingState> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LightingStateResult, RynkError> {
        Ok(match controller(self) {
            Ok(controller) => controller.request_state(RynkLightingCommand::ReadState).await,
            Err(error) => Err(error),
        })
    }
}

impl Handle<GetLightingOutputMode> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LightingOutputModeStateResult, RynkError> {
        Ok(match controller(self) {
            Ok(controller) => {
                controller
                    .request(RynkLightingCommand::ReadOutputMode)
                    .await
                    .and_then(|reply| match reply {
                        RynkLightingReadback::OutputMode(state) => Ok(controller.output_mode_to_wire(state)),
                        _ => Err(LightingError::InvalidRequest),
                    })
            }
            Err(error) => Err(error),
        })
    }
}

impl Handle<GetLightingOverlay> for RynkService<'_> {
    async fn handle(&self, req: LightingOverlayPageRequest) -> Result<LightingOverlayPageResult, RynkError> {
        let result = match controller(self) {
            Ok(controller) => match controller
                .request(RynkLightingCommand::ReadOverlay {
                    expected_revision: req.revision,
                    offset: req.offset,
                })
                .await
            {
                Ok(RynkLightingReadback::OverlayPage(page)) => Ok(page),
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<SetLightingState> for RynkService<'_> {
    async fn handle(&self, req: SetLightingStateRequest) -> Result<LightingStateResult, RynkError> {
        let result = match controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::SetState {
                        expected_revision: req.expected_revision,
                        state: req.state,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<SetLightingOverlay> for RynkService<'_> {
    async fn handle(&self, req: SetLightingOverlayRequest) -> Result<LightingStateResult, RynkError> {
        if let Err(error) = req.cell.validate() {
            return Ok(Err(error));
        }
        let result = match controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::SetOverlay {
                        expected_revision: req.expected_revision,
                        cell: req.cell,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<UnsetLightingOverlay> for RynkService<'_> {
    async fn handle(&self, req: UnsetLightingOverlayRequest) -> Result<LightingStateResult, RynkError> {
        let result = match controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::UnsetOverlay {
                        expected_revision: req.expected_revision,
                        led_id: req.led_id,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<ClearLightingOverlay> for RynkService<'_> {
    async fn handle(&self, req: ClearLightingOverlayRequest) -> Result<LightingStateResult, RynkError> {
        let result = match controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::ClearOverlay {
                        expected_revision: req.expected_revision,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl RynkService<'_> {
    /// `UnknownLayer` for a scene layer outside the live keymap.
    fn check_scene_layer(&self, layer: u8) -> LightingResult<()> {
        let (_, _, num_layers) = self.ctx.keymap_dimensions();
        if (layer as usize) < num_layers {
            Ok(())
        } else {
            Err(LightingError::UnknownLayer { layer })
        }
    }
}

impl Handle<GetLightingSceneStatus> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LightingSceneStatusResult, RynkError> {
        let result = match scene_controller(self) {
            Ok(controller) => match controller.request(RynkLightingCommand::ReadSceneStatus).await {
                Ok(RynkLightingReadback::SceneStatus {
                    revision,
                    scene_len,
                    policy,
                }) => Ok(LightingSceneStatus {
                    revision,
                    capacity: controller.scene_capacity,
                    scene_len,
                    policy,
                    chunk_capacity: LIGHTING_SCENE_CHUNK_SIZE as u8,
                }),
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<GetLightingScenes> for RynkService<'_> {
    async fn handle(&self, req: LightingScenePageRequest) -> Result<LightingScenesPageResult, RynkError> {
        let result = match scene_controller(self) {
            Ok(controller) => match controller
                .request(RynkLightingCommand::ReadScenes {
                    expected_revision: req.revision,
                    offset: req.offset,
                })
                .await
            {
                Ok(RynkLightingReadback::ScenesPage(page)) => Ok(page),
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<GetLightingCompiledSceneStatus> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LightingCompiledSceneStatusResult, RynkError> {
        let result = match controller(self) {
            Ok(controller) => match controller.request(RynkLightingCommand::ReadCompiledSceneStatus).await {
                Ok(RynkLightingReadback::CompiledSceneStatus { scene_len, policy }) => {
                    Ok(LightingCompiledSceneStatus {
                        topology_revision: controller.descriptor.topology_revision,
                        scene_len,
                        policy,
                        chunk_capacity: LIGHTING_SCENE_CHUNK_SIZE as u8,
                    })
                }
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<GetLightingCompiledScenes> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingCompiledScenesPageResult, RynkError> {
        let result = match controller(self).and_then(|controller| {
            check_topology_revision(controller, req.topology_revision)?;
            Ok(controller)
        }) {
            Ok(controller) => match controller
                .request(RynkLightingCommand::ReadCompiledScenes { offset: req.offset })
                .await
            {
                Ok(RynkLightingReadback::CompiledScenesPage(mut page)) => {
                    page.topology_revision = controller.descriptor.topology_revision;
                    Ok(page)
                }
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<GetLightingConditionalSceneStatus> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LightingConditionalSceneStatusResult, RynkError> {
        Ok(controller(self).map(|controller| LightingConditionalSceneStatus {
            topology_revision: controller.descriptor.topology_revision,
            cell_len: count(controller.conditional_scenes.len()),
            chunk_capacity: LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE as u8,
            controls: controller.controls_to_wire(),
        }))
    }
}

impl Handle<GetLightingConditionalScenes> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingConditionalScenesPageResult, RynkError> {
        let result = controller(self).and_then(|controller| {
            check_topology_revision(controller, req.topology_revision)?;
            let total_count = count(controller.conditional_scenes.len());
            let start = (req.offset as usize).min(controller.conditional_scenes.len());
            let end = (start + LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE).min(controller.conditional_scenes.len());
            let mut items: Vec<LightingConditionalSceneCell, LIGHTING_CONDITIONAL_SCENE_CHUNK_SIZE> = Vec::new();
            for cell in &controller.conditional_scenes[start..end] {
                items
                    .push(
                        controller
                            .conditional_scene_cell_to_wire(*cell)
                            .ok_or(LightingError::InvalidRequest)?,
                    )
                    .map_err(|_| LightingError::InvalidRequest)?;
            }
            Ok(LightingConditionalScenesPage {
                topology_revision: controller.descriptor.topology_revision,
                total_count,
                items,
            })
        });
        Ok(result)
    }
}

impl Handle<SetLightingSceneCell> for RynkService<'_> {
    async fn handle(&self, req: SetLightingSceneCellRequest) -> Result<LightingStateResult, RynkError> {
        if let Err(error) = req
            .cell
            .validate()
            .and_then(|()| self.check_scene_layer(req.cell.layer))
        {
            return Ok(Err(error));
        }
        let result = match scene_controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::SetSceneCell {
                        expected_revision: req.expected_revision,
                        cell: req.cell,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<UnsetLightingSceneCell> for RynkService<'_> {
    async fn handle(&self, req: UnsetLightingSceneCellRequest) -> Result<LightingStateResult, RynkError> {
        if let Err(error) = self.check_scene_layer(req.layer) {
            return Ok(Err(error));
        }
        let result = match scene_controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::UnsetSceneCell {
                        expected_revision: req.expected_revision,
                        layer: req.layer,
                        led_id: req.led_id,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<SetLightingLayerPolicy> for RynkService<'_> {
    async fn handle(&self, req: SetLightingLayerPolicyRequest) -> Result<LightingStateResult, RynkError> {
        let result = match scene_controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::SetLayerPolicy {
                        expected_revision: req.expected_revision,
                        policy: req.policy,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<rmk_types::protocol::rynk::command::BeginLightingSceneReplace> for RynkService<'_> {
    async fn handle(&self, req: BeginLightingSceneReplaceRequest) -> Result<LightingSceneTransactionResult, RynkError> {
        let result = match scene_controller(self) {
            Ok(controller) => {
                if req.cell_count > controller.scene_capacity {
                    Err(LightingError::SceneFull {
                        capacity: controller.scene_capacity,
                    })
                } else {
                    match controller
                        .request(RynkLightingCommand::BeginSceneReplace {
                            expected_revision: req.expected_revision,
                            cell_count: req.cell_count,
                        })
                        .await
                    {
                        Ok(RynkLightingReadback::SceneTransaction(transaction)) => Ok(transaction),
                        Ok(_) => Err(LightingError::InvalidRequest),
                        Err(error) => Err(error),
                    }
                }
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<rmk_types::protocol::rynk::command::PutLightingSceneChunk> for RynkService<'_> {
    async fn handle(&self, req: PutLightingSceneChunkRequest) -> Result<LightingUnitResult, RynkError> {
        for cell in &req.cells {
            if let Err(error) = cell.validate().and_then(|()| self.check_scene_layer(cell.layer)) {
                return Ok(Err(error));
            }
        }
        let result = match scene_controller(self) {
            Ok(controller) => match controller
                .request(RynkLightingCommand::PutSceneChunk {
                    transaction_id: req.transaction_id,
                    offset: req.offset,
                    cells: req.cells,
                })
                .await
            {
                Ok(RynkLightingReadback::Unit) => Ok(()),
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<rmk_types::protocol::rynk::command::CommitLightingSceneReplace> for RynkService<'_> {
    async fn handle(&self, req: CommitLightingSceneReplaceRequest) -> Result<LightingStateResult, RynkError> {
        let result = match scene_controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::CommitSceneReplace {
                        transaction_id: req.transaction_id,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<rmk_types::protocol::rynk::command::AbortLightingSceneReplace> for RynkService<'_> {
    async fn handle(&self, req: AbortLightingSceneReplaceRequest) -> Result<LightingUnitResult, RynkError> {
        let result = match scene_controller(self) {
            Ok(controller) => match controller
                .request(RynkLightingCommand::AbortSceneReplace {
                    transaction_id: req.transaction_id,
                })
                .await
            {
                Ok(RynkLightingReadback::Unit) => Ok(()),
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<GetLightingExtension> for RynkService<'_> {
    async fn handle(&self, _: ()) -> Result<LightingExtensionResult, RynkError> {
        let result = match extension_controller(self) {
            Ok(controller) => match controller.request(RynkLightingCommand::ReadExtension).await {
                Ok(RynkLightingReadback::Extension(extension)) => Ok(extension),
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<GetLightingExtensionNames> for RynkService<'_> {
    async fn handle(&self, req: LightingExtensionNamesRequest) -> Result<LightingExtensionNamesPageResult, RynkError> {
        let result = match extension_controller(self) {
            Ok(controller) => match controller
                .request(RynkLightingCommand::ReadExtensionNames {
                    kind: req.kind,
                    offset: req.offset,
                })
                .await
            {
                Ok(RynkLightingReadback::ExtensionNamesPage(page)) => Ok(page),
                Ok(_) => Err(LightingError::InvalidRequest),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<SetLightingExtensionState> for RynkService<'_> {
    async fn handle(&self, req: SetLightingExtensionStateRequest) -> Result<LightingStateResult, RynkError> {
        let result = match extension_controller(self) {
            Ok(controller) => {
                controller
                    .request_state(RynkLightingCommand::SetExtensionState {
                        expected_revision: req.expected_revision,
                        state: req.state,
                    })
                    .await
            }
            Err(error) => Err(error),
        };
        Ok(result)
    }
}

impl Handle<GetLightingKeys> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingKeysPageResult, RynkError> {
        Ok(controller(self).and_then(|binding| keys_page(binding, req)))
    }
}

impl Handle<GetLightingPhysicalKeys> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingPhysicalKeysPageResult, RynkError> {
        Ok(controller(self).and_then(|binding| physical_keys_page(binding, req)))
    }
}

impl Handle<GetLightingLeds> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingLedsPageResult, RynkError> {
        Ok(controller(self).and_then(|binding| leds_page(binding, req)))
    }
}

impl Handle<GetLightingZones> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingZonesPageResult, RynkError> {
        Ok(controller(self).and_then(|binding| zones_page(binding, req)))
    }
}

impl Handle<GetLightingZoneMemberships> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingZoneMembershipsPageResult, RynkError> {
        Ok(controller(self).and_then(|binding| zone_memberships_page(binding, req)))
    }
}

impl Handle<GetLightingOutputs> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingOutputsPageResult, RynkError> {
        Ok(controller(self).and_then(|binding| outputs_page(binding, req)))
    }
}

impl Handle<GetLightingRoutes> for RynkService<'_> {
    async fn handle(&self, req: LightingPageRequest) -> Result<LightingRoutesPageResult, RynkError> {
        Ok(controller(self).and_then(|binding| routes_page(binding, req)))
    }
}

fn capabilities(binding: RynkLightingController<'_>) -> LightingCapabilities {
    let descriptor = binding.descriptor;
    let topology = descriptor.topology;
    let routing = descriptor.routing;
    let mut features = LightingFeatureFlags(
        LightingFeatureFlags::OVERLAY_TTL
            | LightingFeatureFlags::ATOMIC_OVERLAY_REPLACE
            | LightingFeatureFlags::LAYER_AWARE
            | LightingFeatureFlags::OVERLAY_READBACK
            | LightingFeatureFlags::COMPILED_LAYER_SCENES,
    );
    if !topology.physical_layout.keys.is_empty() {
        features.0 |= LightingFeatureFlags::PHYSICAL_GEOMETRY;
    }
    if !topology.zones.is_empty() {
        features.0 |= LightingFeatureFlags::ZONES;
    }
    if !routing.routes.is_empty() || !routing.outputs.is_empty() {
        features.0 |= LightingFeatureFlags::ROUTING;
    }
    if binding.scene_capacity > 0 {
        features.0 |= LightingFeatureFlags::LAYER_SCENES;
    }
    if !binding.conditional_scenes.is_empty() {
        features.0 |= LightingFeatureFlags::COMPILED_CONDITIONAL_SCENES;
    }
    if binding.controls.output_mode_cycle_user_action.is_some() {
        features.0 |= LightingFeatureFlags::OUTPUT_MODE;
    }
    if binding.extension_effects {
        features.0 |= LightingFeatureFlags::EXTENSION_EFFECTS;
    }
    LightingCapabilities {
        topology_revision: descriptor.topology_revision,
        logical_key_count: count(topology.keys.len()),
        physical_key_count: count(topology.physical_layout.keys.len()),
        led_count: count(topology.leds.len()),
        zone_count: count(topology.zones.len()),
        zone_membership_count: count(topology.zone_memberships.len()),
        output_count: count(routing.outputs.len()),
        route_count: count(routing.routes.len()),
        overlay_capacity: binding.overlay_capacity,
        page_capacity: LIGHTING_PAGE_SIZE as u8,
        overlay_chunk_capacity: rmk_types::protocol::rynk::LIGHTING_OVERLAY_CHUNK_SIZE as u8,
        features,
        effects: LightingEffectFlags(
            LightingEffectFlags::SOLID | LightingEffectFlags::BLINK | LightingEffectFlags::BREATHE,
        ),
    }
}

fn count(len: usize) -> u16 {
    len.min(u16::MAX as usize) as u16
}

fn check_topology_revision(binding: RynkLightingController<'_>, expected: u32) -> LightingResult<()> {
    let current = binding.descriptor.topology_revision;
    if expected == current {
        Ok(())
    } else {
        Err(LightingError::TopologyRevisionConflict { expected, current })
    }
}

fn page_range(offset: u16, total: usize) -> core::ops::Range<usize> {
    let start = (offset as usize).min(total);
    start..(start + LIGHTING_PAGE_SIZE).min(total)
}

fn matrix(position: crate::physical_layout::KeyPosition) -> LightingMatrixPosition {
    LightingMatrixPosition {
        row: position.row,
        col: position.col,
    }
}

fn point(point: crate::physical_layout::Point3) -> LightingPoint3 {
    LightingPoint3 {
        x: point.x.raw(),
        y: point.y.raw(),
        z: point.z.raw(),
    }
}

fn keys_page(binding: RynkLightingController<'_>, req: LightingPageRequest) -> LightingResult<LightingKeysPage> {
    check_topology_revision(binding, req.topology_revision)?;
    let values = binding.descriptor.topology.keys;
    let mut items = Vec::new();
    for value in &values[page_range(req.offset, values.len())] {
        items.push(matrix(*value)).expect("page is bounded");
    }
    Ok(LightingKeysPage {
        topology_revision: req.topology_revision,
        total_count: count(values.len()),
        items,
    })
}

fn physical_keys_page(
    binding: RynkLightingController<'_>,
    req: LightingPageRequest,
) -> LightingResult<LightingPhysicalKeysPage> {
    check_topology_revision(binding, req.topology_revision)?;
    let values = binding.descriptor.topology.physical_layout.keys;
    let mut items = Vec::new();
    for value in &values[page_range(req.offset, values.len())] {
        items
            .push(LightingPhysicalKey {
                matrix: matrix(value.matrix),
                center: point(value.center),
                size: rmk_types::protocol::rynk::LightingKeySize {
                    width: value.size.width.raw(),
                    height: value.size.height.raw(),
                },
                rotation: value.rotation.centidegrees(),
            })
            .expect("page is bounded");
    }
    Ok(LightingPhysicalKeysPage {
        topology_revision: req.topology_revision,
        total_count: count(values.len()),
        items,
    })
}

fn leds_page(binding: RynkLightingController<'_>, req: LightingPageRequest) -> LightingResult<LightingLedsPage> {
    check_topology_revision(binding, req.topology_revision)?;
    let values = binding.descriptor.topology.leds;
    let mut items = Vec::new();
    for value in &values[page_range(req.offset, values.len())] {
        items
            .push(LightingLed {
                id: LightingLedId(value.id.0),
                key: value.key.map(matrix),
                position: value.position.map(point),
                zone_start: value.zones.start,
                zone_len: value.zones.len,
            })
            .expect("page is bounded");
    }
    Ok(LightingLedsPage {
        topology_revision: req.topology_revision,
        total_count: count(values.len()),
        items,
    })
}

fn zones_page(binding: RynkLightingController<'_>, req: LightingPageRequest) -> LightingResult<LightingZonesPage> {
    check_topology_revision(binding, req.topology_revision)?;
    let values = binding.descriptor.topology.zones;
    let mut items = Vec::new();
    for value in &values[page_range(req.offset, values.len())] {
        let mut name = String::<LIGHTING_ZONE_NAME_SIZE>::new();
        for character in value.name.chars() {
            if name.push(character).is_err() {
                break;
            }
        }
        items
            .push(LightingZone {
                id: LightingZoneId(value.id.0),
                name,
            })
            .expect("page is bounded");
    }
    Ok(LightingZonesPage {
        topology_revision: req.topology_revision,
        total_count: count(values.len()),
        items,
    })
}

fn zone_memberships_page(
    binding: RynkLightingController<'_>,
    req: LightingPageRequest,
) -> LightingResult<LightingZoneMembershipsPage> {
    check_topology_revision(binding, req.topology_revision)?;
    let values = binding.descriptor.topology.zone_memberships;
    let mut items = Vec::new();
    for value in &values[page_range(req.offset, values.len())] {
        items.push(LightingZoneId(value.0)).expect("page is bounded");
    }
    Ok(LightingZoneMembershipsPage {
        topology_revision: req.topology_revision,
        total_count: count(values.len()),
        items,
    })
}

fn outputs_page(binding: RynkLightingController<'_>, req: LightingPageRequest) -> LightingResult<LightingOutputsPage> {
    check_topology_revision(binding, req.topology_revision)?;
    let values = binding.descriptor.routing.outputs;
    let mut items = Vec::new();
    for value in &values[page_range(req.offset, values.len())] {
        items
            .push(LightingOutput {
                node: rmk_types::protocol::rynk::LightingNodeId(value.node.0),
                id: rmk_types::protocol::rynk::LightingOutputId(value.id.0),
                pixel_count: value.pixel_count,
                capabilities: LightingOutputCapabilities(value.capabilities.bits()),
                coverage: match value.coverage {
                    OutputCoverage::Complete => LightingOutputCoverage::Complete,
                    OutputCoverage::Sparse => LightingOutputCoverage::Sparse,
                },
            })
            .expect("page is bounded");
    }
    Ok(LightingOutputsPage {
        topology_revision: req.topology_revision,
        total_count: count(values.len()),
        items,
    })
}

fn routes_page(binding: RynkLightingController<'_>, req: LightingPageRequest) -> LightingResult<LightingRoutesPage> {
    check_topology_revision(binding, req.topology_revision)?;
    let descriptor = binding.descriptor;
    let values = descriptor.routing.routes;
    let mut items = Vec::new();
    for value in &values[page_range(req.offset, values.len())] {
        let led = descriptor
            .topology
            .led(value.slot)
            .ok_or(LightingError::InvalidRequest)?;
        items
            .push(LightingRoute {
                led_id: LightingLedId(led.id.0),
                node: rmk_types::protocol::rynk::LightingNodeId(value.node.0),
                output: rmk_types::protocol::rynk::LightingOutputId(value.output.0),
                physical_index: value.physical_index,
            })
            .expect("page is bounded");
    }
    Ok(LightingRoutesPage {
        topology_revision: req.topology_revision,
        total_count: count(values.len()),
        items,
    })
}

struct ActiveTransaction {
    id: u32,
    expected_revision: u32,
    expected_count: u16,
    cells: Vec<LightingOverlayCell, RYNK_LIGHTING_TRANSACTION_CAPACITY>,
    last_activity_ms: u64,
}

#[derive(Clone, Copy)]
struct CachedCommit {
    id: u32,
    state: LightingState,
}

pub(in crate::host::rynk) struct LightingTransactionState {
    next_id: u32,
    active: Option<ActiveTransaction>,
    cached_commit: Option<CachedCommit>,
    expired_id: Option<u32>,
}

impl LightingTransactionState {
    pub(in crate::host::rynk) const fn new() -> Self {
        Self {
            next_id: 1,
            active: None,
            cached_commit: None,
            expired_id: None,
        }
    }

    fn expire(&mut self, now_ms: u64) {
        if self
            .active
            .as_ref()
            .is_some_and(|transaction| now_ms.saturating_sub(transaction.last_activity_ms) >= TRANSACTION_TIMEOUT_MS)
        {
            self.expired_id = self.active.as_ref().map(|transaction| transaction.id);
            self.active = None;
        }
    }

    fn transaction_error(&self, id: u32) -> LightingError {
        if self.expired_id == Some(id) {
            LightingError::TransactionExpired
        } else {
            LightingError::InvalidTransaction
        }
    }
}

pub(in crate::host::rynk) async fn serve_begin(
    service: &RynkService<'_>,
    session: &RynkSession,
    msg: &mut RynkMessage<'_>,
) -> Result<(), RynkError> {
    let req = msg.decode_request::<BeginLightingOverlayReplaceRequest>()?;
    let result = begin(service, session, req, Instant::now().as_millis()).await;
    msg.encode_response(&result)
}

async fn begin(
    service: &RynkService<'_>,
    session: &RynkSession,
    req: BeginLightingOverlayReplaceRequest,
    now_ms: u64,
) -> LightingOverlayTransactionResult {
    let binding = controller(service)?;
    let capacity = binding.overlay_capacity.min(RYNK_LIGHTING_TRANSACTION_CAPACITY as u16);
    if req.cell_count > capacity {
        return Err(LightingError::OverlayFull { capacity });
    }
    let mut state = session.lighting.lock().await;
    state.expire(now_ms);
    if state.active.is_some() {
        return Err(LightingError::TransactionBusy);
    }
    let id = state.next_id;
    state.next_id = state.next_id.wrapping_add(1).max(1);
    state.expired_id = None;
    state.active = Some(ActiveTransaction {
        id,
        expected_revision: req.expected_revision,
        expected_count: req.cell_count,
        cells: Vec::new(),
        last_activity_ms: now_ms,
    });
    Ok(LightingOverlayTransaction {
        id,
        cell_count: req.cell_count,
    })
}

pub(in crate::host::rynk) async fn serve_put(
    service: &RynkService<'_>,
    session: &RynkSession,
    msg: &mut RynkMessage<'_>,
) -> Result<(), RynkError> {
    let req = msg.decode_request::<PutLightingOverlayChunkRequest>()?;
    let result = put(service, session, req, Instant::now().as_millis()).await;
    msg.encode_response(&result)
}

async fn put(
    service: &RynkService<'_>,
    session: &RynkSession,
    req: PutLightingOverlayChunkRequest,
    now_ms: u64,
) -> LightingUnitResult {
    controller(service)?;
    for cell in &req.cells {
        cell.validate()?;
    }
    let mut state = session.lighting.lock().await;
    state.expire(now_ms);
    let error = state.transaction_error(req.transaction_id);
    let transaction = state
        .active
        .as_mut()
        .filter(|transaction| transaction.id == req.transaction_id)
        .ok_or(error)?;
    if req.offset as usize != transaction.cells.len()
        || transaction.cells.len() + req.cells.len() > transaction.expected_count as usize
    {
        return Err(LightingError::InvalidRequest);
    }
    transaction
        .cells
        .extend_from_slice(&req.cells)
        .map_err(|_| LightingError::OverlayFull {
            capacity: RYNK_LIGHTING_TRANSACTION_CAPACITY as u16,
        })?;
    transaction.last_activity_ms = now_ms;
    Ok(())
}

pub(in crate::host::rynk) async fn serve_commit(
    service: &RynkService<'_>,
    session: &RynkSession,
    msg: &mut RynkMessage<'_>,
) -> Result<(), RynkError> {
    let req = msg.decode_request::<CommitLightingOverlayReplaceRequest>()?;
    let result = commit(service, session, req, Instant::now().as_millis()).await;
    msg.encode_response(&result)
}

async fn commit(
    service: &RynkService<'_>,
    session: &RynkSession,
    req: CommitLightingOverlayReplaceRequest,
    now_ms: u64,
) -> LightingStateResult {
    let binding = controller(service)?;
    let (expected_revision, cells) = {
        let mut state = session.lighting.lock().await;
        state.expire(now_ms);
        if let Some(cached) = state.cached_commit.filter(|cached| cached.id == req.transaction_id) {
            return Ok(cached.state);
        }
        let error = state.transaction_error(req.transaction_id);
        let transaction = state
            .active
            .as_ref()
            .filter(|transaction| transaction.id == req.transaction_id)
            .ok_or(error)?;
        if transaction.cells.len() != transaction.expected_count as usize {
            return Err(LightingError::TransactionIncomplete {
                expected: transaction.expected_count,
                received: transaction.cells.len() as u16,
            });
        }
        (transaction.expected_revision, transaction.cells.clone())
    };

    let result = binding.replace_overlay(expected_revision, &cells).await;
    if let Ok(state_value) = result {
        let mut state = session.lighting.lock().await;
        if state
            .active
            .as_ref()
            .is_some_and(|active| active.id == req.transaction_id)
        {
            state.active = None;
            state.cached_commit = Some(CachedCommit {
                id: req.transaction_id,
                state: state_value,
            });
            state.expired_id = None;
        }
    }
    result
}

pub(in crate::host::rynk) async fn serve_abort(
    service: &RynkService<'_>,
    session: &RynkSession,
    msg: &mut RynkMessage<'_>,
) -> Result<(), RynkError> {
    let req = msg.decode_request::<AbortLightingOverlayReplaceRequest>()?;
    let result = abort(service, session, req, Instant::now().as_millis()).await;
    msg.encode_response(&result)
}

async fn abort(
    service: &RynkService<'_>,
    session: &RynkSession,
    req: AbortLightingOverlayReplaceRequest,
    now_ms: u64,
) -> LightingUnitResult {
    controller(service)?;
    let mut state = session.lighting.lock().await;
    state.expire(now_ms);
    if state
        .active
        .as_ref()
        .is_some_and(|active| active.id == req.transaction_id)
    {
        state.active = None;
        return Ok(());
    }
    Err(state.transaction_error(req.transaction_id))
}

#[cfg(test)]
mod tests {
    extern crate alloc;

    use alloc::boxed::Box;
    use core::fmt::Debug;

    use embassy_futures::join::join;
    use rmk_types::action::KeyAction;
    use rmk_types::protocol::rynk::command::{
        BeginLightingOverlayReplace, CommitLightingOverlayReplace, GetCapabilities, GetLightingKeys, SetLightingState,
    };
    use rmk_types::protocol::rynk::endpoint::Endpoint;
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    use super::*;
    use crate::config::{BehaviorConfig, PositionalConfig, RmkConfig};
    use crate::host::RynkLightingMailbox;
    use crate::keymap::{KeyMap, KeymapData};
    use crate::lighting::{
        BatteryCondition, BuiltinEffect, ChargeCondition, ConditionSet, ConditionalSceneCell, LayerCondition,
        LayerScene, LedId, LedMetadata, LightingNodeId as CoreNodeId, LightingRouting, LightingTopology, MatrixSize,
        OutputId, OutputMetadata, PhysicalRoute, Rgb8, SceneCell, ZoneSpan,
    };
    use crate::physical_layout::{
        Coordinate, Extent, KeyPosition, KeySize, PhysicalKey, PhysicalLayout, Point3, Rotation,
    };
    use crate::test_support::test_block_on as block_on;

    static LOGICAL_KEYS: [KeyPosition; 2] = [KeyPosition::new(0, 0), KeyPosition::new(0, 2)];
    static PHYSICAL_KEYS: [PhysicalKey; 1] = [PhysicalKey {
        matrix: KeyPosition::new(0, 0),
        center: Point3::new(Coordinate::ZERO, Coordinate::ONE, Coordinate::ZERO),
        size: KeySize::new(Extent::ONE, Extent::ONE),
        rotation: Rotation::ZERO,
    }];
    static LEDS: [LedMetadata; 2] = [
        LedMetadata {
            id: LedId(10),
            key: Some(KeyPosition::new(0, 0)),
            position: None,
            zones: ZoneSpan::new(0, 0),
        },
        LedMetadata {
            id: LedId(42),
            key: None,
            position: Some(Point3::new(Coordinate::ONE, Coordinate::ZERO, Coordinate::ZERO)),
            zones: ZoneSpan::new(0, 0),
        },
    ];
    static OUTPUTS: [OutputMetadata; 1] = [OutputMetadata {
        node: CoreNodeId(0),
        id: OutputId(0),
        pixel_count: 2,
        capabilities: crate::lighting::OutputCapabilities::RGB,
        coverage: OutputCoverage::Complete,
    }];
    static ROUTES: [PhysicalRoute; 2] = [
        PhysicalRoute {
            slot: crate::lighting::LedSlot(1),
            node: CoreNodeId(0),
            output: OutputId(0),
            physical_index: 0,
        },
        PhysicalRoute {
            slot: crate::lighting::LedSlot(0),
            node: CoreNodeId(0),
            output: OutputId(0),
            physical_index: 1,
        },
    ];
    static COMPILED_CELLS: [SceneCell<BuiltinEffect>; 2] = [
        SceneCell {
            slot: crate::lighting::LedSlot(0),
            effect: BuiltinEffect::Solid {
                color: Rgb8::new(10, 20, 30),
            },
        },
        SceneCell {
            slot: crate::lighting::LedSlot(1),
            effect: BuiltinEffect::Solid {
                color: Rgb8::new(40, 50, 60),
            },
        },
    ];
    static COMPILED_LAYERS: [LayerScene<'static, BuiltinEffect>; 1] = [LayerScene {
        layer: 0,
        cells: &COMPILED_CELLS,
    }];
    static CONDITIONAL_CELLS: [ConditionalSceneCell<BuiltinEffect>; 1] = [ConditionalSceneCell {
        conditions: ConditionSet {
            layer: Some(LayerCondition { layer: 1, active: true }),
            battery: Some(BatteryCondition {
                node: 0,
                min_level: Some(21),
                max_level: Some(40),
                charge: ChargeCondition::Discharging,
            }),
        },
        slot: crate::lighting::LedSlot(0),
        effect: BuiltinEffect::Solid {
            color: Rgb8::new(9, 8, 7),
        },
    }];

    fn descriptor() -> super::super::super::lighting::RynkLightingDescriptor<'static> {
        super::super::super::lighting::RynkLightingDescriptor {
            topology_revision: 7,
            topology: LightingTopology {
                matrix: MatrixSize::new(1, 3),
                keys: &LOGICAL_KEYS,
                physical_layout: PhysicalLayout::new(&PHYSICAL_KEYS),
                leds: &LEDS,
                zones: &[],
                zone_memberships: &[],
            },
            routing: LightingRouting {
                outputs: &OUTPUTS,
                routes: &ROUTES,
            },
        }
    }

    fn state(revision: u32, overlay_len: u16) -> LightingState {
        LightingState {
            revision,
            output_enabled: true,
            output_brightness: 200,
            background: rmk_types::protocol::rynk::LightingBackgroundState {
                enabled: true,
                hue: 1,
                saturation: 2,
                value: 3,
                speed: 4,
                mode: rmk_types::protocol::rynk::LightingBackgroundMode::Solid,
            },
            overlay_len,
        }
    }

    fn session() -> RynkSession {
        RynkSession {
            lighting: embassy_sync::mutex::Mutex::new(LightingTransactionState::new()),
        }
    }

    async fn call<E>(
        service: &RynkService<'_>,
        session: &RynkSession,
        request: &E::Request,
    ) -> Result<E::Response, RynkError>
    where
        E: Endpoint,
        E::Request: Serialize,
        E::Response: DeserializeOwned + Debug,
    {
        let mut buffer = Box::new([0u8; rmk_types::constants::RYNK_BUFFER_SIZE]);
        let mut message = RynkMessage::build(&mut buffer[..], E::CMD, 1, request).unwrap();
        service.dispatch(session, &mut message).await;
        postcard::from_bytes(message.payload()).unwrap()
    }

    fn overlay_cell(id: u16) -> LightingOverlayCell {
        LightingOverlayCell {
            led_id: LightingLedId(id),
            effect: rmk_types::protocol::rynk::LightingEffect::Solid {
                color: rmk_types::protocol::rynk::LightingRgb8 { r: 1, g: 2, b: 3 },
            },
            ttl_ms: None,
        }
    }

    #[test]
    fn page_range_clamps_and_preserves_empty_final_page() {
        assert_eq!(page_range(0, 20), 0..8);
        assert_eq!(page_range(16, 20), 16..20);
        assert_eq!(page_range(99, 20), 20..20);
    }

    #[test]
    fn transaction_expiry_is_remembered() {
        let mut state = LightingTransactionState::new();
        state.active = Some(ActiveTransaction {
            id: 7,
            expected_revision: 0,
            expected_count: 0,
            cells: Vec::new(),
            last_activity_ms: 10,
        });
        state.expire(10 + TRANSACTION_TIMEOUT_MS);
        assert!(state.active.is_none());
        assert_eq!(state.transaction_error(7), LightingError::TransactionExpired);
        assert_eq!(state.transaction_error(8), LightingError::InvalidTransaction);
    }

    #[test]
    fn discovery_is_false_unbound_and_true_only_when_attached() {
        block_on(async {
            let mut behavior = BehaviorConfig::default();
            let positional: PositionalConfig<1, 3> = PositionalConfig::default();
            let mut data: KeymapData<1, 3, 1, 0> = KeymapData::new([[[KeyAction::No; 3]]]);
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;
            let mut config = RmkConfig::default();
            config.lock_config.insecure = true;
            let unbound = RynkService::new(&keymap, &config);
            let unbound_session = session();
            let device_caps = call::<GetCapabilities>(&unbound, &unbound_session, &()).await.unwrap();
            assert!(!device_caps.lighting_enabled);
            drop(unbound_session);

            let mailbox = RynkLightingMailbox::new();
            let bound = RynkService::new(&keymap, &config).with_lighting(RynkLightingController::new(
                &mailbox,
                descriptor(),
                128,
            ));
            let bound_session = session();
            let device_caps = call::<GetCapabilities>(&bound, &bound_session, &()).await.unwrap();
            assert!(device_caps.lighting_enabled);
            let lighting = capabilities(bound.lighting.unwrap());
            assert_eq!(lighting.overlay_capacity, RYNK_LIGHTING_TRANSACTION_CAPACITY as u16);
            assert_eq!(lighting.logical_key_count, 2);
            assert_eq!(lighting.physical_key_count, 1);
        });
    }

    #[test]
    fn topology_pages_are_revision_pinned_and_keep_keys_separate_from_geometry() {
        block_on(async {
            let mut behavior = BehaviorConfig::default();
            let positional: PositionalConfig<1, 3> = PositionalConfig::default();
            let mut data: KeymapData<1, 3, 1, 0> = KeymapData::new([[[KeyAction::No; 3]]]);
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;
            let mut config = RmkConfig::default();
            config.lock_config.insecure = true;
            let mailbox = RynkLightingMailbox::new();
            let service = RynkService::new(&keymap, &config).with_lighting(RynkLightingController::new(
                &mailbox,
                descriptor(),
                8,
            ));
            let session = session();

            let stale = call::<GetLightingKeys>(
                &service,
                &session,
                &LightingPageRequest {
                    topology_revision: 6,
                    offset: 0,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                stale,
                Err(LightingError::TopologyRevisionConflict {
                    expected: 6,
                    current: 7
                })
            );

            let page = call::<GetLightingKeys>(
                &service,
                &session,
                &LightingPageRequest {
                    topology_revision: 7,
                    offset: 0,
                },
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(page.total_count, 2);
            assert_eq!(page.items[1], LightingMatrixPosition { row: 0, col: 2 });
            assert_eq!(descriptor().topology.physical_layout.keys.len(), 1);
        });
    }

    #[test]
    fn stale_mutation_is_a_nested_domain_error() {
        block_on(async {
            let mut behavior = BehaviorConfig::default();
            let positional: PositionalConfig<1, 3> = PositionalConfig::default();
            let mut data: KeymapData<1, 3, 1, 0> = KeymapData::new([[[KeyAction::No; 3]]]);
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;
            let mut config = RmkConfig::default();
            config.lock_config.insecure = true;
            let mailbox = RynkLightingMailbox::new();
            let service = RynkService::new(&keymap, &config).with_lighting(RynkLightingController::new(
                &mailbox,
                descriptor(),
                8,
            ));
            let session = session();
            let request = SetLightingStateRequest {
                expected_revision: 3,
                state: rmk_types::protocol::rynk::LightingMutableState {
                    output_enabled: false,
                    output_brightness: 9,
                    background: state(0, 0).background,
                },
            };
            let (response, ()) = join(call::<SetLightingState>(&service, &session, &request), async {
                let pending = mailbox.receive().await;
                assert!(matches!(
                    pending.command,
                    RynkLightingCommand::SetState {
                        expected_revision: 3,
                        ..
                    }
                ));
                mailbox.reply(
                    pending.id,
                    Err(LightingError::StateRevisionConflict {
                        expected: 3,
                        current: 4,
                    }),
                );
            })
            .await;
            assert_eq!(
                response.unwrap(),
                Err(LightingError::StateRevisionConflict {
                    expected: 3,
                    current: 4
                })
            );
        });
    }

    #[test]
    fn replacement_enforces_order_completion_expiry_and_idempotent_commit() {
        block_on(async {
            let mut behavior = BehaviorConfig::default();
            let positional: PositionalConfig<1, 3> = PositionalConfig::default();
            let mut data: KeymapData<1, 3, 1, 0> = KeymapData::new([[[KeyAction::No; 3]]]);
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;
            let mut config = RmkConfig::default();
            config.lock_config.insecure = true;
            let mailbox = RynkLightingMailbox::new();
            let service = RynkService::new(&keymap, &config).with_lighting(RynkLightingController::new(
                &mailbox,
                descriptor(),
                8,
            ));
            let session = session();

            let transaction = begin(
                &service,
                &session,
                BeginLightingOverlayReplaceRequest {
                    expected_revision: 5,
                    cell_count: 2,
                },
                10,
            )
            .await
            .unwrap();
            let mut out_of_order = Vec::new();
            out_of_order.push(overlay_cell(10)).unwrap();
            assert_eq!(
                put(
                    &service,
                    &session,
                    PutLightingOverlayChunkRequest {
                        transaction_id: transaction.id,
                        offset: 1,
                        cells: out_of_order,
                    },
                    11,
                )
                .await,
                Err(LightingError::InvalidRequest)
            );
            assert_eq!(
                commit(
                    &service,
                    &session,
                    CommitLightingOverlayReplaceRequest {
                        transaction_id: transaction.id,
                    },
                    12,
                )
                .await,
                Err(LightingError::TransactionIncomplete {
                    expected: 2,
                    received: 0
                })
            );

            let mut cells = Vec::new();
            cells.push(overlay_cell(10)).unwrap();
            cells.push(overlay_cell(42)).unwrap();
            put(
                &service,
                &session,
                PutLightingOverlayChunkRequest {
                    transaction_id: transaction.id,
                    offset: 0,
                    cells,
                },
                13,
            )
            .await
            .unwrap();
            let committed = state(6, 2);
            let (response, ()) = join(
                commit(
                    &service,
                    &session,
                    CommitLightingOverlayReplaceRequest {
                        transaction_id: transaction.id,
                    },
                    14,
                ),
                async {
                    let pending = mailbox.receive().await;
                    let staged = mailbox.take_replacement(pending.id).await;
                    match pending.command {
                        RynkLightingCommand::ReplaceOverlay { expected_revision } => {
                            assert_eq!(expected_revision, 5);
                            assert_eq!(staged.len(), 2);
                        }
                        _ => panic!("expected replacement"),
                    }
                    mailbox.reply(pending.id, Ok(RynkLightingReadback::State(committed)));
                },
            )
            .await;
            assert_eq!(response, Ok(committed));
            assert_eq!(
                commit(
                    &service,
                    &session,
                    CommitLightingOverlayReplaceRequest {
                        transaction_id: transaction.id,
                    },
                    15,
                )
                .await,
                Ok(committed),
                "repeated successful commit is served from the cache"
            );

            let expiring = begin(
                &service,
                &session,
                BeginLightingOverlayReplaceRequest {
                    expected_revision: 6,
                    cell_count: 0,
                },
                20,
            )
            .await
            .unwrap();
            assert_eq!(
                abort(
                    &service,
                    &session,
                    AbortLightingOverlayReplaceRequest {
                        transaction_id: expiring.id,
                    },
                    20 + TRANSACTION_TIMEOUT_MS,
                )
                .await,
                Err(LightingError::TransactionExpired)
            );
        });
    }

    #[test]
    fn zero_cell_replacement_commits_as_one_atomic_empty_batch() {
        block_on(async {
            let mut behavior = BehaviorConfig::default();
            let positional: PositionalConfig<1, 3> = PositionalConfig::default();
            let mut data: KeymapData<1, 3, 1, 0> = KeymapData::new([[[KeyAction::No; 3]]]);
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;
            let mut config = RmkConfig::default();
            config.lock_config.insecure = true;
            let mailbox = RynkLightingMailbox::new();
            let service = RynkService::new(&keymap, &config).with_lighting(RynkLightingController::new(
                &mailbox,
                descriptor(),
                8,
            ));
            let session = session();
            let transaction = call::<BeginLightingOverlayReplace>(
                &service,
                &session,
                &BeginLightingOverlayReplaceRequest {
                    expected_revision: 9,
                    cell_count: 0,
                },
            )
            .await
            .unwrap()
            .unwrap();
            let cleared = state(10, 0);
            let (response, ()) = join(
                call::<CommitLightingOverlayReplace>(
                    &service,
                    &session,
                    &CommitLightingOverlayReplaceRequest {
                        transaction_id: transaction.id,
                    },
                ),
                async {
                    let pending = mailbox.receive().await;
                    let staged = mailbox.take_replacement(pending.id).await;
                    assert!(matches!(
                        pending.command,
                        RynkLightingCommand::ReplaceOverlay {
                            expected_revision: 9,
                        } if staged.is_empty()
                    ));
                    mailbox.reply(pending.id, Ok(RynkLightingReadback::State(cleared)));
                },
            )
            .await;
            assert_eq!(response.unwrap(), Ok(cleared));
        });
    }

    fn wire_scene_cell(layer: u8, led: u16) -> rmk_types::protocol::rynk::LightingSceneCell {
        rmk_types::protocol::rynk::LightingSceneCell {
            layer,
            led_id: LightingLedId(led),
            effect: rmk_types::protocol::rynk::LightingEffect::Solid {
                color: rmk_types::protocol::rynk::LightingRgb8 { r: 5, g: 6, b: 7 },
            },
        }
    }

    #[test]
    fn scene_endpoints_reject_unsupported_without_a_scene_capacity() {
        block_on(async {
            let mut behavior = BehaviorConfig::default();
            let positional: PositionalConfig<1, 3> = PositionalConfig::default();
            let mut data: KeymapData<1, 3, 1, 0> = KeymapData::new([[[KeyAction::No; 3]]]);
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;
            let mut config = RmkConfig::default();
            config.lock_config.insecure = true;
            let mailbox = RynkLightingMailbox::new();
            // Lighting is bound, but the board wired no runtime scene table.
            let service = RynkService::new(&keymap, &config).with_lighting(RynkLightingController::new(
                &mailbox,
                descriptor(),
                8,
            ));
            let session = session(&keymap, &config);

            let lighting = capabilities(service.lighting.unwrap());
            assert!(!lighting.features.contains(LightingFeatureFlags::LAYER_SCENES));
            assert!(
                lighting.features.contains(LightingFeatureFlags::COMPILED_LAYER_SCENES),
                "compiled-scene readback is supported even when the source is empty"
            );

            let status = call::<rmk_types::protocol::rynk::command::GetLightingSceneStatus>(&service, &session, &())
                .await
                .unwrap();
            assert_eq!(status, Err(LightingError::Unsupported));
            let set = call::<rmk_types::protocol::rynk::command::SetLightingSceneCell>(
                &service,
                &session,
                &rmk_types::protocol::rynk::SetLightingSceneCellRequest {
                    expected_revision: 0,
                    cell: wire_scene_cell(0, 10),
                },
            )
            .await
            .unwrap();
            assert_eq!(set, Err(LightingError::Unsupported));
        });
    }

    /// Full stack: handler validation → protocol mailbox → adapter → engine,
    /// with scene persistence observed on the flash channel.
    #[test]
    fn scene_endpoints_flow_through_adapter_and_engine() {
        use embassy_futures::select::{Either3, select3};
        use rmk_types::protocol::rynk::command::{
            AbortLightingSceneReplace, BeginLightingSceneReplace, CommitLightingSceneReplace,
            GetLightingCompiledSceneStatus, GetLightingCompiledScenes, GetLightingConditionalSceneStatus,
            GetLightingConditionalScenes, GetLightingOutputMode, GetLightingOverlay, GetLightingSceneStatus,
            GetLightingScenes, PutLightingSceneChunk, SetLightingLayerPolicy, SetLightingOverlay, SetLightingSceneCell,
            UnsetLightingSceneCell,
        };
        use rmk_types::protocol::rynk::{
            BeginLightingSceneReplaceRequest, CommitLightingSceneReplaceRequest, LightingLayerPolicy,
            LightingOverlayPageRequest, LightingPageRequest, LightingScenePageRequest, PutLightingSceneChunkRequest,
            SetLightingLayerPolicyRequest, SetLightingOverlayRequest, SetLightingSceneCellRequest,
            UnsetLightingSceneCellRequest,
        };

        use crate::lighting::{
            BackgroundState, BuiltinEffect, EmptySource, LayerPolicy, LayerScenes, LedSlot, LightingContext,
            LightingControls, LightingEngine, LightingMailbox, OutputMode, OutputModeIndicator, Rgb8, StandardCommand,
            StandardError, StandardLightingEngine, StandardReply,
        };

        block_on(async {
            let mut behavior = BehaviorConfig::default();
            let positional: PositionalConfig<1, 3> = PositionalConfig::default();
            let mut data: KeymapData<1, 3, 2, 0> = KeymapData::new([[[KeyAction::No; 3]]; 2]);
            let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;
            let mut config = RmkConfig::default();
            config.lock_config.insecure = true;
            let mailbox = RynkLightingMailbox::new();
            let controls = LightingControls {
                output_toggle_user_action: Some(13),
                output_mode_cycle_user_action: Some(14),
                wake_layer: Some(1),
                initial_output_mode: OutputMode::PoweredOnly,
                powered_only_scope: crate::lighting::PoweredOnlyScope::Local,
                output_mode_indicator: Some(OutputModeIndicator {
                    slot: LedSlot(0),
                    always_on: BuiltinEffect::solid(Rgb8::new(0, 9, 0)),
                    always_off: BuiltinEffect::solid(Rgb8::new(9, 0, 0)),
                    powered_only: BuiltinEffect::solid(Rgb8::new(0, 0, 9)),
                }),
            };
            let service = RynkService::new(&keymap, &config).with_lighting(
                RynkLightingController::new(&mailbox, descriptor(), 8)
                    .with_scene_capacity(4)
                    .with_conditional_scenes(&CONDITIONAL_CELLS)
                    .with_controls(controls),
            );
            let session = session(&keymap, &config);

            let core = LightingMailbox::<StandardCommand<2, 4>, StandardReply, StandardError, 1>::new();
            let mut adapter = super::super::super::lighting::StandardRynkLightingAdapter::<2, 1, 4>::new(
                &mailbox,
                &core,
                descriptor().topology,
            );
            let mut engine: StandardLightingEngine<'static, EmptySource, EmptySource, 2, 2, 4> =
                StandardLightingEngine::new(
                    BackgroundState::default(),
                    LayerScenes {
                        scenes: &COMPILED_LAYERS,
                        policy: LayerPolicy::ActiveStack,
                    },
                    EmptySource,
                    EmptySource,
                )
                .with_controls(controls);

            let lighting = capabilities(service.lighting.unwrap());
            assert!(lighting.features.contains(LightingFeatureFlags::LAYER_SCENES));
            assert!(lighting.features.contains(LightingFeatureFlags::OVERLAY_READBACK));
            assert!(lighting.features.contains(LightingFeatureFlags::COMPILED_LAYER_SCENES));
            assert!(
                lighting
                    .features
                    .contains(LightingFeatureFlags::COMPILED_CONDITIONAL_SCENES)
            );
            assert!(lighting.features.contains(LightingFeatureFlags::OUTPUT_MODE));

            let client = async {
                let compiled_status = call::<GetLightingCompiledSceneStatus>(&service, &session, &())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(compiled_status.topology_revision, 7);
                assert_eq!(compiled_status.scene_len, 2);
                assert_eq!(compiled_status.policy, LightingLayerPolicy::ActiveStack);
                let compiled = call::<GetLightingCompiledScenes>(
                    &service,
                    &session,
                    &LightingPageRequest {
                        topology_revision: 7,
                        offset: 0,
                    },
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(compiled.total_count, 2);
                assert_eq!(compiled.items[0].layer, 0);
                assert_eq!(compiled.items[0].led_id, LightingLedId(10));
                assert_eq!(compiled.items[1].led_id, LightingLedId(42));
                let conditional_status = call::<GetLightingConditionalSceneStatus>(&service, &session, &())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(conditional_status.topology_revision, 7);
                assert_eq!(conditional_status.cell_len, 1);
                assert_eq!(conditional_status.controls.output_toggle_user_action, Some(13));
                assert_eq!(conditional_status.controls.wake_layer, Some(1));
                let output_mode = call::<GetLightingOutputMode>(&service, &session, &())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    output_mode.mode,
                    rmk_types::protocol::rynk::LightingOutputMode::PoweredOnly
                );
                assert_eq!(output_mode.cycle_user_action, Some(14));
                assert_eq!(output_mode.wake_layer, Some(1));
                assert_eq!(output_mode.indicator.unwrap().led_id, LightingLedId(10));
                let conditional = call::<GetLightingConditionalScenes>(
                    &service,
                    &session,
                    &LightingPageRequest {
                        topology_revision: 7,
                        offset: 0,
                    },
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(conditional.total_count, 1);
                assert_eq!(conditional.items[0].led_id, LightingLedId(10));
                assert_eq!(conditional.items[0].conditions.layer.unwrap().layer, 1);
                assert_eq!(conditional.items[0].conditions.battery.unwrap().min_level, Some(21));
                let stale_compiled = call::<GetLightingCompiledScenes>(
                    &service,
                    &session,
                    &LightingPageRequest {
                        topology_revision: 6,
                        offset: 0,
                    },
                )
                .await
                .unwrap();
                assert_eq!(
                    stale_compiled,
                    Err(LightingError::TopologyRevisionConflict {
                        expected: 6,
                        current: 7
                    })
                );

                let status = call::<GetLightingSceneStatus>(&service, &session, &())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(status.capacity, 4);
                assert_eq!(status.scene_len, 0);
                assert_eq!(status.policy, LightingLayerPolicy::ActiveStack);
                assert_eq!(
                    status.chunk_capacity as usize,
                    rmk_types::protocol::rynk::LIGHTING_SCENE_CHUNK_SIZE
                );

                // Handler-side bounds: an out-of-keymap layer and an unknown
                // stable LED never reach the engine.
                let bad_layer = call::<SetLightingSceneCell>(
                    &service,
                    &session,
                    &SetLightingSceneCellRequest {
                        expected_revision: 0,
                        cell: wire_scene_cell(9, 10),
                    },
                )
                .await
                .unwrap();
                assert_eq!(bad_layer, Err(LightingError::UnknownLayer { layer: 9 }));
                let bad_led = call::<SetLightingSceneCell>(
                    &service,
                    &session,
                    &SetLightingSceneCellRequest {
                        expected_revision: 0,
                        cell: wire_scene_cell(1, 7),
                    },
                )
                .await
                .unwrap();
                assert_eq!(
                    bad_led,
                    Err(LightingError::UnknownLed {
                        led_id: LightingLedId(7)
                    })
                );

                let state = call::<SetLightingSceneCell>(
                    &service,
                    &session,
                    &SetLightingSceneCellRequest {
                        expected_revision: 0,
                        cell: wire_scene_cell(1, 42),
                    },
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(state.revision, 1);

                // Pinned page reads round-trip stable LED identity.
                let page =
                    call::<GetLightingScenes>(&service, &session, &LightingScenePageRequest { revision: 1, offset: 0 })
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(page.total_count, 1);
                assert_eq!(page.items[0], wire_scene_cell(1, 42));
                let stale =
                    call::<GetLightingScenes>(&service, &session, &LightingScenePageRequest { revision: 0, offset: 0 })
                        .await
                        .unwrap();
                assert_eq!(
                    stale,
                    Err(LightingError::StateRevisionConflict {
                        expected: 0,
                        current: 1
                    })
                );

                // A whole-table replacement above capacity fails locally.
                let over = call::<BeginLightingSceneReplace>(
                    &service,
                    &session,
                    &BeginLightingSceneReplaceRequest {
                        expected_revision: 1,
                        cell_count: 9,
                    },
                )
                .await
                .unwrap();
                assert_eq!(over, Err(LightingError::SceneFull { capacity: 4 }));

                let transaction = call::<BeginLightingSceneReplace>(
                    &service,
                    &session,
                    &BeginLightingSceneReplaceRequest {
                        expected_revision: 1,
                        cell_count: 1,
                    },
                )
                .await
                .unwrap()
                .unwrap();
                let mut cells = Vec::new();
                cells.push(wire_scene_cell(0, 10)).unwrap();
                call::<PutLightingSceneChunk>(
                    &service,
                    &session,
                    &PutLightingSceneChunkRequest {
                        transaction_id: transaction.id,
                        offset: 0,
                        cells,
                    },
                )
                .await
                .unwrap()
                .unwrap();
                let committed = call::<CommitLightingSceneReplace>(
                    &service,
                    &session,
                    &CommitLightingSceneReplaceRequest {
                        transaction_id: transaction.id,
                    },
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(committed.revision, 2);

                // The pre-replace cell is gone; the replacement is visible.
                let page =
                    call::<GetLightingScenes>(&service, &session, &LightingScenePageRequest { revision: 2, offset: 0 })
                        .await
                        .unwrap()
                        .unwrap();
                assert_eq!(page.total_count, 1);
                assert_eq!(page.items[0], wire_scene_cell(0, 10));

                let state = call::<SetLightingLayerPolicy>(
                    &service,
                    &session,
                    &SetLightingLayerPolicyRequest {
                        expected_revision: 2,
                        policy: LightingLayerPolicy::EffectiveOnly,
                    },
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(state.revision, 3);
                let status = call::<GetLightingSceneStatus>(&service, &session, &())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(status.policy, LightingLayerPolicy::EffectiveOnly);
                assert_eq!(status.scene_len, 1);
                let compiled_status = call::<GetLightingCompiledSceneStatus>(&service, &session, &())
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    compiled_status.policy,
                    LightingLayerPolicy::ActiveStack,
                    "runtime policy mutations do not change the compiled source policy"
                );

                // Unset through stable identity, then abort of a dead
                // transaction reports it as unknown.
                let state = call::<UnsetLightingSceneCell>(
                    &service,
                    &session,
                    &UnsetLightingSceneCellRequest {
                        expected_revision: 3,
                        layer: 0,
                        led_id: LightingLedId(10),
                    },
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(state.revision, 4);
                let aborted = call::<AbortLightingSceneReplace>(
                    &service,
                    &session,
                    &rmk_types::protocol::rynk::AbortLightingSceneReplaceRequest { transaction_id: 999 },
                )
                .await
                .unwrap();
                assert_eq!(aborted, Err(LightingError::InvalidTransaction));

                let mut transient = overlay_cell(42);
                transient.ttl_ms = Some(5_000);
                let state = call::<SetLightingOverlay>(
                    &service,
                    &session,
                    &SetLightingOverlayRequest {
                        expected_revision: 4,
                        cell: transient,
                    },
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(state.revision, 5);
                let overlay = call::<GetLightingOverlay>(
                    &service,
                    &session,
                    &LightingOverlayPageRequest { revision: 5, offset: 0 },
                )
                .await
                .unwrap()
                .unwrap();
                assert_eq!(overlay.total_count, 1);
                assert_eq!(overlay.items[0], transient);
                let stale_overlay = call::<GetLightingOverlay>(
                    &service,
                    &session,
                    &LightingOverlayPageRequest { revision: 4, offset: 0 },
                )
                .await
                .unwrap();
                assert_eq!(
                    stale_overlay,
                    Err(LightingError::StateRevisionConflict {
                        expected: 4,
                        current: 5
                    })
                );
            };

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
            #[cfg(feature = "storage")]
            let persisted = core::cell::RefCell::new(alloc::vec::Vec::new());
            let flash_drain = async {
                #[cfg(feature = "storage")]
                loop {
                    persisted
                        .borrow_mut()
                        .push(crate::channel::FLASH_CHANNEL.receive().await);
                }
                #[cfg(not(feature = "storage"))]
                core::future::pending::<()>().await
            };

            match select3(
                client,
                adapter_loop,
                embassy_futures::join::join(engine_loop, flash_drain),
            )
            .await
            {
                Either3::First(()) => {}
                _ => panic!("service loops must not finish"),
            }

            // Every scene mutation rewrote the durable table: the last
            // header reflects the final one-cell removal and policy change.
            #[cfg(feature = "storage")]
            {
                use crate::storage::FlashOperationMessage;
                let persisted = persisted.into_inner();
                assert!(!persisted.is_empty());
                let last_table = persisted
                    .iter()
                    .rev()
                    .find_map(|message| match message {
                        FlashOperationMessage::LightingSceneTable { len, policy } => Some((*len, *policy)),
                        _ => None,
                    })
                    .expect("scene mutations persist a table header");
                assert_eq!(last_table, (0, LightingLayerPolicy::EffectiveOnly));
                assert!(persisted.iter().any(|message| matches!(
                    message,
                    FlashOperationMessage::LightingSceneShard { index: 0, cells } if cells.len() == 1
                )));
            }
        });
    }
}
