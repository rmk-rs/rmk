//! Wasm-facing Rynk client handle.
//!
//! JS owns the byte link and hands it to [`connect`], which runs the Rynk
//! handshake and returns a [`RynkClient`] wrapping the session's `Client` and
//! `Driver`. Every method is `&self`, so JS holds a parked
//! [`RynkClient::next_topic`] loop while issuing requests — the same
//! full-duplex contract the native transports get from one session `select`.
//! With no resident task to pump the driver, the in-flight calls elect one:
//! see `RynkClient::drive` for the mechanism.

use core::pin::pin;

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use rynk::rmk_types::action::{EncoderAction, KeyAction};
use rynk::rmk_types::battery::BatteryStatus;
use rynk::rmk_types::ble::BleStatus;
use rynk::rmk_types::combo::Combo;
use rynk::rmk_types::connection::{ConnectionStatus, ConnectionType};
use rynk::rmk_types::fork::Fork;
use rynk::rmk_types::led_indicator::LedIndicator;
use rynk::rmk_types::modifier::ModifierCombination;
use rynk::rmk_types::morse::Morse;
use rynk::rmk_types::protocol::rynk::{
    AbortLightingOverlayReplaceRequest, BeginLightingOverlayReplaceRequest, BehaviorConfig,
    ClearLightingOverlayRequest, CommitLightingOverlayReplaceRequest, DeviceCapabilities, DeviceInfo,
    GetComboBulkResponse, GetKeymapBulkResponse, GetMorseBulkResponse, LightingCapabilities, LightingKeysPage,
    LightingLedsPage, LightingOutputsPage, LightingOverlayTransaction, LightingPageRequest, LightingPhysicalKeysPage,
    LightingRoutesPage, LightingState, LightingZoneMembershipsPage, LightingZonesPage, LockStatus, MacroData,
    MatrixState, PeripheralStatus, ProtocolVersion, PutLightingOverlayChunkRequest, SetComboBulkRequest,
    SetKeymapBulkRequest, SetLightingOverlayRequest, SetLightingStateRequest, SetMorseBulkRequest, StorageResetMode,
    UnsetLightingOverlayRequest, BuildInfo, AbortLightingSceneReplaceRequest, BeginLightingSceneReplaceRequest,
    CommitLightingSceneReplaceRequest, LightingScenePageRequest, LightingSceneStatus, LightingSceneTransaction,
    LightingScenesPage, PutLightingSceneChunkRequest, SetLightingLayerPolicyRequest, SetLightingSceneCellRequest,
    UnsetLightingSceneCellRequest, LayerState, LightingCompiledSceneStatus, LightingCompiledScenesPage,
    LightingOverlayPage, LightingOverlayPageRequest, SplitCentralLatencyPolicy, SplitCentralLatencyState,
    LightingConditionalSceneStatus, LightingConditionalScenesPage, LightingOutputModeState, LightingExtension,
    LightingExtensionNamesPage, LightingExtensionNamesRequest, LightingExtensionParamsPage,
    LightingExtensionParamsRequest, SetLightingExtensionParamRequest, SetLightingExtensionStateRequest,
    AbortLightingRuntimeConditionalSceneReplaceRequest, BeginLightingRuntimeConditionalSceneReplaceRequest,
    CommitLightingRuntimeConditionalSceneReplaceRequest, LightingRuntimeConditionalScenePageRequest,
    LightingRuntimeConditionalSceneStatus, LightingRuntimeConditionalSceneTransaction,
    LightingRuntimeConditionalScenesPage, PutLightingRuntimeConditionalSceneChunkRequest,
    SetLightingOutputModeRequest,
};
use rynk::{Client, Driver, LayoutInfo, RynkDevice, RynkHostError, TopicEvent};
use wasm_bindgen::prelude::*;

use crate::transport::{JsByteLink, WasmReader, WasmWriter};

/// Live Rynk client handle exposed to JavaScript.
///
/// Wraps the session's `Client` + `Driver`. All methods are `&self`: a parked
/// `next_topic()` pull and up to four requests run concurrently (full-duplex),
/// with replies matched back by SEQ. Overlapping a `read_all_*` pager with a
/// bulk write is correct, just slower.
/// Dropping the handle, or closing the JS link, ends the session; the link
/// itself stays open until the page closes it.
#[wasm_bindgen]
pub struct RynkClient {
    client: Client,
    driver: Mutex<CriticalSectionRawMutex, Driver<WasmReader, WasmWriter>>,
}

/// Handshake over an already-open JS link and return a client. The link is the
/// web transport's [`RynkDevice`], so the browser path uses the same connect
/// lifecycle as the native serial/BLE transports.
#[wasm_bindgen]
pub async fn connect(link: JsByteLink) -> Result<RynkClient, JsValue> {
    let (client, driver) = link.connect().await?;
    Ok(RynkClient {
        client,
        driver: Mutex::new(driver),
    })
}

impl RynkClient {
    /// Run one client future full-duplex: race it against locking the driver.
    /// The lock winner pumps both directions for every parked call, and
    /// releasing the lock when its own future resolves hands the pump to a
    /// parked call. A dead link surfaces from the pump arm and reproduces for
    /// every later call — the closed transport keeps reporting EOF.
    async fn drive<T>(&self, fut: impl Future<Output = Result<T, RynkHostError>>) -> Result<T, JsValue> {
        let mut fut = pin!(fut);
        match select(self.driver.lock(), &mut fut).await {
            Either::Second(r) => r.map_err(Into::into),
            Either::First(mut driver) => match select(driver.run(&self.client), &mut fut).await {
                Either::First(err) => Err(err.into()),
                Either::Second(r) => r.map_err(Into::into),
            },
        }
    }
}

#[wasm_bindgen]
impl RynkClient {
    /// Pull the next recognized topic push (server→host). Parks until one
    /// arrives; rejects when the link dies. Unrecognized topics are skipped.
    /// JS drives this in a loop, like the native `next_topic()` pull, and it
    /// runs concurrently with the request methods.
    pub async fn next_topic(&self) -> Result<TopicEvent, JsValue> {
        self.drive(async { Ok(self.client.next_topic().await) }).await
    }
}

/// Generate the typed wasm request methods from the native client shape.
/// Arguments and responses are tsify wire types, so wasm-bindgen marshals them
/// across the ABI and emits a precise `.d.ts` (no `JsValue`/`any`). Errors
/// convert to a JS `Error` via `RynkHostError: Into<JsValue>`; a dead link
/// surfaces from the pump arm the same way.
macro_rules! endpoints {
    ($( $name:ident ( $($arg:ident : $arg_ty:ty),* ) -> $rty:ty ),* $(,)?) => {
        #[wasm_bindgen]
        impl RynkClient {
            $(
                pub async fn $name(&self, $($arg: $arg_ty),*) -> Result<$rty, JsValue> {
                    self.drive(self.client.$name($($arg),*)).await
                }
            )*
        }
    };
}

endpoints! {
    // system
    get_version() -> ProtocolVersion,
    get_capabilities() -> DeviceCapabilities,
    get_device_info() -> DeviceInfo,
    get_build_info() -> BuildInfo,
    reboot() -> (),
    bootloader_jump() -> (),
    peripheral_bootloader_jump(slot: u8) -> (),
    storage_reset(mode: StorageResetMode) -> (),
    // lock gate
    get_lock_status() -> LockStatus,
    unlock_poll() -> LockStatus,
    lock() -> (),
    // keymap
    get_key(layer: u8, row: u8, col: u8) -> KeyAction,
    set_key(layer: u8, row: u8, col: u8, action: KeyAction) -> (),
    get_default_layer() -> u8,
    set_default_layer(layer: u8) -> (),
    get_encoder(encoder_id: u8, layer: u8) -> EncoderAction,
    set_encoder(encoder_id: u8, layer: u8, action: EncoderAction) -> (),
    get_keymap_bulk(layer: u8, start_row: u8, start_col: u8) -> GetKeymapBulkResponse,
    set_keymap_bulk(request: SetKeymapBulkRequest) -> (),
    // whole-resource pagers
    read_all_keymap() -> Vec<KeyAction>,
    write_all_keymap(actions: Vec<KeyAction>) -> (),
    read_all_combos() -> Vec<Combo>,
    write_all_combos(configs: Vec<Combo>) -> (),
    read_all_morses() -> Vec<Morse>,
    write_all_morses(configs: Vec<Morse>) -> (),
    get_layout() -> LayoutInfo,
    // combos / forks / morse / macros
    get_combo(index: u8) -> Combo,
    set_combo(index: u8, config: Combo) -> (),
    get_combo_bulk(start_index: u8) -> GetComboBulkResponse,
    set_combo_bulk(request: SetComboBulkRequest) -> (),
    get_fork(index: u8) -> Fork,
    set_fork(index: u8, config: Fork) -> (),
    get_morse(index: u8) -> Morse,
    set_morse(index: u8, config: Morse) -> (),
    get_morse_bulk(start_index: u8) -> GetMorseBulkResponse,
    set_morse_bulk(request: SetMorseBulkRequest) -> (),
    get_macro(offset: u16) -> MacroData,
    set_macro(offset: u16, data: MacroData) -> (),
    // behavior
    get_behavior() -> BehaviorConfig,
    set_behavior(config: BehaviorConfig) -> (),
    get_split_central_latency() -> SplitCentralLatencyState,
    set_split_central_latency(policy: SplitCentralLatencyPolicy) -> SplitCentralLatencyState,
    // status
    get_current_layer() -> u8,
    get_layer_state() -> LayerState,
    get_modifier_state() -> ModifierCombination,
    get_matrix_state() -> MatrixState,
    get_battery_status() -> BatteryStatus,
    get_led_indicator() -> LedIndicator,
    get_peripheral_status(slot: u8) -> PeripheralStatus,
    get_wpm() -> u16,
    get_sleep_state() -> bool,
    // lighting
    get_lighting_capabilities() -> LightingCapabilities,
    get_lighting_state() -> LightingState,
    get_lighting_output_mode() -> LightingOutputModeState,
    set_lighting_output_mode(request: SetLightingOutputModeRequest) -> LightingOutputModeState,
    set_lighting_state(request: SetLightingStateRequest) -> LightingState,
    get_lighting_keys(request: LightingPageRequest) -> LightingKeysPage,
    get_lighting_physical_keys(request: LightingPageRequest) -> LightingPhysicalKeysPage,
    get_lighting_leds(request: LightingPageRequest) -> LightingLedsPage,
    get_lighting_zones(request: LightingPageRequest) -> LightingZonesPage,
    get_lighting_zone_memberships(request: LightingPageRequest) -> LightingZoneMembershipsPage,
    get_lighting_outputs(request: LightingPageRequest) -> LightingOutputsPage,
    get_lighting_routes(request: LightingPageRequest) -> LightingRoutesPage,
    set_lighting_overlay(request: SetLightingOverlayRequest) -> LightingState,
    unset_lighting_overlay(request: UnsetLightingOverlayRequest) -> LightingState,
    clear_lighting_overlay(request: ClearLightingOverlayRequest) -> LightingState,
    begin_lighting_overlay_replace(request: BeginLightingOverlayReplaceRequest) -> LightingOverlayTransaction,
    put_lighting_overlay_chunk(request: PutLightingOverlayChunkRequest) -> (),
    commit_lighting_overlay_replace(request: CommitLightingOverlayReplaceRequest) -> LightingState,
    abort_lighting_overlay_replace(request: AbortLightingOverlayReplaceRequest) -> (),
    get_lighting_overlay(request: LightingOverlayPageRequest) -> LightingOverlayPage,
    get_lighting_extension() -> LightingExtension,
    get_lighting_extension_names(request: LightingExtensionNamesRequest) -> LightingExtensionNamesPage,
    set_lighting_extension_state(request: SetLightingExtensionStateRequest) -> LightingState,
    get_lighting_extension_params(request: LightingExtensionParamsRequest) -> LightingExtensionParamsPage,
    set_lighting_extension_param(request: SetLightingExtensionParamRequest) -> LightingState,
    get_lighting_scene_status() -> LightingSceneStatus,
    get_lighting_scenes(request: LightingScenePageRequest) -> LightingScenesPage,
    get_lighting_compiled_scene_status() -> LightingCompiledSceneStatus,
    get_lighting_compiled_scenes(request: LightingPageRequest) -> LightingCompiledScenesPage,
    get_lighting_conditional_scene_status() -> LightingConditionalSceneStatus,
    get_lighting_conditional_scenes(request: LightingPageRequest) -> LightingConditionalScenesPage,
    get_lighting_runtime_conditional_scene_status() -> LightingRuntimeConditionalSceneStatus,
    get_lighting_runtime_conditional_scenes(request: LightingRuntimeConditionalScenePageRequest) -> LightingRuntimeConditionalScenesPage,
    begin_lighting_runtime_conditional_scene_replace(request: BeginLightingRuntimeConditionalSceneReplaceRequest) -> LightingRuntimeConditionalSceneTransaction,
    put_lighting_runtime_conditional_scene_chunk(request: PutLightingRuntimeConditionalSceneChunkRequest) -> (),
    commit_lighting_runtime_conditional_scene_replace(request: CommitLightingRuntimeConditionalSceneReplaceRequest) -> LightingState,
    abort_lighting_runtime_conditional_scene_replace(request: AbortLightingRuntimeConditionalSceneReplaceRequest) -> (),
    set_lighting_scene_cell(request: SetLightingSceneCellRequest) -> LightingState,
    unset_lighting_scene_cell(request: UnsetLightingSceneCellRequest) -> LightingState,
    set_lighting_layer_policy(request: SetLightingLayerPolicyRequest) -> LightingState,
    begin_lighting_scene_replace(request: BeginLightingSceneReplaceRequest) -> LightingSceneTransaction,
    put_lighting_scene_chunk(request: PutLightingSceneChunkRequest) -> (),
    commit_lighting_scene_replace(request: CommitLightingSceneReplaceRequest) -> LightingState,
    abort_lighting_scene_replace(request: AbortLightingSceneReplaceRequest) -> (),
    // connection
    get_connection_type() -> ConnectionType,
    get_connection_status() -> ConnectionStatus,
    get_ble_status() -> BleStatus,
    switch_ble_profile(slot: u8) -> (),
    clear_ble_profile(slot: u8) -> (),
}
