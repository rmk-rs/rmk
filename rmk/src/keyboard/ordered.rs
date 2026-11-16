//! Ordered execution over the existing held-key records.

use embassy_time::Instant;
use rmk_types::action::{Action, KeyAction};
use rmk_types::keycode::KeyCode;
use rmk_types::morse::{MorseMode, MorsePattern, TAP};

use super::held_buffer::{
    AdmissionError, DeliveryPhase, EventOrder, EventStamp, HeldKey, KeyState, OrderedPolicy, PendingEvent,
};
use super::{HeldKeyDecision, KeyBehaviorDecision, Keyboard};
use crate::event::{KeyboardEvent, KeyboardEventSource};

pub(super) enum ScheduleRequest {
    Drain,
    ObservePending,
    Observe(EventOrder),
}

impl Keyboard<'_> {
    fn ordered_owner(&self) -> Option<&HeldKey> {
        self.held_buffer
            .keys
            .iter()
            .filter(|key| key.state == KeyState::OrderedPending)
            .min_by_key(|key| key.capture.unwrap().down.order)
    }

    fn ordered_resolution(&self, owner: &HeldKey, now: Instant) -> Option<Action> {
        let KeyAction::TapHold(tap, hold, _) = owner.action else {
            unreachable!()
        };
        let down = owner.capture.unwrap().down;
        let policy = owner.capture.unwrap().policy.unwrap();
        let mode = policy.mode;
        let unilateral = policy.unilateral;
        let hand = match owner.event.source {
            KeyboardEventSource::Key(pos) => self.keymap.hand_at(pos.row as usize, pos.col as usize),
            _ => unreachable!(),
        };
        let mut decision = owner
            .capture
            .unwrap()
            .up
            .filter(|up| up.at < owner.timeout_time)
            .map(|up| (up.order, tap));
        let mut first_release = None;
        for key in &self.held_buffer.keys {
            let Some(capture) = key.capture.as_ref() else { continue };
            if capture.phase == DeliveryPhase::Consumed
                || key.state == KeyState::WaitingCombo
                || !key.event.source.is_physical()
                || capture.down.order <= down.order
            {
                continue;
            }
            let stamp = if mode == MorseMode::HoldOnOtherPress {
                capture.down
            } else {
                let Some(up) = capture.up else { continue };
                up
            };
            if stamp.at >= owner.timeout_time {
                continue;
            }
            let same_hand = unilateral
                && mode != MorseMode::HoldOnOtherPress
                && match key.event.source {
                    KeyboardEventSource::Key(pos) => {
                        hand.is_same_side(self.keymap.hand_at(pos.row as usize, pos.col as usize))
                    }
                    _ => false,
                };
            if mode == MorseMode::Normal {
                if first_release.is_none_or(|(order, _)| stamp.order < order) {
                    first_release = Some((stamp.order, same_hand));
                }
            } else if decision.is_none_or(|(order, _)| stamp.order < order) {
                decision = Some((
                    stamp.order,
                    if mode == MorseMode::PermissiveHold && unilateral && same_hand {
                        tap
                    } else {
                        hold
                    },
                ));
            }
        }
        if let Some((_, action)) = decision {
            return Some(action);
        }
        (now >= owner.timeout_time).then_some(if unilateral && first_release.is_some_and(|(_, same_hand)| same_hand) {
            tap
        } else {
            hold
        })
    }

    pub(super) fn ordered_next_deadline(&self) -> Option<Instant> {
        if let Some(owner) = self.ordered_owner() {
            Some(if self.ordered_resolution(owner, Instant::now()).is_some() {
                Instant::now()
            } else {
                owner.timeout_time
            })
        } else {
            self.held_buffer.next_pending_event().map(|_| Instant::now())
        }
    }

    pub(super) fn ordered_protects_oneshot(&self) -> bool {
        self.held_buffer.keys.iter().any(|key| {
            key.state == KeyState::OrderedPending || key.capture.is_some_and(|c| c.phase == DeliveryPhase::Queued)
        })
    }

    pub(super) fn process_ordered_input(
        &mut self,
        event: KeyboardEvent,
        at: Instant,
    ) -> impl core::future::Future<Output = ()> + '_ {
        let action = if event.pressed {
            self.keymap.peek_action(event.source)
        } else {
            self.held_buffer
                .keys
                .iter()
                .find(|key| key.event.source == event.source && key.capture.is_some_and(|c| c.up.is_none()))
                .and_then(|key| key.capture.map(|c| c.admission_action))
                .unwrap_or(KeyAction::No)
        };
        async move {
            let admitted = match self.held_buffer.admit(event, action, at, self.last_press_time) {
                Err(AdmissionError::Full) => {
                    if let Some(owner) = self.ordered_owner().copied() {
                        warn!("Held buffer full; resolving oldest ordered tap-hold as tap");
                        let KeyAction::TapHold(tap, _, _) = owner.action else {
                            unreachable!()
                        };
                        self.resolve_ordered_owner(owner, tap).await;
                        self.drain_ordered_keys().await;
                    }
                    self.held_buffer.admit(event, action, at, self.last_press_time)
                }
                result => result,
            };
            let generation = match admitted {
                Ok(Some(generation)) => generation,
                Ok(None) => return,
                Err(AdmissionError::Full) => {
                    warn!("Held buffer full; rejecting fresh press before side effects");
                    return;
                }
            };
            if self.combo_on {
                self.process_combo(&action, event, at, Some(generation)).await;
            }
            self.drain_ordered_keys().await;
        }
    }

    pub(super) fn enqueue_action(&mut self, action: KeyAction, event: KeyboardEvent, at: Instant) -> bool {
        match self.held_buffer.admit(event, action, at, self.last_press_time) {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(AdmissionError::Full) => {
                error!("Unable to reserve synthetic action ownership: held buffer full");
                false
            }
        }
    }

    fn next_observation(&self) -> Option<(EventStamp, KeyboardEvent, KeyAction)> {
        let mut next: Option<(EventStamp, KeyboardEvent, KeyAction)> = None;
        for owner in &self.held_buffer.keys {
            let Some(c) = owner.capture.as_ref() else { continue };
            if c.phase == DeliveryPhase::Queued
                || c.policy.is_some()
                || !owner.action.is_morse()
                || !matches!(owner.state, KeyState::Pressed(_) | KeyState::Released(_))
            {
                continue;
            }
            for input in &self.held_buffer.keys {
                let Some(captured) = input.capture.as_ref() else {
                    continue;
                };
                if captured.phase == DeliveryPhase::Consumed || input.state == KeyState::WaitingCombo {
                    continue;
                }
                for (stamp, pressed) in [(Some(captured.down), true), (captured.up, false)] {
                    let Some(stamp) = stamp else { continue };
                    // Delayed combo output precedes its interrupting input despite admission order.
                    let causal = c.deliver_before == Some(stamp.order);
                    if (causal || (captured.down.order > c.down.order && stamp.order > c.observed_through))
                        && c.up.is_none_or(|up| stamp.order < up.order)
                        && next.as_ref().is_none_or(|(best, _, _)| stamp.order < best.order)
                    {
                        next = Some((
                            stamp,
                            KeyboardEvent {
                                pressed,
                                source: input.event.source,
                            },
                            captured.admission_action,
                        ));
                    }
                }
            }
        }
        next
    }

    fn resolve_ordered_owner(&mut self, owner: HeldKey, action: Action) -> impl core::future::Future<Output = ()> + '_ {
        let down = owner.capture.unwrap().down;
        let event = owner.event;
        async move {
            self.logical_time = Some(down.at);
            self.process_key_action_normal(action, event).await;
            if let Some(index) = self.held_buffer.generation_index(down.order) {
                self.held_buffer.keys[index].state = KeyState::ProcessedButReleaseNotReportedYet(action);
            }
            self.logical_time = None;
        }
    }

    pub(super) async fn ordered_combo_timeout(
        &mut self,
        event: KeyboardEvent,
        action: KeyAction,
        timeout_time: Instant,
        generation: EventOrder,
    ) {
        let mut release = event;
        release.pressed = false;
        if self.trigger_delayed_combo(&action, release, timeout_time, None) {
            self.run_schedule(ScheduleRequest::ObservePending).await;
        }
        if let Some(index) = self.held_buffer.generation_index(generation)
            && self.held_buffer.keys[index].state == KeyState::WaitingCombo
        {
            self.held_buffer.keys[index].state = KeyState::Deferred;
            self.run_schedule(ScheduleRequest::Observe(generation)).await;
            self.keymap.with_combos_mut(|combos| {
                for combo in combos
                    .iter_mut()
                    .flatten()
                    .filter(|combo| !combo.is_triggered() && combo.config.contains(&action))
                {
                    combo.reset();
                }
            });
        }
    }

    pub(super) fn fire_ordered_combo(&mut self, index: usize, at: Instant, deliver_before: Option<EventOrder>) -> bool {
        let Some((output, chord)) = self.keymap.with_combos(|combos| {
            combos[index]
                .as_ref()
                .map(|combo| (combo.config.output, combo.config.actions.clone()))
        }) else {
            return false;
        };
        let admission = self.held_buffer.admit(
            KeyboardEvent::combo(index as u8, true),
            output,
            at,
            self.last_press_time,
        );
        if !matches!(admission, Ok(Some(_))) {
            match admission {
                Err(AdmissionError::Full) => {
                    warn!("Held buffer full; dispatching combo members without triggering an unowned output");
                }
                Ok(None) => {
                    warn!("Combo output already owned; dispatching pending members without retriggering");
                }
                Ok(Some(_)) => unreachable!(),
            }
            for key in &mut self.held_buffer.keys {
                if key.state == KeyState::WaitingCombo {
                    key.state = KeyState::Deferred;
                }
            }
            self.keymap.with_combos_mut(|combos| {
                for combo in combos.iter_mut().flatten().filter(|combo| !combo.is_triggered()) {
                    combo.reset();
                }
            });
            return false;
        }
        let generation = admission.unwrap().unwrap();
        let output_index = self.held_buffer.generation_index(generation).unwrap();
        self.held_buffer.keys[output_index]
            .capture
            .as_mut()
            .unwrap()
            .deliver_before = deliver_before;
        self.keymap.with_combos_mut(|combos| {
            combos[index].as_mut().unwrap().trigger();
        });
        for key in &mut self.held_buffer.keys {
            if key.state == KeyState::WaitingCombo && chord.contains(&key.action) {
                key.state = KeyState::Deferred;
                let capture = key.capture.as_mut().unwrap();
                capture.phase = DeliveryPhase::Consumed;
            }
        }
        self.reset_shadowed_combos(&chord);
        true
    }

    pub(super) fn drain_ordered_keys(&mut self) -> impl core::future::Future<Output = ()> + '_ {
        self.run_schedule(ScheduleRequest::Drain)
    }

    pub(super) fn run_schedule(&mut self, request: ScheduleRequest) -> impl core::future::Future<Output = ()> + '_ {
        let (drain, pending, mut explicit) = match request {
            ScheduleRequest::Drain => (true, true, None),
            ScheduleRequest::ObservePending => (false, true, None),
            ScheduleRequest::Observe(generation) => (false, false, Some(generation)),
        };
        async move {
            for _ in 0..if drain { 16 } else { 1 } {
                if drain {
                    self.held_buffer.keys.retain(|key| {
                        !key.capture
                            .is_some_and(|c| c.phase == DeliveryPhase::Consumed && c.up.is_some())
                    });
                }
                while let Some((stamp, event, action)) = explicit
                    .take()
                    .map(|generation| {
                        let index = self.held_buffer.generation_index(generation).unwrap();
                        let key = &self.held_buffer.keys[index];
                        (key.capture.unwrap().down, key.event, key.action)
                    })
                    .or_else(|| if pending { self.next_observation() } else { None })
                {
                    while let Some(key) = self.held_buffer.next_timeout(|key| {
                        self.held_buffer.morse_timeout_eligible(key)
                            && key.timeout_time < stamp.at
                            && key.capture.is_some_and(|capture| capture.policy.is_none())
                    }) {
                        self.logical_time = Some(key.timeout_time);
                        self.handle_morse_timeout(key.event, key.action, key.state).await;
                        self.logical_time = None;
                    }
                    self.logical_time = Some(stamp.at);
                    self.executing_order = Some(stamp.order);
                    let (_, mut decisions) = self.make_decisions_for_keys(&action, event);
                    decisions
                        .retain(|(source, decision)| *source != event.source && *decision != HeldKeyDecision::Normal);
                    self.fire_held_keys(KeyBehaviorDecision::Ignore, decisions).await;
                    for key in &mut self.held_buffer.keys {
                        if let Some(capture) = key.capture.as_mut()
                            && capture.phase != DeliveryPhase::Queued
                        {
                            if capture.deliver_before == Some(stamp.order) {
                                capture.deliver_before = None;
                            }
                            if matches!(key.state, KeyState::Pressed(_) | KeyState::Released(_))
                                && capture.down.order < stamp.order
                            {
                                capture.observed_through = capture.observed_through.max(stamp.order);
                            }
                        }
                    }
                    self.logical_time = None;
                    self.executing_order = None;
                }
                if !drain {
                    break;
                }
                let owner = self.ordered_owner();
                if let Some(owner) = owner
                    && let Some(action) = self.ordered_resolution(owner, Instant::now())
                {
                    self.resolve_ordered_owner(*owner, action).await;
                    continue;
                }
                let pending = if let Some(owner) = owner {
                    self.held_buffer.keys.iter().filter_map(|key| {
                    let c = key.capture?;
                    if c.phase != DeliveryPhase::PressDelivered || c.up.is_none() || c.down.order >= owner.capture.unwrap().down.order { return None; }
                    let ordinary = matches!(c.binding, Some(KeyAction::Single(Action::Key(KeyCode::Hid(code)))) if !code.is_modifier())
                        || matches!(key.state, KeyState::ProcessedButReleaseNotReportedYet(Action::Key(KeyCode::Hid(code)))
                            | KeyState::FlowTapped(Action::Key(KeyCode::Hid(code))) if !code.is_modifier());
                    let general_morse = c.policy.is_none() && key.action.is_morse()
                        && matches!(key.state, KeyState::Pressed(_) | KeyState::Holding(_) | KeyState::Released(_));
                    (ordinary || general_morse).then_some(PendingEvent {
                        generation: c.down.order,
                        event: KeyboardEvent { pressed: false, source: key.event.source },
                        at: c.up?.at,
                        order: c.up?.order,
                    })
                }).min_by_key(|event| event.order)
                } else {
                    self.held_buffer.next_pending_event()
                };
                let Some(pending) = pending else { break };
                if let Some(key) = self
                    .held_buffer
                    .next_timeout(|key| self.held_buffer.morse_timeout_eligible(key) && key.timeout_time < pending.at)
                {
                    self.logical_time = Some(key.timeout_time);
                    self.handle_morse_timeout(key.event, key.action, key.state).await;
                    self.logical_time = None;
                    continue;
                }
                self.execute_ordered_event(pending).await;
                self.fire_mouse_repeat().await;
                self.fire_macro().await;
                #[cfg(feature = "_ble")]
                self.fire_user_hold().await;
                if self.keyboard_event_subscriber.available() > 0 {
                    break;
                }
            }
        }
    }

    async fn execute_ordered_event(&mut self, pending: PendingEvent) {
        let Some(index) = self.held_buffer.generation_index(pending.generation) else {
            return;
        };
        let mut key = self.held_buffer.keys[index];
        self.logical_time = Some(pending.at);
        self.executing_order = Some(pending.order);
        self.held_buffer.complete_pending_event(pending);
        let mut general = None;
        if pending.event.pressed {
            self.held_buffer.keys.remove(index);
            if let Some(previous) = self.held_buffer.find_executing_source(pending.event.source).copied()
                && previous.action.is_morse()
            {
                let (candidate, layer) = if pending.event.source.is_physical() {
                    self.keymap.peek_action_with_layer(pending.event.source)
                } else {
                    (key.action, self.keymap.pressed_layer(pending.event.source))
                };
                let candidate = self.select_forks(&candidate, pending.event, layer).0;
                match previous.state {
                    KeyState::Released(pattern) if previous.action != candidate => {
                        self.held_buffer
                            .remove_executing_if(|old| old.event.source == pending.event.source);
                        let action = Self::action_from_pattern(self.keymap, &previous.action, pattern);
                        self.process_key_action_tap(action, previous.event).await;
                    }
                    KeyState::EarlyFired(_) if previous.action != candidate => {
                        self.held_buffer
                            .remove_executing_if(|old| old.event.source == pending.event.source);
                    }
                    _ => {}
                }
            }
            let binding = if pending.event.source.is_physical() {
                self.keymap.get_action_with_layer_cache(pending.event)
            } else {
                key.action
            };
            let (binding, dropped) = self.try_start_forks(&binding, pending.event);
            self.prepared_source = Some(pending.event.source);
            if dropped {
                self.send_keyboard_report_with_resolved_modifiers(true).await;
            }
            {
                let capture = key.capture.as_mut().unwrap();
                capture.binding = Some(binding);
                capture.fork_bound = self
                    .fork_states
                    .iter()
                    .flatten()
                    .any(|fork| fork.source == pending.event.source);
                capture.phase = DeliveryPhase::PressDelivered;
            }
            key.action = binding;
            let ordered = matches!(pending.event.source, KeyboardEventSource::Key(_))
                && matches!(binding, KeyAction::TapHold(..));
            if ordered {
                let recent = [key.capture.unwrap().last_simple_before, self.last_press_time]
                    .into_iter()
                    .filter(|at| *at <= pending.at)
                    .max();
                let flow = Self::is_flow_tap_enabled(self.keymap, &binding)
                    && recent.is_some_and(|at| {
                        pending.at.saturating_duration_since(at) < self.keymap.morse_prior_idle_time()
                    });
                let quick = self
                    .held_buffer
                    .find_executing_source(pending.event.source)
                    .is_some_and(|previous| {
                        previous.action == binding
                            && matches!(previous.state, KeyState::EarlyFired(_))
                            && Self::quick_tap_window(self.keymap, &binding).is_some_and(|window| {
                                pending.at.saturating_duration_since(previous.press_time) <= window
                            })
                    });
                self.held_buffer.remove_executing_if(|old| {
                    old.event.source == pending.event.source
                        && matches!(old.state, KeyState::EarlyFired(_) | KeyState::Released(_))
                });
                key.capture.as_mut().unwrap().policy = Some(OrderedPolicy {
                    mode: Self::tap_hold_mode(self.keymap, &binding),
                    unilateral: Self::is_unilateral_tap_enabled(self.keymap, &binding),
                    quick_tap_ms: Self::quick_tap_window(self.keymap, &binding)
                        .map_or(0, |window| window.as_millis() as u16),
                });
                key.state = KeyState::OrderedPending;
                key.timeout_time = pending.at + Self::morse_timeout(self.keymap, &binding, true);
                self.held_buffer.push(key);
                if flow || quick {
                    let KeyAction::TapHold(tap, _, _) = binding else {
                        unreachable!()
                    };
                    self.resolve_ordered_owner(key, tap).await;
                }
            } else {
                general = Some(binding);
            }
        } else if let Some(policy) = key.capture.unwrap().policy {
            if let KeyState::ProcessedButReleaseNotReportedYet(action) = key.state {
                self.process_key_action_normal(action, pending.event).await;
                if self.try_finish_forks(pending.event) {
                    self.send_keyboard_report_with_resolved_modifiers(false).await;
                }
                if matches!(key.action, KeyAction::TapHold(tap, _, _) if tap == action) && policy.quick_tap_ms > 0 {
                    let key = &mut self.held_buffer.keys[index];
                    key.state = KeyState::EarlyFired(TAP);
                    key.press_time = pending.at;
                    key.timeout_time = pending.at + embassy_time::Duration::from_millis(policy.quick_tap_ms as u64);
                } else {
                    self.held_buffer.keys.remove(index);
                }
            }
        } else {
            general = Some(key.capture.unwrap().binding.unwrap());
        }
        if let Some(mut binding) = general {
            let event = pending.event;
            let (decision, decisions) = self.make_decisions_for_keys(&binding, event);
            self.held_buffer
                .keys
                .retain(|held| !matches!(held.state, KeyState::EarlyFired(_)) || held.event.source == event.source);
            let (updated, decision) = self.fire_held_keys(decision, decisions).await;
            if decision != KeyBehaviorDecision::Buffer && updated && event.source.is_physical() && event.pressed {
                binding = self.keymap.get_action_with_layer_cache(event);
            }
            if !event.pressed {
                binding = self
                    .held_buffer
                    .find_executing_source(event.source)
                    .and_then(|held| held.capture.and_then(|capture| capture.binding))
                    .unwrap_or(binding);
            }
            if decision == KeyBehaviorDecision::Buffer {
                if binding.is_morse() {
                    self.process_key_action_morse(&binding, event, pending.at).await;
                } else {
                    self.held_buffer.push(HeldKey::new(
                        event,
                        binding,
                        KeyState::Pressed(MorsePattern::default()),
                        pending.at,
                        pending.at,
                    ));
                }
            } else if decision == KeyBehaviorDecision::FlowTap
                && binding.is_morse()
                && Self::is_flow_tap_enabled(self.keymap, &binding)
            {
                let action = Self::action_from_pattern(self.keymap, &binding, TAP);
                self.process_key_action_normal(action, event).await;
                self.held_buffer
                    .remove_executing_if(|held| held.event.source == event.source);
                let now = self.action_time();
                self.held_buffer.push(HeldKey::new(
                    event,
                    binding,
                    KeyState::FlowTapped(action),
                    now,
                    now + Self::morse_timeout(self.keymap, &binding, true),
                ));
            } else {
                self.process_key_action_inner(&binding, event, pending.at).await;
            }
            if event.pressed {
                if let Some(existing) = self.held_buffer.find_executing_source_mut(event.source) {
                    existing.capture = key.capture;
                } else {
                    key.state = KeyState::ProcessedButReleaseNotReportedYet(Action::No);
                    self.held_buffer.push(key);
                }
            } else {
                if let Some(index) = self.held_buffer.generation_index(pending.generation)
                    && !matches!(
                        self.held_buffer.keys[index].state,
                        KeyState::Released(_) | KeyState::EarlyFired(_)
                    )
                {
                    self.held_buffer.keys.remove(index);
                }
            }
        }
        self.logical_time = None;
        self.executing_order = None;
        self.prepared_source = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BehaviorConfig, Hand, PositionalConfig};
    use crate::keyboard::held_buffer::HeldBuffer;
    use crate::keymap::{KeyMap, KeymapData};
    use crate::test_support::test_block_on;

    // Collect and sort the physical edges rather than selecting a policy's
    // decision directly, preserving the original chronological semantics.
    fn chronological_resolution(keyboard: &Keyboard<'_>, owner: HeldKey, now: Instant) -> Option<Action> {
        let KeyAction::TapHold(tap, hold, _) = owner.action else {
            unreachable!()
        };
        let capture = owner.capture.unwrap();
        let policy = capture.policy.unwrap();
        let mut edges = std::vec::Vec::new();
        for key in &keyboard.held_buffer.keys {
            let Some(c) = key.capture else { continue };
            if c.phase == DeliveryPhase::Consumed
                || key.state == KeyState::WaitingCombo
                || !key.event.source.is_physical()
            {
                continue;
            }
            for (stamp, pressed) in [(Some(c.down), true), (c.up, false)] {
                if let Some(stamp) = stamp
                    && stamp.order > capture.down.order
                {
                    edges.push((stamp, pressed, key.event.source, c.down.order));
                }
            }
        }
        edges.sort_by_key(|(stamp, ..)| stamp.order);
        let mut first_release = None;
        for (stamp, pressed, source, generation) in edges {
            if stamp.at >= owner.timeout_time {
                break;
            }
            if generation == capture.down.order && !pressed {
                return Some(tap);
            }
            if generation <= capture.down.order {
                continue;
            }
            if pressed && policy.mode == MorseMode::HoldOnOtherPress {
                return Some(hold);
            }
            if !pressed {
                let same_hand = match (owner.event.source, source) {
                    (KeyboardEventSource::Key(a), KeyboardEventSource::Key(b)) => keyboard
                        .keymap
                        .hand_at(a.row as usize, a.col as usize)
                        .is_same_side(keyboard.keymap.hand_at(b.row as usize, b.col as usize)),
                    _ => false,
                };
                first_release.get_or_insert(same_hand);
                if policy.mode == MorseMode::PermissiveHold {
                    return Some(if policy.unilateral && same_hand { tap } else { hold });
                }
            }
        }
        (now >= owner.timeout_time).then_some(if policy.unilateral && first_release == Some(true) {
            tap
        } else {
            hold
        })
    }

    #[test]
    fn ordered_policy_matches_chronological_physical_traces() {
        test_block_on(async {
            let behavior = Box::leak(Box::new(BehaviorConfig::default()));
            let positional = Box::leak(Box::new(PositionalConfig::new([[
                Hand::Left,
                Hand::Left,
                Hand::Right,
                Hand::Unknown,
                Hand::Bilateral,
            ]])));
            let data = Box::leak(Box::new(KeymapData::new([[[KeyAction::No; 5]]])));
            let keymap = KeyMap::new(data, behavior, positional).await;
            let mut keyboard = Keyboard::new(&keymap);
            let sources = [
                KeyboardEvent::key(0, 0, true),
                KeyboardEvent::key(0, 1, true),
                KeyboardEvent::key(0, 2, true),
                KeyboardEvent::key(0, 3, true),
                KeyboardEvent::key(0, 4, true),
                KeyboardEvent::rotary_encoder(0, crate::input_device::rotary_encoder::Direction::Clockwise, true),
                KeyboardEvent::combo(0, true),
            ];
            let tap_hold = KeyAction::TapHold(Action::LayerOn(1), Action::LayerOn(2), u8::MAX);
            let mut random = 0x7d35_9a21u32;
            for mode in [
                MorseMode::Normal,
                MorseMode::PermissiveHold,
                MorseMode::HoldOnOtherPress,
            ] {
                for unilateral in [false, true] {
                    for trace in 0..512 {
                        keyboard.held_buffer = HeldBuffer::new();
                        let at = Instant::from_millis(100);
                        keyboard.held_buffer.admit(sources[1], KeyAction::No, at, at).unwrap();
                        let id = keyboard
                            .held_buffer
                            .admit(sources[0], tap_hold, at, at)
                            .unwrap()
                            .unwrap();
                        let index = keyboard.held_buffer.generation_index(id).unwrap();
                        let owner = &mut keyboard.held_buffer.keys[index];
                        owner.state = KeyState::OrderedPending;
                        owner.timeout_time = Instant::from_millis(200);
                        owner.capture.as_mut().unwrap().phase = DeliveryPhase::PressDelivered;
                        owner.capture.as_mut().unwrap().policy = Some(OrderedPolicy {
                            mode,
                            unilateral,
                            quick_tap_ms: 0,
                        });
                        let mut pressed = [false; 7];
                        pressed[0] = true;
                        pressed[1] = true;
                        for step in 0..12 {
                            random ^= random << 13;
                            random ^= random >> 17;
                            random ^= random << 5;
                            let source = random as usize % sources.len();
                            let event = KeyboardEvent {
                                pressed: !pressed[source],
                                ..sources[source]
                            };
                            pressed[source] = event.pressed;
                            // Monotonic capture times, including ties and the exact deadline.
                            let at = Instant::from_millis([100, 150, 199, 200][step / 3]);
                            let generation = keyboard
                                .held_buffer
                                .admit(event, KeyAction::No, at, at)
                                .unwrap()
                                .unwrap();
                            if source != 0 {
                                let i = keyboard.held_buffer.generation_index(generation).unwrap();
                                match (random >> 8) % 4 {
                                    0 => keyboard.held_buffer.keys[i].state = KeyState::WaitingCombo,
                                    1 => {
                                        keyboard.held_buffer.keys[i].capture.as_mut().unwrap().phase =
                                            DeliveryPhase::Consumed
                                    }
                                    _ => (),
                                }
                            }
                            let owner = keyboard.held_buffer.keys[keyboard.held_buffer.generation_index(id).unwrap()];
                            for now in [199, 200, 201] {
                                let now = Instant::from_millis(now).max(at);
                                assert_eq!(
                                    keyboard.ordered_resolution(&owner, now),
                                    chronological_resolution(&keyboard, owner, now),
                                    "mode={mode:?} unilateral={unilateral} trace={trace} step={step} now={now:?}"
                                );
                            }
                            let len = keyboard.held_buffer.keys.len();
                            keyboard.held_buffer.keys.swap(0, len - 1);
                        }
                    }
                }
            }
        });
    }
}
