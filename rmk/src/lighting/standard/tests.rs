use core::num::NonZeroU32;

use rmk_types::action::LightAction;

use super::*;
use crate::lighting::compositor::{
    Contribution, ExtensionDescriptor, ExtensionParamSpec, ExtensionState, LightingSource, LogicalFrame,
    RenderInput as SourceRenderInput,
};
use crate::lighting::effect::BuiltinEffect;
use crate::lighting::service::{Invalidation, LightingEngine, RenderInput};
use crate::lighting::source::{
    BatteryStatusProvider, ConditionSet, LayerScenes, LightingControls, OutputMode, OverlayError, SparseScene,
};
use crate::lighting::topology::LedSlot;
use crate::lighting::{LayerPolicy, LayerScene, LayerState, LightingContext, Rgb8, SceneCell};

const RED: Rgb8 = Rgb8::new(200, 0, 0);
const GREEN: Rgb8 = Rgb8::new(0, 200, 0);

type Engine = StandardLightingEngine<'static, EmptySource, EmptySource, 2, 2>;

/// Effect 0 advertises two parameters; effect 1 advertises none, so the
/// "active effect has nothing to replicate" path stays covered.
static REPLICA_EFFECT_NAMES: &[&str] = &["A", "B"];
static REPLICA_EFFECT_PARAMS: &[&[ExtensionParamSpec]] = &[&[
    ExtensionParamSpec {
        name: "Density",
        min: 1,
        max: 8,
        default: 3,
    },
    ExtensionParamSpec {
        name: "Fade",
        min: 0,
        max: 255,
        default: 128,
    },
]];

#[derive(Copy, Clone)]
struct ReplicaExtension {
    state: ExtensionState,
    accept: bool,
    params: [u8; 2],
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

    fn extension_descriptor(&self) -> Option<ExtensionDescriptor> {
        Some(ExtensionDescriptor {
            effects: REPLICA_EFFECT_NAMES,
            palettes: &[],
            params: REPLICA_EFFECT_PARAMS,
        })
    }

    fn extension_param(&self, effect: u8, index: u8) -> Option<u8> {
        if effect != 0 {
            return None;
        }
        self.params.get(index as usize).copied()
    }

    fn apply_extension_param(&mut self, effect: u8, index: u8, value: u8) -> bool {
        if !self.accept || effect != 0 {
            return false;
        }
        match self.params.get_mut(index as usize) {
            Some(slot) => {
                *slot = value;
                true
            }
            None => false,
        }
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
            params: [3, 128],
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
    assert_eq!(snapshot.scenes.iter().collect::<heapless::Vec<_, 4>>(), &[scene_cell]);

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
    assert_eq!(replica.scenes().iter().collect::<heapless::Vec<_, 4>>(), &[scene_cell]);
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
        runtime_conditional_scenes: RuntimeConditionalSceneTable::new(),
        context: context(0),
        sample_time_ms: 9,
        extension: None,
        extension_params: None,
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
        runtime_conditional_scenes: RuntimeConditionalSceneTable::new(),
        context: context(0),
        sample_time_ms: 50,
        extension: Some(next_extension),
        extension_params: None,
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
fn replica_export_carries_the_active_effect_parameters() {
    let mut authority = replica_engine(true);
    authority
        .handle_command(
            0,
            StandardCommand::SetExtensionParamIfRevision {
                expected_revision: 0,
                effect: 0,
                index: 1,
                value: 200,
            },
            &context(0),
        )
        .unwrap();

    static EXPORT_SLOT: StandardReplicaSlot<2> = StandardReplicaSlot::new();
    authority
        .handle_command(10, StandardCommand::ExportReplica(&EXPORT_SLOT), &context(0))
        .unwrap();
    let snapshot = EXPORT_SLOT.take().unwrap();
    let params = snapshot.extension_params.expect("active effect has parameters");
    assert_eq!(params.effect, 0);
    assert_eq!(params.values(), &[3, 200]);

    // An active effect without parameters exports nothing to replicate.
    authority
        .handle_command(
            20,
            StandardCommand::SetExtensionIfRevision {
                expected_revision: authority.state().revision,
                state: ExtensionState {
                    effect: 1,
                    palette: 0,
                    value: 10,
                    speed: 20,
                },
            },
            &context(0),
        )
        .unwrap();
    authority
        .handle_command(30, StandardCommand::ExportReplica(&EXPORT_SLOT), &context(0))
        .unwrap();
    assert_eq!(EXPORT_SLOT.take().unwrap().extension_params, None);
}

#[test]
fn replica_application_installs_exported_parameters() {
    let mut authority = replica_engine(true);
    authority
        .handle_command(
            0,
            StandardCommand::SetExtensionParamIfRevision {
                expected_revision: 0,
                effect: 0,
                index: 0,
                value: 7,
            },
            &context(0),
        )
        .unwrap();
    static ROUND_TRIP_SLOT: StandardReplicaSlot<2> = StandardReplicaSlot::new();
    authority
        .handle_command(10, StandardCommand::ExportReplica(&ROUND_TRIP_SLOT), &context(0))
        .unwrap();
    let snapshot = ROUND_TRIP_SLOT.take().unwrap();

    let mut replica = replica_engine(true);
    assert_eq!(replica.extension().params, [3, 128]);
    ROUND_TRIP_SLOT.put(snapshot).unwrap();
    replica
        .handle_command(100, StandardCommand::ApplyReplica(&ROUND_TRIP_SLOT), &context(0))
        .unwrap();
    assert_eq!(replica.extension().params, [7, 128]);
    assert_eq!(replica.state().revision, snapshot.revision);
}

#[test]
fn replica_parameters_are_validated_like_a_host_set() {
    let mut authority = replica_engine(true);
    static SEED_SLOT: StandardReplicaSlot<2> = StandardReplicaSlot::new();
    authority
        .handle_command(0, StandardCommand::ExportReplica(&SEED_SLOT), &context(0))
        .unwrap();
    let mut snapshot = SEED_SLOT.take().unwrap();
    // A selection change that must not survive the rejected parameter.
    snapshot.extension = Some(ExtensionState {
        effect: 0,
        palette: 7,
        value: 77,
        speed: 88,
    });
    // 9 is outside the "Density" spec's 1..=8 range.
    snapshot.extension_params = Some(ExtensionReplicaParams {
        effect: 0,
        len: 1,
        values: [9, 0, 0, 0, 0, 0, 0, 0],
    });

    let mut replica = replica_engine(true);
    let before = replica.extension().params;
    let before_state = replica.extension().state;
    static INVALID_PARAM_SLOT: StandardReplicaSlot<2> = StandardReplicaSlot::new();
    INVALID_PARAM_SLOT.put(snapshot).unwrap();
    assert_eq!(
        replica.handle_command(100, StandardCommand::ApplyReplica(&INVALID_PARAM_SLOT), &context(0)),
        Err(StandardError::ExtensionUnsupported)
    );
    // The pre-apply check keeps the whole extension untouched, selection
    // included, not just the parameter that failed.
    assert_eq!(replica.extension().params, before);
    assert_eq!(replica.extension().state, before_state);
    assert_eq!(replica.state().revision, 0);
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
        runtime_conditional_scenes: RuntimeConditionalSceneTable::new(),
        context: context(0),
        sample_time_ms: 50,
        extension: Some(ExtensionState {
            effect: 1,
            palette: 2,
            value: 30,
            speed: 40,
        }),
        extension_params: None,
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

    let mut engine: StandardLightingEngine<'static, ClaimingSource, EmptySource, 2, 2> = StandardLightingEngine::new(
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

#[test]
fn runtime_conditional_replace_preserves_order_and_output_mode_is_revision_checked() {
    use rmk_types::battery::{BatteryStatus, ChargeState};

    use crate::lighting::{BatteryCondition, ChargeCondition, LayerCondition};

    struct Batteries;
    impl BatteryStatusProvider for Batteries {
        fn battery_status(&self, node: u8) -> BatteryStatus {
            if node == 1 {
                BatteryStatus::Available {
                    charge_state: ChargeState::Discharging,
                    level: Some(50),
                }
            } else {
                BatteryStatus::Unavailable
            }
        }
    }
    static BATTERIES: Batteries = Batteries;

    let mut engine = scene_engine().with_battery_status_provider(&BATTERIES);
    let snapshot = context(0);
    let transaction_id = match engine
        .handle_command(
            0,
            StandardCommand::BeginRuntimeConditionalSceneReplace {
                expected_revision: 0,
                cell_count: 2,
            },
            &snapshot,
        )
        .unwrap()
        .reply
    {
        StandardReply::RuntimeConditionalSceneTransaction { id, .. } => id,
        other => panic!("expected runtime conditional transaction, got {other:?}"),
    };
    let mut cells = RuntimeConditionalSceneChunk::new();
    cells
        .push(RuntimeConditionalSceneCell {
            conditions: ConditionSet {
                layer: None,
                battery: Some(BatteryCondition {
                    node: 1,
                    min_level: Some(25),
                    max_level: Some(75),
                    charge: ChargeCondition::Discharging,
                }),
            },
            slot: LedSlot(1),
            effect: BuiltinEffect::Solid { color: RED },
        })
        .unwrap();
    cells
        .push(RuntimeConditionalSceneCell {
            conditions: ConditionSet {
                layer: Some(LayerCondition { layer: 0, active: true }),
                battery: None,
            },
            slot: LedSlot(1),
            effect: BuiltinEffect::Solid { color: GREEN },
        })
        .unwrap();
    engine
        .handle_command(
            1,
            StandardCommand::PutRuntimeConditionalSceneChunk {
                transaction_id,
                offset: 0,
                cells,
            },
            &snapshot,
        )
        .unwrap();
    let committed = engine
        .handle_command(
            2,
            StandardCommand::CommitRuntimeConditionalSceneReplace { transaction_id },
            &snapshot,
        )
        .unwrap();
    assert_eq!(committed.reply.state().unwrap().revision, 1);
    assert_eq!(engine.runtime_conditional_scenes().as_slice(), cells.as_slice());

    let mut frame = LogicalFrame::new(Rgb8::BLACK);
    engine
        .render(
            RenderInput {
                now_ms: 2,
                snapshot: &snapshot,
            },
            &mut frame,
        )
        .unwrap();
    assert_eq!(frame.as_slice()[1], GREEN, "later matching rules win");

    assert_eq!(
        engine.handle_command(
            3,
            StandardCommand::SetOutputModeIfRevision {
                expected_revision: 0,
                mode: OutputMode::PoweredOnly,
            },
            &snapshot,
        ),
        Err(StandardError::RevisionConflict {
            expected: 0,
            current: 1,
        })
    );
    let set = engine
        .handle_command(
            3,
            StandardCommand::SetOutputModeIfRevision {
                expected_revision: 1,
                mode: OutputMode::PoweredOnly,
            },
            &snapshot,
        )
        .unwrap();
    assert_eq!(set.reply.state().unwrap().output_mode, OutputMode::PoweredOnly);
    assert_eq!(set.reply.state().unwrap().revision, 2);
}

/// Interning is what shrinks a cell from 20 bytes to 4: many keys painted
/// from few colours share one entry, and the effect's type stops mattering
/// to the cell, so blink and breathe need no separate table.
#[test]
fn interning_shares_one_entry_per_distinct_effect() {
    let mut pool = StylePool::new();
    let blue = BuiltinEffect::solid(Rgb8::new(0, 0, 9));
    let green = BuiltinEffect::solid(Rgb8::new(0, 9, 0));

    let first = pool.intern(blue).unwrap();
    assert_eq!(pool.intern(blue), Some(first), "same effect, same entry");
    let other = pool.intern(green).unwrap();
    assert_ne!(first, other);
    assert_eq!(pool.len(), 2);
    assert_eq!(pool.get(first), Some(blue));
    assert_eq!(pool.get(other), Some(green));
}

#[test]
fn a_full_pool_declines_only_new_effects() {
    let mut pool = StylePool::new();
    for shade in 0..SCENE_STYLE_CAP {
        pool.intern(BuiltinEffect::solid(Rgb8::new(shade as u8, 0, 0))).unwrap();
    }
    assert_eq!(pool.len(), SCENE_STYLE_CAP);

    // Already held: still fine, however full.
    let held = BuiltinEffect::solid(Rgb8::new(0, 0, 0));
    assert!(pool.intern(held).is_some());

    // New: declined, and declining mutates nothing.
    let before = pool;
    assert_eq!(pool.intern(BuiltinEffect::solid(Rgb8::new(0, 1, 2))), None);
    assert_eq!(pool, before);
}

/// A style goes when nothing points at it, found by scanning rather than by
/// keeping counts -- and it must survive while another cell still holds it.
#[test]
fn a_style_outlives_all_but_its_last_reference() {
    let mut table = SceneTable::<4>::new();
    let red = BuiltinEffect::solid(Rgb8::new(9, 0, 0));
    let cell = |slot| SceneTableCell {
        layer: 0,
        slot: LedSlot(slot),
        effect: red,
    };
    table.set(cell(0)).unwrap();
    table.set(cell(1)).unwrap();
    assert_eq!((table.len(), table.pool.len()), (2, 1));

    assert!(table.unset(0, LedSlot(0)));
    assert_eq!(table.pool.len(), 1, "the other cell still holds it");

    assert!(table.unset(0, LedSlot(1)));
    assert_eq!((table.len(), table.pool.len()), (0, 0));
}

/// Repainting one key is what painting in a configurator does, and it must
/// not strand an entry each time: 64 repaints would exhaust the pool.
#[test]
fn repainting_a_cell_does_not_strand_styles() {
    let mut table = SceneTable::<4>::new();
    for shade in 0..(SCENE_STYLE_CAP as u8 * 3) {
        table
            .set(SceneTableCell {
                layer: 0,
                slot: LedSlot(0),
                effect: BuiltinEffect::solid(Rgb8::new(shade, shade, shade)),
            })
            .expect("overwriting one cell never needs a second entry");
        assert_eq!(table.pool.len(), 1, "the previous shade was freed");
    }
    assert_eq!(table.len(), 1);
}

#[test]
fn runtime_conditional_cells_outrank_layer_scenes_and_compiled_conditional_rules() {
    use rmk_types::battery::BatteryStatus;

    use crate::lighting::{ConditionalSceneCell, ConditionalScenes, LayerCondition};

    struct NoBatteries;
    impl BatteryStatusProvider for NoBatteries {
        fn battery_status(&self, _node: u8) -> BatteryStatus {
            BatteryStatus::Unavailable
        }
    }
    static NO_BATTERIES: NoBatteries = NoBatteries;

    // Slot 0 is contested three ways: a compiled layer scene paints it RED
    // on layer 1, a compiled conditional rule paints it GREEN, and the
    // runtime table paints it BLUE. Only the host's rule should survive.
    static COMPILED_CONDITIONAL: [ConditionalSceneCell<BuiltinEffect>; 1] = [ConditionalSceneCell {
        conditions: ConditionSet {
            layer: Some(LayerCondition { layer: 1, active: true }),
            battery: None,
        },
        slot: LedSlot(0),
        effect: BuiltinEffect::Solid { color: GREEN },
    }];

    let mut engine: StandardLightingEngine<
        'static,
        EmptySource,
        ConditionalScenes<'static, BuiltinEffect, NoBatteries>,
        2,
        2,
        4,
    > = StandardLightingEngine::new(
        BackgroundState {
            value: 10,
            ..BackgroundState::default()
        },
        LayerScenes {
            scenes: &LAYERS,
            policy: LayerPolicy::ActiveStack,
        },
        EmptySource,
        ConditionalScenes::new(&COMPILED_CONDITIONAL, &NO_BATTERIES),
    )
    .with_battery_status_provider(&NO_BATTERIES);

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
    assert_eq!(
        frame.as_slice()[0],
        GREEN,
        "compiled conditional rules outrank compiled layer scenes"
    );

    engine
        .install_runtime_conditional_scene_cell(RuntimeConditionalSceneCell {
            conditions: ConditionSet {
                layer: Some(LayerCondition { layer: 1, active: true }),
                battery: None,
            },
            slot: LedSlot(0),
            effect: BuiltinEffect::Solid { color: BLUE },
        })
        .unwrap();
    let mut frame = LogicalFrame::new(Rgb8::BLACK);
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
        frame.as_slice()[0],
        BLUE,
        "a runtime conditional cell replaces the compiled rule on the same slot"
    );
}
