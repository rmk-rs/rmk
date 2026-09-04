//! Sticky Key behavior for held [`Action`]s.
//!
//! Every active action uses the same entry lifecycle. Action-specific code is
//! confined to applying and releasing the effect; modifier and layer entries
//! may coexist, while a modified tap key remains deliberately exclusive.

use core::future::Future;

use embassy_time::{Duration, Instant};
use rmk_types::action::Action;
use rmk_types::keycode::{HidKeyCode, KeyCode};
use rmk_types::modifier::ModifierCombination;

#[cfg(test)]
use crate::config::StickyKeyHoldDuration;
use crate::config::StickyKeyReleaseMode;
use crate::event::{KeyboardEvent, KeyboardEventPos};
use crate::keyboard::Keyboard;
use crate::keymap::{StickyKeyPolicy, StickyKeyShape};

fn deadline_from(start: Instant, duration: Duration) -> Option<Instant> {
    (duration != Duration::MAX).then(|| start + duration)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StickyPhase {
    /// One or more physical producers are still down.
    Pressed,
    /// A producer remains down, but its press deadline is absent or consumed.
    /// `StickyEntry::timing_marker` stores the continuous chord's start without scheduling a wake.
    PressDeadlineInactive,
    /// Every producer is up and the effect is armed.
    Latched,
    /// A foreign key was pressed while a producer remained down.
    Held,
    /// The configured key-up release threshold elapsed while a producer remained down.
    HoldQualified,
}

#[derive(Clone, Copy, Debug)]
struct StickyEntry {
    /// Canonical identity of the held action. Effect code derives its shape
    /// from this value instead of parallel modifier/layer/tap-key slots.
    action: Action,
    source: StickyKeySource,
    policy: StickyKeyPolicy,
    phase: StickyPhase,
    repeat_count: u16,
    /// Active deadline in `Pressed`/`Latched`, or the chord start while a
    /// physical producer is down and no wakeup is needed.
    timing_marker: Option<Instant>,
    buffered_claim: bool,
    /// Action-local state that cannot be derived from the held `Action`.
    effect_state: StickyEffectState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhysicalRelease {
    Ignored,
    Latched,
    Released,
}

impl StickyEntry {
    fn new(
        action: Action,
        source: StickyKeySource,
        event_pos: KeyboardEventPos,
        policy: StickyKeyPolicy,
        pressed_at: Instant,
    ) -> Self {
        let (phase, timing_marker) = Self::press_state(policy, pressed_at, pressed_at);
        Self {
            action,
            source,
            policy,
            phase,
            repeat_count: 1,
            timing_marker,
            buffered_claim: false,
            effect_state: match action {
                Action::Modifier(modifiers) => {
                    StickyEffectState::Modifier(StickyModifierEffect::new(modifiers, source))
                }
                _ => StickyEffectState::ActionOnly(event_pos),
            },
        }
    }

    fn new_modifier(
        modifiers: ModifierCombination,
        source: KeyboardEventPos,
        producer_source: StickyKeySource,
        policy: StickyKeyPolicy,
        pressed_at: Instant,
    ) -> Self {
        Self::new(Action::Modifier(modifiers), producer_source, source, policy, pressed_at)
    }

    fn is_modifier(&self) -> bool {
        matches!(self.action, Action::Modifier(_))
    }

    fn is_layer(&self) -> bool {
        matches!(self.action, Action::LayerOn(_))
    }

    fn is_tap_key(&self) -> bool {
        matches!(self.action, Action::KeyWithModifier(_, _))
    }

    fn modifiers(&self) -> ModifierCombination {
        match self.action {
            Action::Modifier(modifiers) | Action::KeyWithModifier(_, modifiers) => modifiers,
            _ => ModifierCombination::new(),
        }
    }

    fn layer(&self) -> Option<u8> {
        match self.action {
            Action::LayerOn(layer) => Some(layer),
            _ => None,
        }
    }

    fn add_modifier_producer(
        &mut self,
        modifiers: ModifierCombination,
        source: StickyKeySource,
    ) -> ModifierProducerInsert {
        let inserted = self.modifier_effect_mut().begin_press(modifiers, source);
        if inserted == ModifierProducerInsert::Added {
            let Action::Modifier(current) = self.action else {
                unreachable!("modifier entry must contain a modifier action");
            };
            self.action = Action::Modifier(current | modifiers);
        }
        inserted
    }

    fn on_exact_modifier_release(
        &mut self,
        modifiers: ModifierCombination,
        source: StickyKeySource,
    ) -> ModifierProducerRelease {
        let Action::Modifier(old_effective) = self.action else {
            unreachable!("modifier entry must contain a modifier action");
        };
        let release = self
            .modifier_effect_mut()
            .on_exact_release(modifiers, source, old_effective);
        if let ModifierProducerRelease::Released { new_effective, .. } = release {
            self.action = Action::Modifier(new_effective);
        }
        release
    }

    fn retain_modifier(&mut self, modifiers: ModifierCombination) {
        self.modifier_effect_mut().retain(modifiers);
        let Action::Modifier(current) = self.action else {
            unreachable!("modifier entry must contain a modifier action");
        };
        self.action = Action::Modifier(current | modifiers);
    }

    fn modifier_effect_mut(&mut self) -> &mut StickyModifierEffect {
        match &mut self.effect_state {
            StickyEffectState::Modifier(effect) => effect,
            StickyEffectState::ActionOnly(_) => unreachable!("modifier action must own modifier effect state"),
        }
    }

    fn deadline(&self) -> Option<Instant> {
        if matches!(self.phase, StickyPhase::Pressed | StickyPhase::Latched) {
            self.timing_marker
        } else {
            None
        }
    }

    fn press_state(policy: StickyKeyPolicy, chord_started_at: Instant, now: Instant) -> (StickyPhase, Option<Instant>) {
        let Some(hold_duration) = policy.release_after_hold.duration() else {
            return (StickyPhase::PressDeadlineInactive, Some(chord_started_at));
        };

        match deadline_from(chord_started_at, hold_duration) {
            Some(deadline) if deadline > now => (StickyPhase::Pressed, Some(deadline)),
            Some(_) => (StickyPhase::HoldQualified, Some(chord_started_at)),
            None => (StickyPhase::PressDeadlineInactive, Some(chord_started_at)),
        }
    }

    fn begin_press(
        &mut self,
        source: StickyKeySource,
        event_pos: KeyboardEventPos,
        policy: StickyKeyPolicy,
        pressed_at: Instant,
    ) {
        self.source = source;
        if let StickyEffectState::ActionOnly(release_pos) = &mut self.effect_state {
            *release_pos = event_pos;
        }
        self.policy = policy;
        (self.phase, self.timing_marker) = Self::press_state(policy, pressed_at, pressed_at);
        self.buffered_claim = false;
    }

    /// Add a producer to the current modifier chord without forgetting how
    /// long the chord has already been held. The latest producer still selects
    /// the policy, but its threshold is measured from the chord's first press.
    fn begin_modifier_press(&mut self, source: StickyKeySource, policy: StickyKeyPolicy, pressed_at: Instant) {
        let chord_started_at = match self.phase {
            StickyPhase::Pressed => self
                .timing_marker
                .zip(self.policy.release_after_hold.duration())
                .map(|(deadline, hold_duration)| deadline - hold_duration),
            StickyPhase::PressDeadlineInactive | StickyPhase::HoldQualified => self.timing_marker,
            StickyPhase::Latched | StickyPhase::Held => None,
        }
        .unwrap_or(pressed_at);

        self.source = source;
        self.policy = policy;
        (self.phase, self.timing_marker) = Self::press_state(policy, chord_started_at, pressed_at);
        self.buffered_claim = false;
    }

    fn on_physical_release(&mut self, owner: Option<StickyKeySource>, now: Instant) -> PhysicalRelease {
        if owner.is_some_and(|owner| owner != self.source) {
            return PhysicalRelease::Ignored;
        }
        match self.phase {
            StickyPhase::Pressed => {
                if self.policy.release_after_hold.duration().is_some()
                    && self.timing_marker.is_some_and(|deadline| deadline <= now)
                {
                    self.phase = StickyPhase::HoldQualified;
                    self.timing_marker = None;
                    return PhysicalRelease::Released;
                }
                self.phase = StickyPhase::Latched;
                self.timing_marker = deadline_from(now, self.policy.timeout);
                PhysicalRelease::Latched
            }
            StickyPhase::PressDeadlineInactive => {
                self.phase = StickyPhase::Latched;
                self.timing_marker = deadline_from(now, self.policy.timeout);
                PhysicalRelease::Latched
            }
            StickyPhase::Held | StickyPhase::HoldQualified => {
                self.timing_marker = None;
                PhysicalRelease::Released
            }
            StickyPhase::Latched => PhysicalRelease::Ignored,
        }
    }

    fn mark_foreign_key(&mut self) {
        if matches!(
            self.phase,
            StickyPhase::Pressed | StickyPhase::PressDeadlineInactive | StickyPhase::HoldQualified
        ) {
            self.phase = StickyPhase::Held;
            self.timing_marker = None;
        }
    }

    fn trigger_for_key(&self, pressed: bool) -> bool {
        let trigger = if pressed {
            StickyKeyReleaseMode::OTHER_KEY_PRESS
        } else {
            StickyKeyReleaseMode::OTHER_KEY_RELEASE
        };
        self.policy.release_mode.intersects(trigger)
    }

    fn claim_buffered_press(&mut self, source: KeyboardEventPos) {
        if self.phase == StickyPhase::Latched
            && self.source != StickyKeySource::Direct(source)
            && self.trigger_for_key(true)
        {
            self.buffered_claim = true;
            self.timing_marker = None;
        }
    }

    fn finish_buffered_claim(&mut self) {
        if self.buffered_claim {
            self.buffered_claim = false;
            self.timing_marker = deadline_from(Instant::now(), self.policy.timeout);
        }
    }

    fn is_double_tap(&self, source: StickyKeySource, policy: StickyKeyPolicy) -> bool {
        self.phase == StickyPhase::Latched
            && self.source == source
            && policy.release_mode.intersects(StickyKeyReleaseMode::DOUBLE_TAP)
    }

    /// A timeout cannot erase a latch whose physical producer is still down:
    /// its later release must still complete the lifecycle.
    fn deadline_disposition(&mut self, now: Instant) -> DeadlineDisposition {
        let Some(deadline) = self.deadline().filter(|deadline| *deadline <= now) else {
            return DeadlineDisposition::Pending;
        };
        if self.phase == StickyPhase::Pressed {
            self.timing_marker = self
                .policy
                .release_after_hold
                .duration()
                .map(|hold_duration| deadline - hold_duration);
            self.phase = StickyPhase::HoldQualified;
            DeadlineDisposition::Deferred
        } else {
            DeadlineDisposition::Release
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeadlineDisposition {
    Pending,
    Deferred,
    Release,
}

/// Direct keys and combo outputs share this bound while they are physically
/// held. Eight permits one producer per HID modifier while bounding each active
/// Sticky entry on small MCUs. An overflow press and its later release are both
/// ignored, so they cannot disturb the accepted producers.
const MAX_STICKY_MODIFIER_PRODUCERS: usize = 8;
const MAX_ACTIVE_STICKY_KEYS: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
struct ModifierProducer {
    source: StickyKeySource,
    modifiers: ModifierCombination,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum StickyKeySource {
    /// A physical key, matched by its matrix position.
    Direct(KeyboardEventPos),
    /// A combo output, matched by its stable slot in the combo table.
    Combo(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModifierProducerInsert {
    Added,
    Full,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModifierProducerRelease {
    Unmatched,
    Released {
        removed: ModifierProducer,
        old_effective: ModifierCombination,
        new_effective: ModifierCombination,
        producers_remain: bool,
    },
}

/// Physical and retained ownership for accumulated Sticky modifiers.
///
/// Producer identities keep late or rejected releases from removing another
/// owner with the same modifier mask. Combo slots provide a stable identity
/// even when different constituent positions trigger press and release.
#[derive(Clone, Copy, Debug)]
struct StickyModifierEffect {
    producers: [Option<ModifierProducer>; MAX_STICKY_MODIFIER_PRODUCERS],
    retained: ModifierCombination,
}

impl StickyModifierEffect {
    fn new(modifiers: ModifierCombination, source: StickyKeySource) -> Self {
        let mut producers = [None; MAX_STICKY_MODIFIER_PRODUCERS];
        producers[0] = Some(ModifierProducer { source, modifiers });
        Self {
            producers,
            retained: ModifierCombination::new(),
        }
    }

    fn begin_press(&mut self, modifiers: ModifierCombination, source: StickyKeySource) -> ModifierProducerInsert {
        let producer = ModifierProducer { source, modifiers };
        if let Some(slot) = self.producers.iter_mut().find(|slot| slot.is_none()) {
            *slot = Some(producer);
            ModifierProducerInsert::Added
        } else {
            warn!(
                "Too many simultaneous Sticky modifier producers; ignoring {:?}",
                producer
            );
            ModifierProducerInsert::Full
        }
    }

    fn retain(&mut self, modifiers: ModifierCombination) {
        self.retained |= modifiers;
    }

    fn effective(&self) -> ModifierCombination {
        self.producers
            .iter()
            .flatten()
            .fold(self.retained, |modifiers, producer| modifiers | producer.modifiers)
    }

    fn release_at(&mut self, index: usize, old_effective: ModifierCombination) -> ModifierProducerRelease {
        let removed = self.producers[index]
            .take()
            .expect("release index must contain a modifier producer");
        let new_effective = self.effective();
        ModifierProducerRelease::Released {
            removed,
            old_effective,
            new_effective,
            producers_remain: self.producers.iter().any(Option::is_some),
        }
    }

    // `StickyEntry::action` caches `old_effective` so the release path does not
    // scan the fixed producer table twice.
    fn on_exact_release(
        &mut self,
        modifiers: ModifierCombination,
        source: StickyKeySource,
        old_effective: ModifierCombination,
    ) -> ModifierProducerRelease {
        let exact = ModifierProducer { source, modifiers };
        let Some(index) = self.producers.iter().position(|producer| *producer == Some(exact)) else {
            return ModifierProducerRelease::Unmatched;
        };
        self.release_at(index, old_effective)
    }
}

#[derive(Clone, Copy, Debug)]
enum StickyEffectState {
    /// Layer and tap-key effects also retain the position used for HID cleanup.
    ActionOnly(KeyboardEventPos),
    /// Modifier effects additionally track physical and retained ownership.
    Modifier(StickyModifierEffect),
}

#[derive(Clone, Copy, Debug, Default)]
struct StickyKeyUpdate {
    modifier_consumed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StickyKeyPostAction {
    None,
    Release,
}

/// Bounded runtime collection for held actions.
///
/// Two entries preserve the only simultaneous configuration supported by the
/// behavior contract: one accumulated modifier action plus one layer action.
/// Modified tap keys are exclusive and therefore require only one entry.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct StickyKeyState {
    active: [Option<StickyEntry>; MAX_ACTIVE_STICKY_KEYS],
}

impl StickyKeyState {
    fn index_matching(&self, predicate: impl Fn(&StickyEntry) -> bool) -> Option<usize> {
        self.active
            .iter()
            .position(|entry| entry.as_ref().is_some_and(&predicate))
    }

    fn modifier_index(&self) -> Option<usize> {
        self.index_matching(StickyEntry::is_modifier)
    }

    fn layer_index(&self) -> Option<usize> {
        self.index_matching(StickyEntry::is_layer)
    }

    fn tap_key_index(&self) -> Option<usize> {
        self.index_matching(StickyEntry::is_tap_key)
    }

    fn insert(&mut self, entry: StickyEntry) -> Result<usize, StickyEntry> {
        let Some(index) = self.active.iter().position(Option::is_none) else {
            return Err(entry);
        };
        self.active[index] = Some(entry);
        Ok(index)
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.active.iter().flatten().filter_map(StickyEntry::deadline).min()
    }

    pub(crate) fn claim_buffered_press(&mut self, event: KeyboardEvent) {
        if !event.pressed {
            return;
        }
        for entry in self.active.iter_mut().flatten() {
            entry.claim_buffered_press(event.pos);
        }
    }

    pub(crate) fn finish_buffered_claim(&mut self) {
        for entry in self.active.iter_mut().flatten() {
            entry.finish_buffered_claim();
        }
    }

    fn modifier_releases_on_press(&self) -> bool {
        self.active
            .iter()
            .flatten()
            .find(|entry| entry.is_modifier())
            .is_some_and(|entry| entry.trigger_for_key(true))
    }

    pub(crate) fn modifiers(&self, pressed: bool) -> ModifierCombination {
        let mut modifiers = ModifierCombination::new();
        for entry in self.active.iter().flatten() {
            if entry.is_tap_key() || (entry.is_modifier() && (pressed || entry.phase == StickyPhase::Held)) {
                modifiers |= entry.modifiers();
            }
        }
        modifiers
    }
}

// Return each selected handler future directly. A const branch inside the
// shared async dispatcher leaves all three handler futures in its state
// machine, while an extra async wrapper increases the executor task size.
trait ModifierStickyDispatch<const ENABLED: bool> {
    fn dispatch_modifier<const STICKY_LAYER: bool, const STICKY_TAP_KEY: bool>(
        keyboard: &mut Keyboard<'_, ENABLED, STICKY_LAYER, STICKY_TAP_KEY>,
        modifiers: ModifierCombination,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()>;
}

impl ModifierStickyDispatch<true> for () {
    fn dispatch_modifier<const STICKY_LAYER: bool, const STICKY_TAP_KEY: bool>(
        keyboard: &mut Keyboard<'_, true, STICKY_LAYER, STICKY_TAP_KEY>,
        modifiers: ModifierCombination,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        let policy = keyboard.keymap.sticky_key_profile(profile, StickyKeyShape::PureMod);
        keyboard.process_modifier_effect(modifiers, policy, event, event_time, source)
    }
}

// Keep the disabled implementations in the same explicit-future form as the
// enabled implementations. This dispatch boundary is flash-sensitive and was
// measured in this form when the unused handlers were eliminated.
#[allow(clippy::manual_async_fn)]
impl ModifierStickyDispatch<false> for () {
    fn dispatch_modifier<const STICKY_LAYER: bool, const STICKY_TAP_KEY: bool>(
        _keyboard: &mut Keyboard<'_, false, STICKY_LAYER, STICKY_TAP_KEY>,
        _modifiers: ModifierCombination,
        _profile: u8,
        _event: KeyboardEvent,
        _event_time: Instant,
        _source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        async { warn!("Sticky modifier omitted from this static firmware") }
    }
}

trait LayerStickyDispatch<const ENABLED: bool> {
    fn dispatch_layer<const STICKY_MODIFIER: bool, const STICKY_TAP_KEY: bool>(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, ENABLED, STICKY_TAP_KEY>,
        layer: u8,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()>;
}

impl LayerStickyDispatch<true> for () {
    fn dispatch_layer<const STICKY_MODIFIER: bool, const STICKY_TAP_KEY: bool>(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, true, STICKY_TAP_KEY>,
        layer: u8,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        let policy = keyboard.keymap.sticky_key_profile(profile, StickyKeyShape::Layer);
        keyboard.process_layer_effect(layer, policy, event, event_time, source)
    }
}

#[allow(clippy::manual_async_fn)]
impl LayerStickyDispatch<false> for () {
    fn dispatch_layer<const STICKY_MODIFIER: bool, const STICKY_TAP_KEY: bool>(
        _keyboard: &mut Keyboard<'_, STICKY_MODIFIER, false, STICKY_TAP_KEY>,
        _layer: u8,
        _profile: u8,
        _event: KeyboardEvent,
        _event_time: Instant,
        _source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        async { warn!("Sticky layer omitted from this static firmware") }
    }
}

trait TapKeyStickyDispatch<const ENABLED: bool> {
    fn dispatch_tap_key<const STICKY_MODIFIER: bool, const STICKY_LAYER: bool>(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, ENABLED>,
        key: HidKeyCode,
        modifiers: ModifierCombination,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()>;
}

impl TapKeyStickyDispatch<true> for () {
    fn dispatch_tap_key<const STICKY_MODIFIER: bool, const STICKY_LAYER: bool>(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, true>,
        key: HidKeyCode,
        modifiers: ModifierCombination,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        let policy = keyboard.keymap.sticky_key_profile(profile, StickyKeyShape::TapKey);
        keyboard.process_tap_key_effect(key, modifiers, policy, event, event_time, source)
    }
}

#[allow(clippy::manual_async_fn)]
impl TapKeyStickyDispatch<false> for () {
    fn dispatch_tap_key<const STICKY_MODIFIER: bool, const STICKY_LAYER: bool>(
        _keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, false>,
        _key: HidKeyCode,
        _modifiers: ModifierCombination,
        _profile: u8,
        _event: KeyboardEvent,
        _event_time: Instant,
        _source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        async { warn!("Sticky tap key omitted from this static firmware") }
    }
}

/// One internal dispatch bound for the three independently selected shapes.
/// The blanket implementation keeps the public `Keyboard` type usable with
/// default consts while concrete generated builds still monomorphize each
/// effect call independently.
pub(crate) trait StickyKeyDispatch<const STICKY_MODIFIER: bool, const STICKY_LAYER: bool, const STICKY_TAP_KEY: bool> {
    fn modifier(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>,
        modifiers: ModifierCombination,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()>;

    fn layer(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>,
        layer: u8,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()>;

    fn tap_key(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>,
        key: HidKeyCode,
        modifiers: ModifierCombination,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()>;
}

impl<const STICKY_MODIFIER: bool, const STICKY_LAYER: bool, const STICKY_TAP_KEY: bool>
    StickyKeyDispatch<STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY> for ()
where
    (): ModifierStickyDispatch<STICKY_MODIFIER>,
    (): LayerStickyDispatch<STICKY_LAYER>,
    (): TapKeyStickyDispatch<STICKY_TAP_KEY>,
{
    fn modifier(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>,
        modifiers: ModifierCombination,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        <() as ModifierStickyDispatch<STICKY_MODIFIER>>::dispatch_modifier(
            keyboard, modifiers, profile, event, event_time, source,
        )
    }

    fn layer(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>,
        layer: u8,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        <() as LayerStickyDispatch<STICKY_LAYER>>::dispatch_layer(keyboard, layer, profile, event, event_time, source)
    }

    fn tap_key(
        keyboard: &mut Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>,
        key: HidKeyCode,
        modifiers: ModifierCombination,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) -> impl Future<Output = ()> {
        <() as TapKeyStickyDispatch<STICKY_TAP_KEY>>::dispatch_tap_key(
            keyboard, key, modifiers, profile, event, event_time, source,
        )
    }
}

impl<const STICKY_MODIFIER: bool, const STICKY_LAYER: bool, const STICKY_TAP_KEY: bool>
    Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>
{
    pub(crate) async fn process_action_sticky_key(
        &mut self,
        action: Action,
        profile: u8,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) where
        (): StickyKeyDispatch<STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>,
    {
        // The action match is the narrow effect boundary. All variants store
        // and advance the same `StickyEntry` lifecycle.
        match action {
            Action::Modifier(modifiers) => {
                <() as StickyKeyDispatch<STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>>::modifier(
                    self, modifiers, profile, event, event_time, source,
                )
                .await
            }
            Action::LayerOn(layer) => {
                <() as StickyKeyDispatch<STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>>::layer(
                    self, layer, profile, event, event_time, source,
                )
                .await
            }
            Action::KeyWithModifier(key, modifiers) => {
                <() as StickyKeyDispatch<STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>>::tap_key(
                    self, key, modifiers, profile, event, event_time, source,
                )
                .await
            }
            _ => warn!("Unsupported Sticky Key action: {:?}", action),
        }
    }
}

impl<const STICKY_MODIFIER: bool, const STICKY_LAYER: bool, const STICKY_TAP_KEY: bool>
    Keyboard<'_, STICKY_MODIFIER, STICKY_LAYER, STICKY_TAP_KEY>
{
    // Keeping each effect future separate saves 128 bytes on the minimum
    // STM32F1 firmware by avoiding one merged async state machine.
    #[inline(never)]
    async fn process_modifier_effect(
        &mut self,
        modifiers: ModifierCombination,
        policy: StickyKeyPolicy,
        event: KeyboardEvent,
        event_time: Instant,
        producer_source: StickyKeySource,
    ) {
        if event.pressed {
            if let Some(index) = self.sticky_key_state.tap_key_index() {
                self.release_sticky_entry(index).await;
            }
            let modifier_index = self.sticky_key_state.modifier_index();
            if modifier_index.is_some_and(|index| {
                self.sticky_key_state.active[index].is_some_and(|entry| entry.is_double_tap(producer_source, policy))
            }) {
                self.release_sticky_entry(modifier_index.expect("modifier checked above"))
                    .await;
                return;
            }

            if let Some(index) = modifier_index {
                let insertion = self.sticky_key_state.active[index]
                    .as_mut()
                    .expect("modifier index must contain an entry")
                    .add_modifier_producer(modifiers, producer_source);
                if insertion == ModifierProducerInsert::Full {
                    return;
                }
                let entry = self.sticky_key_state.active[index]
                    .as_mut()
                    .expect("modifier index must contain an entry");
                entry.begin_modifier_press(producer_source, policy, event_time);
            } else if self
                .sticky_key_state
                .insert(StickyEntry::new_modifier(
                    modifiers,
                    event.pos,
                    producer_source,
                    policy,
                    event_time,
                ))
                .is_err()
            {
                warn!("No capacity for Sticky modifier action");
                return;
            }
            if policy.activate_on_keypress {
                self.send_keyboard_report_with_resolved_modifiers(true).await;
            }
        } else {
            if let Some(index) = self.sticky_key_state.modifier_index() {
                let transition = {
                    let entry = self.sticky_key_state.active[index]
                        .as_mut()
                        .expect("modifier index must contain an entry");
                    entry.on_exact_modifier_release(modifiers, producer_source)
                };
                self.finish_modifier_producer_release(index, transition, event_time)
                    .await;
            }
        }
    }

    async fn finish_modifier_producer_release(
        &mut self,
        index: usize,
        transition: ModifierProducerRelease,
        event_time: Instant,
    ) -> bool {
        let ModifierProducerRelease::Released {
            removed,
            old_effective,
            new_effective,
            producers_remain,
        } = transition
        else {
            return false;
        };

        let mut final_effective = new_effective;
        let mut release_entry = false;
        if !producers_remain {
            if self.physical_keys_down > 0 {
                release_entry = true;
            } else {
                let entry = self.sticky_key_state.active[index]
                    .as_mut()
                    .expect("modifier index must contain an entry");
                match entry.on_physical_release(None, event_time) {
                    PhysicalRelease::Latched => {
                        entry.retain_modifier(removed.modifiers);
                        final_effective = entry.modifiers();
                    }
                    PhysicalRelease::Released => release_entry = true,
                    PhysicalRelease::Ignored => {}
                }
            }
        }

        if release_entry {
            self.remove_sticky_modifier_entry(index);
            final_effective = ModifierCombination::new();
        }
        let current_non_sticky = self
            .resolve_modifier_breakdown(self.last_modifier_report.pressed_intent)
            .non_sticky;
        self.send_sticky_modifier_live_release(old_effective, final_effective, current_non_sticky)
            .await;
        true
    }

    #[inline(never)]
    async fn process_layer_effect(
        &mut self,
        layer: u8,
        policy: StickyKeyPolicy,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) {
        if event.pressed {
            if let Some(index) = self.sticky_key_state.tap_key_index() {
                self.release_sticky_entry(index).await;
            }
            if layer as usize >= self.keymap.num_layer() {
                // Keep KeyMap's established diagnostic, but never arm an invalid layer.
                self.keymap.activate_layer(layer);
                return;
            }
            if let Some(index) = self.sticky_key_state.layer_index() {
                let previous = self.sticky_key_state.active[index]
                    .as_ref()
                    .expect("layer index must contain an entry");
                if previous.is_double_tap(source, policy) {
                    self.release_sticky_entry(index).await;
                    return;
                }
                if previous.layer() == Some(layer) {
                    self.keymap.activate_layer(layer);
                    self.sticky_key_state.active[index]
                        .as_mut()
                        .expect("layer index must contain an entry")
                        .begin_press(source, event.pos, policy, event_time);
                    return;
                }
                self.release_sticky_entry(index).await;
            }
            if self
                .sticky_key_state
                .insert(StickyEntry::new(
                    Action::LayerOn(layer),
                    source,
                    event.pos,
                    policy,
                    event_time,
                ))
                .is_err()
            {
                warn!("No capacity for Sticky layer action");
                return;
            }
            self.keymap.activate_layer(layer);
        } else if let Some(index) = self.sticky_key_state.layer_index() {
            let release = self.sticky_key_state.active[index]
                .as_mut()
                .expect("layer index must contain an entry")
                .on_physical_release(Some(source), event_time)
                == PhysicalRelease::Released;
            if release {
                self.release_sticky_entry(index).await;
            }
        }
    }

    #[inline(never)]
    async fn process_tap_key_effect(
        &mut self,
        key: HidKeyCode,
        modifiers: ModifierCombination,
        policy: StickyKeyPolicy,
        event: KeyboardEvent,
        event_time: Instant,
        source: StickyKeySource,
    ) {
        let action = Action::KeyWithModifier(key, modifiers);

        if event.pressed {
            for index in 0..MAX_ACTIVE_STICKY_KEYS {
                if self.sticky_key_state.active[index].is_some_and(|entry| !entry.is_tap_key()) {
                    self.release_sticky_entry(index).await;
                }
            }

            let tap_key_index = self.sticky_key_state.tap_key_index();
            let same_tap_key = tap_key_index.is_some_and(|index| {
                self.sticky_key_state.active[index]
                    .is_some_and(|entry| entry.source == source && entry.action == action)
            });
            if same_tap_key
                && tap_key_index.is_some_and(|index| {
                    self.sticky_key_state.active[index].is_some_and(|entry| entry.is_double_tap(source, policy))
                })
            {
                self.release_sticky_entry(tap_key_index.expect("tap key checked above"))
                    .await;
                return;
            }
            if !same_tap_key && let Some(index) = tap_key_index {
                self.release_sticky_entry(index).await;
            }

            let mut deactivate = false;
            if let Some(index) = self.sticky_key_state.tap_key_index() {
                let entry = self.sticky_key_state.active[index]
                    .as_mut()
                    .expect("tap key index must contain an entry");
                entry.repeat_count = entry.repeat_count.saturating_add(1);
                if policy.max_repeat > 0 && entry.repeat_count > policy.max_repeat {
                    deactivate = true;
                } else {
                    entry.begin_press(source, event.pos, policy, event_time);
                }
            } else if self
                .sticky_key_state
                .insert(StickyEntry::new(action, source, event.pos, policy, event_time))
                .is_err()
            {
                warn!("No capacity for Sticky tap-key action");
                return;
            }

            if deactivate {
                self.release_sticky_entry(
                    self.sticky_key_state
                        .tap_key_index()
                        .expect("tap key must exist before repeat deactivation"),
                )
                .await;
            } else {
                self.process_action_key(key, event).await;
            }
        } else if let Some(index) = self.sticky_key_state.tap_key_index() {
            let release_physical_key = self.sticky_key_state.active[index].is_some_and(|entry| {
                entry.source == source
                    && matches!(entry.phase, StickyPhase::Pressed | StickyPhase::PressDeadlineInactive)
            });
            if release_physical_key {
                self.sticky_key_state.active[index]
                    .as_mut()
                    .expect("tap key index must contain an entry")
                    .on_physical_release(Some(source), event_time);
                self.process_action_key(key, event).await;
            }
        }
    }

    /// Release a tap-key entry around a foreign action at the ordering boundary
    /// required by its policy. Shape checks remain internal to Sticky Key.
    pub(crate) async fn prepare_sticky_key_for_action(
        &mut self,
        action: Action,
        event: KeyboardEvent,
    ) -> StickyKeyPostAction {
        let Some(index) = self.sticky_key_state.tap_key_index() else {
            return StickyKeyPostAction::None;
        };
        let preserves_tap_key = matches!(action, Action::Modifier(_))
            || matches!(action, Action::Key(KeyCode::Hid(key)) if key.is_modifier());
        let releases = self.sticky_key_state.active[index]
            .is_some_and(|entry| !preserves_tap_key && entry.trigger_for_key(event.pressed));
        if releases && event.pressed {
            self.release_sticky_entry(index).await;
            StickyKeyPostAction::None
        } else if releases {
            StickyKeyPostAction::Release
        } else {
            StickyKeyPostAction::None
        }
    }

    pub(crate) async fn finish_sticky_key_after_action(&mut self, post_action: StickyKeyPostAction) {
        if post_action == StickyKeyPostAction::Release
            && let Some(index) = self.sticky_key_state.tap_key_index()
        {
            self.release_sticky_entry(index).await;
        }
    }

    /// Apply a foreign key event uniformly to all active entries.
    fn update_sticky_key(&mut self, event: KeyboardEvent) -> StickyKeyUpdate {
        let mut update = StickyKeyUpdate::default();

        for index in 0..MAX_ACTIVE_STICKY_KEYS {
            let Some(entry) = self.sticky_key_state.active[index].as_mut() else {
                continue;
            };
            if entry.is_tap_key() {
                continue;
            }
            match entry.phase {
                StickyPhase::Pressed | StickyPhase::PressDeadlineInactive | StickyPhase::HoldQualified => {
                    entry.mark_foreign_key()
                }
                StickyPhase::Latched if entry.trigger_for_key(event.pressed) => {
                    let entry = self.sticky_key_state.active[index]
                        .take()
                        .expect("active entry checked above");
                    match entry.action {
                        Action::Modifier(_) => {
                            update.modifier_consumed = true;
                        }
                        Action::LayerOn(layer) => {
                            self.keymap.deactivate_layer_if_active(layer);
                        }
                        _ => unreachable!("tap-key and unsupported entries are not foreign-key candidates"),
                    }
                }
                StickyPhase::Latched | StickyPhase::Held => {}
            }
        }
        update
    }

    /// Finish Sticky Key handling after a concrete key report has been sent.
    /// The result tells the caller whether a balancing keyboard report is
    /// needed; modifier/layer/tap-key details remain inside this module.
    pub(crate) fn finish_sticky_key_for_key(&mut self, event: KeyboardEvent, is_basic_keyboard_key: bool) -> bool {
        let modifier_was_host_visible = self.last_modifier_report.sticky_contributed.into_bits() != 0;
        let modifier_releases_on_press = self.sticky_key_state.modifier_releases_on_press();
        let update = self.update_sticky_key(event);

        (is_basic_keyboard_key && event.pressed && modifier_releases_on_press && update.modifier_consumed)
            || (!is_basic_keyboard_key && update.modifier_consumed && modifier_was_host_visible)
    }

    pub(crate) async fn release_sticky_key_on_layer_event(&mut self, event: StickyKeyReleaseMode) {
        for index in 0..MAX_ACTIVE_STICKY_KEYS {
            if self.sticky_key_state.active[index].is_some_and(|entry| entry.policy.release_mode.intersects(event)) {
                self.release_sticky_entry(index).await;
            }
        }
    }

    pub(crate) async fn release_sticky_key_if_active_on_timeout(&mut self) {
        let now = Instant::now();
        for index in 0..MAX_ACTIVE_STICKY_KEYS {
            let release = self.sticky_key_state.active[index]
                .as_mut()
                .is_some_and(|entry| entry.deadline_disposition(now) == DeadlineDisposition::Release);
            if release {
                self.release_sticky_entry(index).await;
            }
        }
    }

    /// Remove one entry and release the concrete device effect it owns.
    async fn release_sticky_entry(&mut self, index: usize) {
        let Some(entry) = self.sticky_key_state.active[index].take() else {
            return;
        };
        match entry.action {
            Action::Modifier(modifiers) => {
                let StickyEffectState::Modifier(_) = entry.effect_state else {
                    unreachable!("modifier action must own modifier effect state");
                };
                let current_non_sticky = self
                    .resolve_modifier_breakdown(self.last_modifier_report.pressed_intent)
                    .non_sticky;
                let removed_from_host = modifiers & self.last_modifier_report.sticky_contributed & !current_non_sticky;
                if removed_from_host.into_bits() != 0 {
                    self.send_keyboard_report_with_resolved_modifiers(false).await;
                }
            }
            Action::LayerOn(layer) => {
                self.keymap.deactivate_layer_if_active(layer);
            }
            Action::KeyWithModifier(key, _) => {
                let StickyEffectState::ActionOnly(release_pos) = entry.effect_state else {
                    unreachable!("tap-key action must own its release position");
                };
                if matches!(entry.phase, StickyPhase::Pressed | StickyPhase::PressDeadlineInactive) {
                    self.process_action_key(
                        key,
                        KeyboardEvent {
                            pressed: false,
                            pos: release_pos,
                        },
                    )
                    .await;
                } else {
                    self.send_keyboard_report_with_resolved_modifiers(false).await;
                }
            }
            _ => unreachable!("unsupported action cannot enter Sticky Key state"),
        }
    }

    fn remove_sticky_modifier_entry(&mut self, index: usize) {
        let Some(entry) = self.sticky_key_state.active[index].take() else {
            return;
        };
        let StickyEffectState::Modifier(_) = entry.effect_state else {
            unreachable!("modifier index must contain a modifier entry");
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_dispatch_set<const MODIFIER: bool, const LAYER: bool, const TAP_KEY: bool>()
    where
        (): StickyKeyDispatch<MODIFIER, LAYER, TAP_KEY>,
    {
    }

    #[test]
    fn every_sticky_shape_set_has_static_dispatch() {
        assert_dispatch_set::<false, false, false>();
        assert_dispatch_set::<false, false, true>();
        assert_dispatch_set::<false, true, false>();
        assert_dispatch_set::<false, true, true>();
        assert_dispatch_set::<true, false, false>();
        assert_dispatch_set::<true, false, true>();
        assert_dispatch_set::<true, true, false>();
        assert_dispatch_set::<true, true, true>();
    }

    fn pos(col: u8) -> KeyboardEventPos {
        KeyboardEventPos::key_pos(col, 0)
    }

    fn direct(col: u8) -> StickyKeySource {
        StickyKeySource::Direct(pos(col))
    }

    fn release_producer(
        effect: &mut StickyModifierEffect,
        modifiers: ModifierCombination,
        source: StickyKeySource,
    ) -> ModifierProducerRelease {
        effect.on_exact_release(modifiers, source, effect.effective())
    }

    fn policy(release_mode: StickyKeyReleaseMode) -> StickyKeyPolicy {
        StickyKeyPolicy {
            timeout: Duration::from_secs(1),
            activate_on_keypress: false,
            release_after_hold: StickyKeyHoldDuration::DISABLED,
            max_repeat: 0,
            release_mode,
        }
    }

    fn modifier_entry(
        modifiers: ModifierCombination,
        source: KeyboardEventPos,
        policy: StickyKeyPolicy,
        pressed_at: Instant,
    ) -> StickyEntry {
        StickyEntry::new(
            Action::Modifier(modifiers),
            StickyKeySource::Direct(source),
            source,
            policy,
            pressed_at,
        )
    }

    #[test]
    fn modifier_effect_counts_overlapping_physical_producers() {
        let pressed_at = Instant::now();
        let mut latch = modifier_entry(
            ModifierCombination::LCTRL,
            pos(0),
            policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE),
            pressed_at,
        );
        latch.add_modifier_producer(ModifierCombination::LSHIFT, direct(1));
        latch.begin_modifier_press(direct(1), policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE), pressed_at);

        assert_eq!(
            latch.on_exact_modifier_release(ModifierCombination::LCTRL, direct(0)),
            ModifierProducerRelease::Released {
                removed: ModifierProducer {
                    source: direct(0),
                    modifiers: ModifierCombination::LCTRL,
                },
                old_effective: ModifierCombination::LCTRL | ModifierCombination::LSHIFT,
                new_effective: ModifierCombination::LSHIFT,
                producers_remain: true,
            }
        );
        assert_eq!(latch.phase, StickyPhase::PressDeadlineInactive);
        let final_release = latch.on_exact_modifier_release(ModifierCombination::LSHIFT, direct(1));
        assert!(matches!(
            final_release,
            ModifierProducerRelease::Released {
                new_effective,
                producers_remain: false,
                ..
            } if new_effective == ModifierCombination::new()
        ));
        assert_eq!(
            latch.on_physical_release(None, Instant::now()),
            PhysicalRelease::Latched
        );
        latch.retain_modifier(ModifierCombination::LSHIFT);
        assert_eq!(latch.phase, StickyPhase::Latched);
        assert_eq!(latch.modifiers(), ModifierCombination::LSHIFT);
    }

    #[test]
    fn held_latch_releases_after_last_physical_producer() {
        let pressed_at = Instant::now();
        let mut latch = modifier_entry(
            ModifierCombination::LCTRL,
            pos(0),
            policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE),
            pressed_at,
        );
        latch.add_modifier_producer(ModifierCombination::LSHIFT, direct(1));
        latch.begin_modifier_press(direct(1), policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE), pressed_at);
        latch.mark_foreign_key();

        assert!(matches!(
            latch.on_exact_modifier_release(ModifierCombination::LCTRL, direct(0)),
            ModifierProducerRelease::Released {
                producers_remain: true,
                ..
            }
        ));
        assert!(matches!(
            latch.on_exact_modifier_release(ModifierCombination::LSHIFT, direct(1)),
            ModifierProducerRelease::Released {
                producers_remain: false,
                ..
            }
        ));
        assert_eq!(
            latch.on_physical_release(None, Instant::now()),
            PhysicalRelease::Released
        );
    }

    #[test]
    fn disabled_hold_threshold_has_no_press_deadline() {
        let pressed_at = Instant::now();
        let mut latch = modifier_entry(
            ModifierCombination::LCTRL,
            pos(0),
            policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE),
            pressed_at,
        );

        assert_eq!(latch.phase, StickyPhase::PressDeadlineInactive);
        assert_eq!(latch.deadline(), None);
        assert_eq!(latch.timing_marker, Some(pressed_at));
        assert_eq!(
            latch.on_physical_release(None, pressed_at + Duration::from_secs(2)),
            PhysicalRelease::Latched
        );
        assert_eq!(latch.phase, StickyPhase::Latched);
        assert!(latch.timing_marker.is_some());
    }

    #[test]
    fn configured_hold_threshold_releases_after_deferred_timeout() {
        let mut hold_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        hold_policy.release_after_hold = StickyKeyHoldDuration::from_duration(Duration::from_millis(300));
        let mut latch = modifier_entry(ModifierCombination::LCTRL, pos(0), hold_policy, Instant::now());
        let deadline = latch.timing_marker.unwrap();

        assert_eq!(latch.deadline_disposition(deadline), DeadlineDisposition::Deferred);
        assert_eq!(latch.phase, StickyPhase::HoldQualified);
        assert_eq!(latch.on_physical_release(None, deadline), PhysicalRelease::Released);
        assert_eq!(latch.timing_marker, None);
    }

    #[test]
    fn configured_hold_threshold_is_inclusive() {
        let mut hold_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        hold_policy.release_after_hold = StickyKeyHoldDuration::from_duration(Duration::from_millis(300));
        let mut short_press = modifier_entry(ModifierCombination::LCTRL, pos(0), hold_policy, Instant::now());
        let threshold = short_press.timing_marker.unwrap();
        let just_before_threshold = threshold - Duration::from_millis(1);

        assert_eq!(
            short_press.on_physical_release(None, just_before_threshold),
            PhysicalRelease::Latched
        );
        assert_eq!(short_press.phase, StickyPhase::Latched);
        assert_eq!(
            short_press.timing_marker,
            Some(just_before_threshold + hold_policy.timeout)
        );

        let mut threshold_press = modifier_entry(ModifierCombination::LCTRL, pos(0), hold_policy, Instant::now());
        let threshold = threshold_press.timing_marker.unwrap();

        assert_eq!(
            threshold_press.on_physical_release(None, threshold),
            PhysicalRelease::Released
        );
        assert_eq!(threshold_press.timing_marker, None);
    }

    #[test]
    fn hold_threshold_may_exceed_the_latched_timeout() {
        let mut hold_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        hold_policy.timeout = Duration::from_millis(300);
        hold_policy.release_after_hold = StickyKeyHoldDuration::from_duration(Duration::from_secs(1));
        let pressed_at = Instant::now();
        let released_at = pressed_at + Duration::from_millis(600);
        let mut latch = modifier_entry(ModifierCombination::LCTRL, pos(0), hold_policy, pressed_at);

        assert_eq!(latch.on_physical_release(None, released_at), PhysicalRelease::Latched);
        assert_eq!(latch.phase, StickyPhase::Latched);
        assert_eq!(latch.timing_marker, Some(released_at + hold_policy.timeout));
    }

    #[test]
    fn hold_threshold_uses_original_buffered_press_time() {
        let mut hold_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        hold_policy.release_after_hold = StickyKeyHoldDuration::from_duration(Duration::from_millis(300));
        let pressed_at = Instant::now();
        let dispatched_at = pressed_at + Duration::from_millis(250);
        let mut latch = modifier_entry(ModifierCombination::LCTRL, pos(0), hold_policy, pressed_at);

        assert_eq!(
            latch.on_physical_release(None, dispatched_at + Duration::from_millis(50)),
            PhysicalRelease::Released
        );
    }

    #[test]
    fn overlapping_modifier_preserves_chord_hold_start() {
        let mut hold_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        hold_policy.release_after_hold = StickyKeyHoldDuration::from_duration(Duration::from_millis(300));
        let first_press = Instant::now();
        let second_press = first_press + Duration::from_millis(250);
        let release_time = first_press + Duration::from_millis(400);
        let mut latch = modifier_entry(ModifierCombination::LCTRL, pos(0), hold_policy, first_press);
        latch.add_modifier_producer(ModifierCombination::LSHIFT, direct(1));
        latch.begin_modifier_press(direct(1), hold_policy, second_press);

        assert!(matches!(
            latch.on_exact_modifier_release(ModifierCombination::LSHIFT, direct(1)),
            ModifierProducerRelease::Released {
                producers_remain: true,
                ..
            }
        ));
        assert!(matches!(
            latch.on_exact_modifier_release(ModifierCombination::LCTRL, direct(0)),
            ModifierProducerRelease::Released {
                producers_remain: false,
                ..
            }
        ));
        assert_eq!(latch.on_physical_release(None, release_time), PhysicalRelease::Released);

        let mut reverse_release = modifier_entry(ModifierCombination::LCTRL, pos(0), hold_policy, first_press);
        reverse_release.add_modifier_producer(ModifierCombination::LSHIFT, direct(1));
        reverse_release.begin_modifier_press(direct(1), hold_policy, second_press);
        assert!(matches!(
            reverse_release.on_exact_modifier_release(ModifierCombination::LCTRL, direct(0)),
            ModifierProducerRelease::Released {
                producers_remain: true,
                ..
            }
        ));
        assert!(matches!(
            reverse_release.on_exact_modifier_release(ModifierCombination::LSHIFT, direct(1)),
            ModifierProducerRelease::Released {
                producers_remain: false,
                ..
            }
        ));
        assert_eq!(
            reverse_release.on_physical_release(None, release_time),
            PhysicalRelease::Released
        );
    }

    #[test]
    fn latest_modifier_policy_uses_chord_hold_start() {
        let mut first_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        first_policy.release_after_hold = StickyKeyHoldDuration::from_duration(Duration::from_millis(300));
        let mut latest_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        latest_policy.release_after_hold = StickyKeyHoldDuration::from_duration(Duration::from_millis(500));
        let first_press = Instant::now();
        let mut latch = modifier_entry(ModifierCombination::LCTRL, pos(0), first_policy, first_press);
        latch.add_modifier_producer(ModifierCombination::LSHIFT, direct(1));
        latch.begin_modifier_press(direct(1), latest_policy, first_press + Duration::from_millis(250));

        assert_eq!(
            latch.on_physical_release(None, first_press + Duration::from_millis(400)),
            PhysicalRelease::Latched
        );

        let mut timeout_first = modifier_entry(ModifierCombination::LCTRL, pos(0), first_policy, first_press);
        assert_eq!(
            timeout_first.deadline_disposition(first_press + Duration::from_millis(300)),
            DeadlineDisposition::Deferred
        );
        timeout_first.add_modifier_producer(ModifierCombination::LSHIFT, direct(1));
        timeout_first.begin_modifier_press(direct(1), latest_policy, first_press + Duration::from_millis(400));
        assert_eq!(
            timeout_first.on_physical_release(None, first_press + Duration::from_millis(450)),
            PhysicalRelease::Latched
        );
    }

    #[test]
    fn mixed_modifier_profiles_are_independent_of_deadline_poll_order() {
        let mut enabled_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        enabled_policy.release_after_hold = StickyKeyHoldDuration::from_duration(Duration::from_millis(500));
        let mut disabled_policy = policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE);
        disabled_policy.timeout = Duration::from_millis(300);
        let first_press = Instant::now();

        for poll_first in [false, true] {
            let mut disabled_then_enabled =
                modifier_entry(ModifierCombination::LCTRL, pos(0), disabled_policy, first_press);
            if poll_first {
                assert_eq!(
                    disabled_then_enabled.deadline_disposition(first_press + Duration::from_millis(300)),
                    DeadlineDisposition::Pending
                );
            }
            disabled_then_enabled.add_modifier_producer(ModifierCombination::LSHIFT, direct(1));
            disabled_then_enabled.begin_modifier_press(
                direct(1),
                enabled_policy,
                first_press + Duration::from_millis(400),
            );
            assert_eq!(
                disabled_then_enabled.on_physical_release(None, first_press + Duration::from_millis(450)),
                PhysicalRelease::Latched
            );

            let mut enabled_then_disabled =
                modifier_entry(ModifierCombination::LCTRL, pos(0), enabled_policy, first_press);
            if poll_first {
                assert_eq!(
                    enabled_then_disabled.deadline_disposition(first_press + Duration::from_millis(500)),
                    DeadlineDisposition::Deferred
                );
            }
            enabled_then_disabled.add_modifier_producer(ModifierCombination::LSHIFT, direct(1));
            enabled_then_disabled.begin_modifier_press(
                direct(1),
                disabled_policy,
                first_press + Duration::from_millis(600),
            );
            assert_eq!(
                enabled_then_disabled.on_physical_release(None, first_press + Duration::from_millis(650)),
                PhysicalRelease::Latched
            );
        }
    }

    #[test]
    fn buffered_foreign_press_claims_latch_until_action_resolves() {
        let mut latch = modifier_entry(
            ModifierCombination::LCTRL,
            pos(0),
            policy(StickyKeyReleaseMode::OTHER_KEY_PRESS),
            Instant::now(),
        );
        assert_eq!(
            latch.on_physical_release(None, Instant::now()),
            PhysicalRelease::Latched
        );

        latch.claim_buffered_press(pos(1));
        assert!(latch.buffered_claim);
        assert_eq!(latch.timing_marker, None);

        latch.finish_buffered_claim();
        assert!(!latch.buffered_claim);
        assert!(latch.timing_marker.is_some());
    }

    #[test]
    fn active_state_stores_actions_in_one_uniform_collection() {
        let mut state = StickyKeyState::default();
        let modifier = modifier_entry(
            ModifierCombination::LCTRL,
            pos(0),
            policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE),
            Instant::now(),
        );
        let layer = StickyEntry::new(
            Action::LayerOn(2),
            direct(1),
            pos(1),
            policy(StickyKeyReleaseMode::LAYER_EXIT),
            Instant::now(),
        );

        state.insert(modifier).unwrap();
        state.insert(layer).unwrap();

        assert!(
            state
                .active
                .iter()
                .flatten()
                .any(|entry| { entry.action == Action::Modifier(ModifierCombination::LCTRL) })
        );
        assert!(
            state
                .active
                .iter()
                .flatten()
                .any(|entry| entry.action == Action::LayerOn(2))
        );

        let overflow = StickyEntry::new(
            Action::KeyWithModifier(HidKeyCode::A, ModifierCombination::LSHIFT),
            direct(2),
            pos(2),
            policy(StickyKeyReleaseMode::OTHER_KEY_RELEASE),
            Instant::now(),
        );
        assert_eq!(state.insert(overflow).unwrap_err().action, overflow.action);
    }

    #[test]
    fn stale_direct_and_combo_releases_cannot_remove_a_current_same_mask_producer() {
        let modifiers = ModifierCombination::LSHIFT;
        let mut direct_effect = StickyModifierEffect::new(modifiers, direct(1));

        assert_eq!(
            release_producer(&mut direct_effect, modifiers, StickyKeySource::Combo(0)),
            ModifierProducerRelease::Unmatched
        );
        assert_eq!(direct_effect.effective(), modifiers);

        let mut combo_effect = StickyModifierEffect::new(modifiers, StickyKeySource::Combo(1));
        assert_eq!(
            release_producer(&mut combo_effect, modifiers, direct(0)),
            ModifierProducerRelease::Unmatched
        );
        assert_eq!(
            release_producer(&mut combo_effect, modifiers, StickyKeySource::Combo(0)),
            ModifierProducerRelease::Unmatched
        );
        assert_eq!(combo_effect.effective(), modifiers);
    }

    #[test]
    fn same_mask_combo_siblings_release_only_their_own_slot() {
        let modifiers = ModifierCombination::LSHIFT;
        let mut effect = StickyModifierEffect::new(modifiers, StickyKeySource::Combo(0));
        assert_eq!(
            effect.begin_press(modifiers, StickyKeySource::Combo(3)),
            ModifierProducerInsert::Added
        );

        assert!(matches!(
            release_producer(&mut effect, modifiers, StickyKeySource::Combo(0)),
            ModifierProducerRelease::Released {
                removed: ModifierProducer {
                    source: StickyKeySource::Combo(0),
                    modifiers: removed_modifiers,
                },
                producers_remain: true,
                ..
            } if removed_modifiers == modifiers
        ));
        assert!(matches!(
            release_producer(&mut effect, modifiers, StickyKeySource::Combo(3)),
            ModifierProducerRelease::Released {
                removed: ModifierProducer {
                    source: StickyKeySource::Combo(3),
                    modifiers: removed_modifiers,
                },
                producers_remain: false,
                ..
            } if removed_modifiers == modifiers
        ));
    }

    #[test]
    fn eight_combo_slots_report_the_exact_removed_owner() {
        let modifiers = ModifierCombination::LCTRL;
        let mut effect = StickyModifierEffect::new(modifiers, StickyKeySource::Combo(0));
        for index in 1..MAX_STICKY_MODIFIER_PRODUCERS {
            assert_eq!(
                effect.begin_press(modifiers, StickyKeySource::Combo(index as u16)),
                ModifierProducerInsert::Added
            );
        }

        for index in [4, 0, 7, 2, 5, 1, 6, 3] {
            assert!(matches!(
                release_producer(&mut effect, modifiers, StickyKeySource::Combo(index)),
                ModifierProducerRelease::Released {
                    removed: ModifierProducer {
                        source: StickyKeySource::Combo(removed_index),
                        modifiers: removed_modifiers,
                    },
                    ..
                } if removed_index == index && removed_modifiers == modifiers
            ));
        }
    }

    #[test]
    fn modifier_producer_insertion_is_atomic_at_capacity() {
        let mut effect = StickyModifierEffect::new(ModifierCombination::LCTRL, direct(0));
        for index in 1..MAX_STICKY_MODIFIER_PRODUCERS {
            assert_eq!(
                effect.begin_press(ModifierCombination::LSHIFT, direct(index as u8)),
                ModifierProducerInsert::Added
            );
        }
        let producers = effect.producers;
        let effective = effect.effective();

        assert_eq!(
            effect.begin_press(ModifierCombination::LALT, direct(9)),
            ModifierProducerInsert::Full
        );
        assert_eq!(effect.producers, producers);
        assert_eq!(effect.effective(), effective);
    }

    #[test]
    fn rejected_releases_beyond_both_old_capacities_preserve_accepted_owners() {
        let modifiers = ModifierCombination::LCTRL;
        for combo_sources in [false, true] {
            let source = |index: usize| {
                if combo_sources {
                    StickyKeySource::Combo(index as u16)
                } else {
                    direct(index as u8)
                }
            };
            let mut effect = StickyModifierEffect::new(modifiers, source(0));
            for index in 1..MAX_STICKY_MODIFIER_PRODUCERS {
                assert_eq!(
                    effect.begin_press(modifiers, source(index)),
                    ModifierProducerInsert::Added
                );
            }
            for index in MAX_STICKY_MODIFIER_PRODUCERS..(MAX_STICKY_MODIFIER_PRODUCERS * 3) {
                assert_eq!(
                    effect.begin_press(modifiers, source(index)),
                    ModifierProducerInsert::Full
                );
            }

            for index in (MAX_STICKY_MODIFIER_PRODUCERS..(MAX_STICKY_MODIFIER_PRODUCERS * 3)).rev() {
                assert_eq!(
                    release_producer(&mut effect, modifiers, source(index)),
                    ModifierProducerRelease::Unmatched
                );
            }
            assert_eq!(effect.effective(), modifiers);
            assert_eq!(effect.producers.iter().flatten().count(), MAX_STICKY_MODIFIER_PRODUCERS);
        }
    }

    #[test]
    fn direct_and_combo_identity_use_the_same_release_transition() {
        let modifiers = ModifierCombination::LCTRL;
        let mut direct_effect = StickyModifierEffect::new(modifiers, direct(0));
        let mut combo_effect = StickyModifierEffect::new(modifiers, StickyKeySource::Combo(3));

        for release in [
            release_producer(&mut direct_effect, modifiers, direct(0)),
            release_producer(&mut combo_effect, modifiers, StickyKeySource::Combo(3)),
        ] {
            assert!(matches!(
                release,
                ModifierProducerRelease::Released {
                    old_effective,
                    new_effective,
                    producers_remain: false,
                    ..
                } if old_effective == modifiers && new_effective == ModifierCombination::new()
            ));
        }
    }

    #[test]
    fn overlapping_modifier_producers_keep_shared_bits() {
        let mut effect = StickyModifierEffect::new(ModifierCombination::LCTRL, direct(0));
        effect.begin_press(ModifierCombination::LCTRL | ModifierCombination::LSHIFT, direct(1));

        assert!(matches!(
            release_producer(&mut effect, ModifierCombination::LCTRL, direct(0)),
            ModifierProducerRelease::Released {
                old_effective,
                new_effective,
                producers_remain: true,
                ..
            } if old_effective == ModifierCombination::LCTRL | ModifierCombination::LSHIFT
                && new_effective == ModifierCombination::LCTRL | ModifierCombination::LSHIFT
        ));
    }

    #[test]
    fn retained_modifiers_combine_with_live_producers() {
        let mut effect = StickyModifierEffect::new(ModifierCombination::LSHIFT, direct(1));
        effect.retain(ModifierCombination::LCTRL);

        assert_eq!(
            effect.effective(),
            ModifierCombination::LCTRL | ModifierCombination::LSHIFT
        );
        assert!(matches!(
            release_producer(&mut effect, ModifierCombination::LSHIFT, direct(1)),
            ModifierProducerRelease::Released {
                old_effective,
                new_effective,
                producers_remain: false,
                ..
            } if old_effective == ModifierCombination::LCTRL | ModifierCombination::LSHIFT
                && new_effective == ModifierCombination::LCTRL
        ));
    }
}
