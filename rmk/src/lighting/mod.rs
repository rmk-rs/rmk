//! Hardware- and protocol-independent lighting primitives.
//!
//! The module separates stable semantic LED identity, local frame slots, and
//! physical routing. Rendering uses caller-provided fixed storage and performs
//! no allocation, I/O, sleeping, or protocol handling.

use embassy_sync::channel::Channel;
use rmk_types::action::LightAction;

use crate::RawMutex;

pub mod color;
pub mod compositor;
pub mod context;
pub mod effect;
pub mod output;
pub mod processor;
pub mod rmk_state;
pub mod selector;
pub mod service;
pub mod source;
pub mod standard;
pub mod topology;

pub use color::Rgb8;
pub use compositor::{Compositor, LogicalFrame, RenderError, RenderResult, RenderTransaction};
pub use context::{IndicatorState, LayerState, LightingContext, LightingContextProvider};
pub use effect::{BuiltinEffect, EffectSample, LightingEffect};
pub use output::{
    BrightnessTransform, OutputSelection, RouteError, RoutedFrameSink, RoutedPixel, ValidatedRouting, VisitSummary,
};
pub use processor::{LightingMailbox, LightingProcessor};
pub use rmk_state::{KeymapLightingState, TooManyLayers};
pub use selector::{LedSelector, ResolveError, ResolvedTargets};
pub use service::{
    CommandResult, Invalidation, LightingEngine, LightingOutput, LightingService, OutputCompletion, OutputOperation,
    OutputState, PowerState, RenderOutcome, ServiceAction, SnapshotProvider,
};
pub use source::{
    BatteryCondition, BatteryStatusProvider, ChargeCondition, ConditionSet, ConditionalSceneCell, ConditionalScenes,
    DenseSource, Indicator, IndicatorScene, IndicatorScenes, LayerCondition, LayerPolicy, LayerScene, LayerScenes,
    LightingControls, OutputMode, OutputModeIndicator, OverlayError, OverlayUpdate, SceneCell, SparseScene, TtlOverlay,
};
pub use standard::{
    BackgroundMode, BackgroundPatch, BackgroundState, CompiledScenePage, EmptySource, OVERLAY_CHUNK_SIZE, OverlayBatch,
    OverlayCell, OverlayPage, ReplicaSlotError, SCENE_CHUNK_SIZE, SCENE_TRANSACTION_TIMEOUT_MS, SceneChunk, ScenePage,
    SceneTable, SceneTableCell, StandardCommand, StandardError, StandardInput, StandardLightingEngine,
    StandardMutableState, StandardReplicaSlot, StandardReplicaState, StandardReply, StandardState, UniformBackground,
};
pub use topology::*;

/// Dedicated lossless path for edge-sensitive lighting key actions. State
/// notifications may be coalesced, but brightness/toggle/mode presses may not.
static LIGHT_ACTIONS: Channel<RawMutex, LightAction, 4> = Channel::new();

/// Deliver a lighting action to the active lighting processor.
///
/// This is public so board-specific user actions can reuse the same ordered,
/// lossless path as keymap-native lighting actions.
pub async fn send_light_action(action: LightAction) {
    LIGHT_ACTIONS.send(action).await;
}

async fn next_light_action() -> LightAction {
    LIGHT_ACTIONS.receive().await
}
