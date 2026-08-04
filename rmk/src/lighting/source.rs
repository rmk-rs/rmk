use rmk_types::ble::BleState;
use rmk_types::connection::ConnectionType;

use super::compositor::{Contribution, LightingSource, RenderInput};
use super::context::{LightingContext, LightingContextProvider};
use super::effect::{BuiltinEffect, EffectSample, LightingEffect};
use super::topology::LedSlot;
use crate::types::battery::{BatteryStatus, ChargeState};

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SceneCell<E> {
    pub slot: LedSlot,
    pub effect: E,
}

/// A pre-resolved sparse scene. Key, stable-ID, and zone selectors are
/// resolved to local slots when configuration is installed, not while
/// rendering.
#[derive(Copy, Clone, Debug)]
pub struct SparseScene<'a, E> {
    pub cells: &'a [SceneCell<E>],
}

impl<C, Context, E> LightingSource<C, Context> for SparseScene<'_, E>
where
    E: LightingEffect<C>,
{
    fn len(&self, _: &RenderInput<'_, Context>) -> usize {
        self.cells.len()
    }

    fn slot(&self, index: usize, _: &RenderInput<'_, Context>) -> LedSlot {
        self.cells[index].slot
    }

    fn contribution(&mut self, index: usize, input: &RenderInput<'_, Context>) -> Contribution<C> {
        Contribution::Opaque(self.cells[index].effect.sample(input.now_ms))
    }
}

#[derive(Copy, Clone, Debug)]
pub struct LayerScene<'a, E> {
    pub layer: u8,
    pub cells: &'a [SceneCell<E>],
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum LayerPolicy {
    /// Only the effective layer contributes.
    EffectiveOnly,
    /// Default first, then the complete active set in ascending RMK layer
    /// precedence, with the effective layer last. Sparse cells fall through.
    ActiveStack,
}

/// RMK's built-in, layer-aware sparse source.
#[derive(Copy, Clone, Debug)]
pub struct LayerScenes<'a, E> {
    pub scenes: &'a [LayerScene<'a, E>],
    pub policy: LayerPolicy,
}

impl<'a, E> LayerScenes<'a, E> {
    /// Number of board-compiled cells across every configured layer.
    pub fn cell_count(&self) -> usize {
        self.scenes.iter().map(|scene| scene.cells.len()).sum()
    }

    /// Iterate every board-compiled cell in declaration order while retaining
    /// the layer carried by its enclosing scene.
    pub fn cells(&self) -> impl Iterator<Item = (u8, &'a SceneCell<E>)> + '_ {
        self.scenes
            .iter()
            .flat_map(|scene| scene.cells.iter().map(move |cell| (scene.layer, cell)))
    }

    fn cell_for_layer(&self, layer: u8, wanted: &mut usize) -> Option<&SceneCell<E>> {
        for scene in self.scenes.iter().filter(|scene| scene.layer == layer) {
            if *wanted < scene.cells.len() {
                return Some(&scene.cells[*wanted]);
            }
            *wanted -= scene.cells.len();
        }
        None
    }

    fn cell_at(&self, context: &LightingContext, mut wanted: usize) -> &SceneCell<E> {
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
        self.scenes
            .iter()
            .filter(|scene| match self.policy {
                LayerPolicy::EffectiveOnly => scene.layer == effective,
                LayerPolicy::ActiveStack => {
                    scene.layer == context.layers.default
                        || scene.layer == effective
                        || context.layers.is_active(scene.layer)
                }
            })
            .map(|scene| scene.cells.len())
            .sum()
    }
}

impl<C, Context, E> LightingSource<C, Context> for LayerScenes<'_, E>
where
    E: LightingEffect<C>,
    Context: LightingContextProvider,
{
    fn len(&self, input: &RenderInput<'_, Context>) -> usize {
        self.included_len(input.context.lighting_context())
    }

    fn slot(&self, index: usize, input: &RenderInput<'_, Context>) -> LedSlot {
        self.cell_at(input.context.lighting_context(), index).slot
    }

    fn contribution(&mut self, index: usize, input: &RenderInput<'_, Context>) -> Contribution<C> {
        Contribution::Opaque(
            self.cell_at(input.context.lighting_context(), index)
                .effect
                .sample(input.now_ms),
        )
    }
}

/// Board-owned lookup used by declarative battery conditions. Node IDs match
/// lighting topology node IDs, so a split board can expose each half without
/// embedding board-specific left/right semantics in the renderer.
pub trait BatteryStatusProvider {
    fn battery_status(&self, node: u8) -> BatteryStatus;
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LayerCondition {
    pub layer: u8,
    pub active: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ChargeCondition {
    Any,
    Charging,
    Discharging,
    Unknown,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BatteryCondition {
    pub node: u8,
    pub min_level: Option<u8>,
    pub max_level: Option<u8>,
    pub charge: ChargeCondition,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ActiveTransport {
    Usb,
    Ble,
    NoneActive,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ConnectionCondition {
    pub transport: Option<ActiveTransport>,
    pub profile: Option<u8>,
    pub ble_state: Option<BleState>,
    /// Gate on a specific slot holding (or not holding) a stored bond,
    /// independent of which profile is active. This is what lets one rule per
    /// slot key say "paired" or "empty" for every slot at once.
    pub bonded: Option<BondedSlotCondition>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BondedSlotCondition {
    pub slot: u8,
    pub bonded: bool,
}

/// Satisfied when the extension band's enabled state equals `enabled`. An
/// extension is enabled when its [`ExtensionState::value`] is non-zero, which
/// is what `RgbTog` flips, so this is how a key can indicate whether effects
/// are currently rendering.
///
/// [`ExtensionState::value`]: crate::lighting::ExtensionState::value
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct EffectsCondition {
    pub enabled: bool,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct ConditionSet {
    pub layer: Option<LayerCondition>,
    pub battery: Option<BatteryCondition>,
    pub connection: Option<ConnectionCondition>,
    /// Satisfied when the live output-mode policy equals this. Sources that
    /// cannot observe the policy treat it as unsatisfiable rather than true,
    /// so a rule gated on it never fires where it cannot be evaluated.
    pub output_mode: Option<OutputMode>,
    /// Satisfied when the extension band's enabled state matches. Sources
    /// without a view of the extension treat it as unsatisfiable, on the same
    /// grounds as `output_mode`.
    pub effects: Option<EffectsCondition>,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum OutputMode {
    #[default]
    AlwaysOn,
    AlwaysOff,
    PoweredOnly,
}

/// Which power source a split renderer uses for `PoweredOnly`.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub enum PoweredOnlyScope {
    /// Follow the authoritative renderer's power state on every node.
    #[default]
    Authority,
    /// Let each renderer use its own node's local VBUS state.
    Local,
}

impl OutputMode {
    pub const fn next(self) -> Self {
        match self {
            Self::AlwaysOn => Self::AlwaysOff,
            Self::AlwaysOff => Self::PoweredOnly,
            Self::PoweredOnly => Self::AlwaysOn,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OutputModeIndicator {
    pub slot: LedSlot,
    pub always_on: BuiltinEffect,
    pub always_off: BuiltinEffect,
    pub powered_only: BuiltinEffect,
}

impl OutputModeIndicator {
    pub const fn effect(self, mode: OutputMode) -> BuiltinEffect {
        match mode {
            OutputMode::AlwaysOn => self.always_on,
            OutputMode::AlwaysOff => self.always_off,
            OutputMode::PoweredOnly => self.powered_only,
        }
    }
}

/// Optional board controls compiled from `[lighting.controls]`.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct LightingControls {
    /// Legacy two-state action retained for existing boards.
    pub output_toggle_user_action: Option<u8>,
    pub output_mode_cycle_user_action: Option<u8>,
    /// Layers that temporarily wake lighting while held, as a bitmask over
    /// [`LayerState`](crate::lighting::LayerState)'s layers. A set rather than
    /// one layer, so waking is not limited to a single "magic" layer, and a
    /// mask rather than a list because the active layers are already one --
    /// the check is a single AND. Zero means no layer wakes lighting.
    pub wake_layers: u64,
    pub initial_output_mode: OutputMode,
    pub powered_only_scope: PoweredOnlyScope,
    pub output_mode_indicator: Option<OutputModeIndicator>,
}

impl ConditionSet {
    pub(crate) fn matches<Context, Batteries>(
        &self,
        context: &Context,
        batteries: &Batteries,
        output_mode: Option<OutputMode>,
        effects_enabled: Option<bool>,
    ) -> bool
    where
        Context: LightingContextProvider,
        Batteries: BatteryStatusProvider + ?Sized,
    {
        if let Some(wanted) = self.output_mode
            && output_mode != Some(wanted)
        {
            return false;
        }
        if let Some(wanted) = self.effects
            && effects_enabled != Some(wanted.enabled)
        {
            return false;
        }
        if let Some(condition) = self.layer
            && context.lighting_context().layers.is_active(condition.layer) != condition.active
        {
            return false;
        }
        if let Some(condition) = self.connection {
            let connection = context.lighting_context().connection;
            if let Some(wanted) = condition.transport {
                let active = match connection.decide_active() {
                    Some(ConnectionType::Usb) => ActiveTransport::Usb,
                    Some(ConnectionType::Ble) => ActiveTransport::Ble,
                    None => ActiveTransport::NoneActive,
                };
                if active != wanted {
                    return false;
                }
            }
            if condition
                .profile
                .is_some_and(|profile| connection.ble.profile != profile)
                || condition.ble_state.is_some_and(|state| connection.ble.state != state)
            {
                return false;
            }
            if let Some(wanted) = condition.bonded {
                let bonded = (context.lighting_context().bonded_slots >> wanted.slot) & 1 == 1;
                if bonded != wanted.bonded {
                    return false;
                }
            }
        }
        let Some(condition) = self.battery else {
            return true;
        };
        let BatteryStatus::Available { charge_state, level } = batteries.battery_status(condition.node) else {
            return false;
        };
        if !matches!(
            (condition.charge, charge_state),
            (ChargeCondition::Any, _)
                | (ChargeCondition::Charging, ChargeState::Charging)
                | (ChargeCondition::Discharging, ChargeState::Discharging)
                | (ChargeCondition::Unknown, ChargeState::Unknown)
        ) {
            return false;
        }
        if condition.min_level.is_none() && condition.max_level.is_none() {
            return true;
        }
        let Some(level) = level else {
            return false;
        };
        condition.min_level.is_none_or(|min| level >= min) && condition.max_level.is_none_or(|max| level <= max)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ConditionalSceneCell<E> {
    pub conditions: ConditionSet,
    pub slot: LedSlot,
    pub effect: E,
}

/// Immutable conditional cells compiled from `keyboard.toml`.
///
/// Matching cells compose in declaration order. This permits a broad rule to
/// establish a default and a later, narrower rule to override the same LED.
#[derive(Copy, Clone, Debug)]
pub struct ConditionalScenes<'a, E, Batteries: ?Sized> {
    pub cells: &'a [ConditionalSceneCell<E>],
    pub batteries: &'a Batteries,
}

impl<'a, E, Batteries: ?Sized> ConditionalScenes<'a, E, Batteries> {
    pub const fn new(cells: &'a [ConditionalSceneCell<E>], batteries: &'a Batteries) -> Self {
        Self { cells, batteries }
    }

    pub const fn cell_count(&self) -> usize {
        self.cells.len()
    }

    fn cell_at<Context>(&self, context: &Context, mut wanted: usize) -> &ConditionalSceneCell<E>
    where
        Context: LightingContextProvider,
        Batteries: BatteryStatusProvider,
    {
        for cell in self
            .cells
            .iter()
            .filter(|cell| cell.conditions.matches(context, self.batteries, None, None))
        {
            if wanted == 0 {
                return cell;
            }
            wanted -= 1;
        }
        panic!("LightingSource index must be below len")
    }
}

impl<C, Context, E, Batteries> LightingSource<C, Context> for ConditionalScenes<'_, E, Batteries>
where
    Context: LightingContextProvider,
    E: LightingEffect<C>,
    Batteries: BatteryStatusProvider + ?Sized,
{
    fn len(&self, input: &RenderInput<'_, Context>) -> usize {
        self.cells
            .iter()
            .filter(|cell| cell.conditions.matches(input.context, self.batteries, None, None))
            .count()
    }

    fn slot(&self, index: usize, input: &RenderInput<'_, Context>) -> LedSlot {
        self.cell_at(input.context, index).slot
    }

    fn contribution(&mut self, index: usize, input: &RenderInput<'_, Context>) -> Contribution<C> {
        Contribution::Opaque(self.cell_at(input.context, index).effect.sample(input.now_ms))
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Indicator {
    NumLock,
    CapsLock,
    ScrollLock,
    Compose,
    Kana,
}

#[derive(Copy, Clone, Debug)]
pub struct IndicatorScene<'a, E> {
    pub indicator: Indicator,
    pub active: bool,
    pub cells: &'a [SceneCell<E>],
}

/// Sparse status scenes driven from the authoritative indicator snapshot.
/// Multiple matching scenes compose in declaration order within the source.
#[derive(Copy, Clone, Debug)]
pub struct IndicatorScenes<'a, E> {
    pub scenes: &'a [IndicatorScene<'a, E>],
}

impl<E> IndicatorScenes<'_, E> {
    fn scene_active<Context: LightingContextProvider>(scene: &IndicatorScene<'_, E>, context: &Context) -> bool {
        let indicators = context.lighting_context().indicators;
        let actual = match scene.indicator {
            Indicator::NumLock => indicators.num_lock,
            Indicator::CapsLock => indicators.caps_lock,
            Indicator::ScrollLock => indicators.scroll_lock,
            Indicator::Compose => indicators.compose,
            Indicator::Kana => indicators.kana,
        };
        actual == scene.active
    }

    fn cell_at<Context: LightingContextProvider>(&self, context: &Context, mut wanted: usize) -> &SceneCell<E> {
        for scene in self.scenes.iter().filter(|scene| Self::scene_active(scene, context)) {
            if wanted < scene.cells.len() {
                return &scene.cells[wanted];
            }
            wanted -= scene.cells.len();
        }
        panic!("LightingSource index must be below len")
    }
}

impl<C, Context, E> LightingSource<C, Context> for IndicatorScenes<'_, E>
where
    Context: LightingContextProvider,
    E: LightingEffect<C>,
{
    fn len(&self, input: &RenderInput<'_, Context>) -> usize {
        self.scenes
            .iter()
            .filter(|scene| Self::scene_active(scene, input.context))
            .map(|scene| scene.cells.len())
            .sum()
    }

    fn slot(&self, index: usize, input: &RenderInput<'_, Context>) -> LedSlot {
        self.cell_at(input.context, index).slot
    }

    fn contribution(&mut self, index: usize, input: &RenderInput<'_, Context>) -> Contribution<C> {
        Contribution::Opaque(self.cell_at(input.context, index).effect.sample(input.now_ms))
    }
}

/// Adapter for a caller-owned dense effect buffer.
#[derive(Copy, Clone, Debug)]
pub struct DenseSource<'a, C> {
    pub pixels: &'a [C],
    pub next_change_ms: Option<u64>,
}

impl<C: Copy, Context> LightingSource<C, Context> for DenseSource<'_, C> {
    fn len(&self, _: &RenderInput<'_, Context>) -> usize {
        self.pixels.len()
    }

    fn slot(&self, index: usize, _: &RenderInput<'_, Context>) -> LedSlot {
        LedSlot::from_index(index)
    }

    fn contribution(&mut self, index: usize, _: &RenderInput<'_, Context>) -> Contribution<C> {
        Contribution::Opaque(EffectSample {
            color: self.pixels[index],
            next_change_ms: self.next_change_ms,
        })
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OverlayUpdate<E> {
    pub slot: LedSlot,
    pub effect: E,
    pub expires_ms: Option<u64>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct OverlayEntry<E> {
    active: bool,
    update: OverlayUpdate<E>,
}

impl<E: Copy + Default> OverlayEntry<E> {
    fn empty() -> Self {
        Self {
            active: false,
            update: OverlayUpdate {
                slot: LedSlot(0),
                effect: E::default(),
                expires_ms: None,
            },
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum OverlayError {
    Full,
    TooManyEntries { supplied: usize, capacity: usize },
    DuplicateSlot { slot: LedSlot },
}

/// Fixed-capacity host/transient overlay. It is an ordinary source rather
/// than a privileged side channel, so priority and occlusion rules stay
/// uniform.
pub struct TtlOverlay<E, const CAP: usize> {
    entries: [OverlayEntry<E>; CAP],
}

impl<E: Copy + Default, const CAP: usize> TtlOverlay<E, CAP> {
    pub fn new() -> Self {
        Self {
            entries: [OverlayEntry::empty(); CAP],
        }
    }

    pub fn set(&mut self, now_ms: u64, update: OverlayUpdate<E>) -> Result<(), OverlayError> {
        self.prune_expired(now_ms);
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.active && entry.update.slot == update.slot)
        {
            entry.update = update;
            return Ok(());
        }
        let Some(entry) = self.entries.iter_mut().find(|entry| !entry.active) else {
            return Err(OverlayError::Full);
        };
        *entry = OverlayEntry { active: true, update };
        Ok(())
    }

    pub fn unset(&mut self, slot: LedSlot) -> bool {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.active && entry.update.slot == slot)
        {
            entry.active = false;
            true
        } else {
            false
        }
    }

    pub fn clear(&mut self) {
        for entry in &mut self.entries {
            entry.active = false;
        }
    }

    /// Atomically replace the overlay; validation happens before mutation.
    pub fn replace(&mut self, now_ms: u64, updates: &[OverlayUpdate<E>]) -> Result<(), OverlayError> {
        if updates.len() > CAP {
            return Err(OverlayError::TooManyEntries {
                supplied: updates.len(),
                capacity: CAP,
            });
        }
        for (index, update) in updates.iter().enumerate() {
            if updates[..index].iter().any(|previous| previous.slot == update.slot) {
                return Err(OverlayError::DuplicateSlot { slot: update.slot });
            }
        }
        self.clear();
        for (entry, update) in self.entries.iter_mut().zip(updates.iter().copied()) {
            if !update.expires_ms.is_some_and(|expires| expires <= now_ms) {
                *entry = OverlayEntry { active: true, update };
            }
        }
        Ok(())
    }

    pub fn prune_expired(&mut self, now_ms: u64) {
        for entry in &mut self.entries {
            if entry.active && entry.update.expires_ms.is_some_and(|expires| expires <= now_ms) {
                entry.active = false;
            }
        }
    }

    pub fn active_len(&self) -> usize {
        self.entries.iter().filter(|entry| entry.active).count()
    }

    /// Iterate the currently stored absolute updates in stable slot order.
    ///
    /// This is intentionally a read-only state boundary rather than an
    /// encoding API. A board-level replication transport can take an atomic
    /// engine snapshot, convert absolute expiries to remaining TTLs, and
    /// choose its own bounded wire representation.
    pub fn active_updates(&self) -> impl Iterator<Item = OverlayUpdate<E>> + '_ {
        self.entries
            .iter()
            .filter(|entry| entry.active)
            .map(|entry| entry.update)
    }

    fn active_entry(&self, wanted: usize) -> &OverlayEntry<E> {
        self.entries
            .iter()
            .filter(|entry| entry.active)
            .nth(wanted)
            .expect("LightingSource index must be below len")
    }
}

impl<E: Copy + Default, const CAP: usize> Default for TtlOverlay<E, CAP> {
    fn default() -> Self {
        Self::new()
    }
}

impl<C, E, Context, const CAP: usize> LightingSource<C, Context> for TtlOverlay<E, CAP>
where
    E: Copy + Default + LightingEffect<C>,
{
    fn len(&self, _: &RenderInput<'_, Context>) -> usize {
        self.active_len()
    }

    fn slot(&self, index: usize, _: &RenderInput<'_, Context>) -> LedSlot {
        self.active_entry(index).update.slot
    }

    fn contribution(&mut self, index: usize, input: &RenderInput<'_, Context>) -> Contribution<C> {
        let update = self.active_entry(index).update;
        if update.expires_ms.is_some_and(|expires| expires <= input.now_ms) {
            Contribution::Transparent { next_change_ms: None }
        } else {
            let mut sample = update.effect.sample(input.now_ms);
            sample.next_change_ms = earliest(sample.next_change_ms, update.expires_ms);
            Contribution::Opaque(sample)
        }
    }
}

fn earliest(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use rmk_types::ble::{BleState, BleStatus};
    use rmk_types::connection::{ConnectionStatus, ConnectionType, UsbState};

    use super::super::{BuiltinEffect, Compositor, LayerState, LogicalFrame, Rgb8};
    use super::*;
    use crate::types::battery::{BatteryStatus, ChargeState};

    const RED: Rgb8 = Rgb8::new(10, 0, 0);
    const GREEN: Rgb8 = Rgb8::new(0, 10, 0);
    const BLUE: Rgb8 = Rgb8::new(0, 0, 10);

    struct NoBatteries;

    impl BatteryStatusProvider for NoBatteries {
        fn battery_status(&self, _: u8) -> BatteryStatus {
            BatteryStatus::Unavailable
        }
    }

    #[test]
    fn connection_conditions_conjoin_transport_profile_and_ble_state() {
        let mut context = LightingContext {
            connection: ConnectionStatus {
                usb: UsbState::Configured,
                ble: BleStatus {
                    profile: 3,
                    state: BleState::Connected,
                },
                preferred: ConnectionType::Ble,
            },
            ..LightingContext::default()
        };
        let conditions = ConditionSet {
            connection: Some(ConnectionCondition {
                transport: Some(ActiveTransport::Ble),
                profile: Some(3),
                ble_state: Some(BleState::Connected),
                bonded: None,
            }),
            ..ConditionSet::default()
        };

        assert!(conditions.matches(&context, &NoBatteries, None, None));
        context.connection.preferred = ConnectionType::Usb;
        assert!(!conditions.matches(&context, &NoBatteries, None, None));

        let usb = ConditionSet {
            connection: Some(ConnectionCondition {
                transport: Some(ActiveTransport::Usb),
                profile: Some(3),
                ble_state: Some(BleState::Connected),
                bonded: None,
            }),
            ..ConditionSet::default()
        };
        assert!(usb.matches(&context, &NoBatteries, None, None));

        context.connection.ble.profile = 4;
        assert!(!usb.matches(&context, &NoBatteries, None, None));
        context.connection.ble.profile = 3;
        context.connection.ble.state = BleState::Advertising;
        assert!(!usb.matches(&context, &NoBatteries, None, None));
    }

    #[test]
    fn effects_condition_needs_a_source_that_can_answer() {
        let context = LightingContext::default();
        let on = ConditionSet {
            effects: Some(EffectsCondition { enabled: true }),
            ..ConditionSet::default()
        };
        let off = ConditionSet {
            effects: Some(EffectsCondition { enabled: false }),
            ..ConditionSet::default()
        };

        assert!(on.matches(&context, &NoBatteries, None, Some(true)));
        assert!(!on.matches(&context, &NoBatteries, None, Some(false)));
        assert!(off.matches(&context, &NoBatteries, None, Some(false)));
        assert!(!off.matches(&context, &NoBatteries, None, Some(true)));

        // A source with no view of the extension leaves both unsatisfiable,
        // rather than letting one of them fire wherever it cannot be judged.
        assert!(!on.matches(&context, &NoBatteries, None, None));
        assert!(!off.matches(&context, &NoBatteries, None, None));
    }

    #[test]
    fn bonded_slot_condition_reads_the_bitmap_regardless_of_active_profile() {
        let mut context = LightingContext {
            bonded_slots: 0b101,
            ..LightingContext::default()
        };
        let paired = |slot| ConditionSet {
            connection: Some(ConnectionCondition {
                bonded: Some(BondedSlotCondition { slot, bonded: true }),
                ..ConnectionCondition::default()
            }),
            ..ConditionSet::default()
        };
        let empty = |slot| ConditionSet {
            connection: Some(ConnectionCondition {
                bonded: Some(BondedSlotCondition { slot, bonded: false }),
                ..ConnectionCondition::default()
            }),
            ..ConditionSet::default()
        };

        assert!(paired(0).matches(&context, &NoBatteries, None, None));
        assert!(empty(1).matches(&context, &NoBatteries, None, None));
        assert!(paired(2).matches(&context, &NoBatteries, None, None));
        assert!(!paired(1).matches(&context, &NoBatteries, None, None));
        assert!(!empty(0).matches(&context, &NoBatteries, None, None));

        // The active profile plays no part: switching it changes nothing.
        context.connection.ble.profile = 1;
        assert!(paired(0).matches(&context, &NoBatteries, None, None));
        assert!(empty(1).matches(&context, &NoBatteries, None, None));
    }

    #[test]
    fn connection_condition_matches_none_active_and_empty_condition() {
        let context = LightingContext {
            connection: ConnectionStatus {
                usb: UsbState::Enabled,
                ble: BleStatus {
                    profile: 2,
                    state: BleState::Advertising,
                },
                preferred: ConnectionType::Usb,
            },
            ..LightingContext::default()
        };
        let none_active = ConditionSet {
            connection: Some(ConnectionCondition {
                transport: Some(ActiveTransport::NoneActive),
                profile: Some(2),
                ble_state: Some(BleState::Advertising),
                bonded: None,
            }),
            ..ConditionSet::default()
        };
        let empty = ConditionSet {
            connection: Some(ConnectionCondition::default()),
            ..ConditionSet::default()
        };

        assert!(none_active.matches(&context, &NoBatteries, None, None));
        assert!(empty.matches(&context, &NoBatteries, None, None));
    }

    #[test]
    fn layer_source_has_sparse_active_stack_fallthrough() {
        let base = [SceneCell {
            slot: LedSlot(0),
            effect: BuiltinEffect::solid(RED),
        }];
        let held = [SceneCell {
            slot: LedSlot(1),
            effect: BuiltinEffect::solid(GREEN),
        }];
        let effective = [SceneCell {
            slot: LedSlot(0),
            effect: BuiltinEffect::solid(BLUE),
        }];
        let scenes = [
            LayerScene { layer: 0, cells: &base },
            LayerScene { layer: 2, cells: &held },
            LayerScene {
                layer: 3,
                cells: &effective,
            },
        ];
        let mut source = LayerScenes {
            scenes: &scenes,
            policy: LayerPolicy::ActiveStack,
        };
        let context = LightingContext {
            layers: LayerState::new(3, 0, 0b1101),
            indicators: Default::default(),
            powered: false,
            connection: Default::default(),
            bonded_slots: 0,
        };
        let compositor = Compositor::<Rgb8, 2>::new(Rgb8::BLACK);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let mut tx = compositor.begin(0, &context, Rgb8::BLACK, &mut frame);
        tx.apply(10, &mut source).unwrap();
        tx.finish();
        assert_eq!(frame.as_slice(), &[BLUE, GREEN]);
    }

    #[test]
    fn ttl_overlay_expires_exactly_and_replace_is_atomic() {
        let mut overlay = TtlOverlay::<BuiltinEffect, 2>::new();
        overlay
            .set(
                0,
                OverlayUpdate {
                    slot: LedSlot(0),
                    effect: BuiltinEffect::solid(RED),
                    expires_ms: Some(10),
                },
            )
            .unwrap();
        overlay
            .set(
                0,
                OverlayUpdate {
                    slot: LedSlot(1),
                    effect: BuiltinEffect::solid(GREEN),
                    expires_ms: None,
                },
            )
            .unwrap();
        let before = overlay.active_len();
        assert_eq!(
            overlay.replace(
                0,
                &[
                    OverlayUpdate {
                        slot: LedSlot(0),
                        effect: BuiltinEffect::solid(BLUE),
                        expires_ms: None
                    },
                    OverlayUpdate {
                        slot: LedSlot(0),
                        effect: BuiltinEffect::solid(GREEN),
                        expires_ms: None
                    },
                ]
            ),
            Err(OverlayError::DuplicateSlot { slot: LedSlot(0) })
        );
        assert_eq!(overlay.active_len(), before);

        let compositor = Compositor::<Rgb8, 2>::new(Rgb8::BLACK);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let mut tx = compositor.begin(9, &(), Rgb8::BLACK, &mut frame);
        tx.apply(0, &mut overlay).unwrap();
        assert_eq!(tx.finish().next_wake_ms, Some(10));
        let mut tx = compositor.begin(10, &(), Rgb8::BLACK, &mut frame);
        tx.apply(0, &mut overlay).unwrap();
        tx.finish();
        assert_eq!(frame.as_slice(), &[Rgb8::BLACK, GREEN]);
    }

    #[test]
    fn conditional_scenes_conjoin_live_layer_and_battery_state() {
        struct Batteries;
        impl BatteryStatusProvider for Batteries {
            fn battery_status(&self, node: u8) -> BatteryStatus {
                if node == 0 {
                    BatteryStatus::Available {
                        charge_state: ChargeState::Discharging,
                        level: Some(35),
                    }
                } else {
                    BatteryStatus::Unavailable
                }
            }
        }
        let layer = Some(LayerCondition { layer: 2, active: true });
        let cells = [
            ConditionalSceneCell {
                conditions: ConditionSet {
                    layer,
                    battery: Some(BatteryCondition {
                        node: 0,
                        min_level: Some(1),
                        max_level: None,
                        charge: ChargeCondition::Any,
                    }),
                    connection: None,
                    output_mode: None,
                    effects: None,
                },
                slot: LedSlot(0),
                effect: BuiltinEffect::solid(GREEN),
            },
            ConditionalSceneCell {
                conditions: ConditionSet {
                    layer,
                    battery: Some(BatteryCondition {
                        node: 0,
                        min_level: Some(41),
                        max_level: None,
                        charge: ChargeCondition::Any,
                    }),
                    connection: None,
                    output_mode: None,
                    effects: None,
                },
                slot: LedSlot(1),
                effect: BuiltinEffect::solid(BLUE),
            },
            // A later, narrower match overrides the broad green contribution.
            ConditionalSceneCell {
                conditions: ConditionSet {
                    layer,
                    battery: Some(BatteryCondition {
                        node: 0,
                        min_level: Some(1),
                        max_level: Some(40),
                        charge: ChargeCondition::Discharging,
                    }),
                    connection: None,
                    output_mode: None,
                    effects: None,
                },
                slot: LedSlot(0),
                effect: BuiltinEffect::solid(RED),
            },
        ];
        let batteries = Batteries;
        let mut source = ConditionalScenes::new(&cells, &batteries);
        let context = LightingContext {
            layers: LayerState::new(2, 0, 0b101),
            indicators: Default::default(),
            powered: false,
            connection: Default::default(),
            bonded_slots: 0,
        };
        let compositor = Compositor::<Rgb8, 2>::new(Rgb8::BLACK);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let mut tx = compositor.begin(0, &context, Rgb8::BLACK, &mut frame);
        tx.apply(0, &mut source).unwrap();
        tx.finish();
        assert_eq!(frame.as_slice(), &[RED, Rgb8::BLACK]);
    }

    #[test]
    fn indicator_scenes_use_extended_context_provider() {
        #[derive(Default)]
        struct Extended {
            lighting: LightingContext,
            _battery_percent: u8,
        }
        impl LightingContextProvider for Extended {
            fn lighting_context(&self) -> &LightingContext {
                &self.lighting
            }
        }
        let cells = [SceneCell {
            slot: LedSlot(0),
            effect: BuiltinEffect::solid(RED),
        }];
        let scenes = [IndicatorScene {
            indicator: Indicator::CapsLock,
            active: true,
            cells: &cells,
        }];
        let mut source = IndicatorScenes { scenes: &scenes };
        let mut context = Extended::default();
        context.lighting.indicators.caps_lock = true;
        let compositor = Compositor::<Rgb8, 1>::new(Rgb8::BLACK);
        let mut frame = LogicalFrame::new(Rgb8::BLACK);
        let mut tx = compositor.begin(0, &context, Rgb8::BLACK, &mut frame);
        tx.apply(0, &mut source).unwrap();
        tx.finish();
        assert_eq!(frame.as_slice(), &[RED]);
    }
}
