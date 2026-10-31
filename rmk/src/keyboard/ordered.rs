//! Ordered execution over the existing held-key records.

use embassy_time::Instant;
use rmk_types::action::{Action, KeyAction};
use rmk_types::keycode::KeyCode;
use rmk_types::morse::{MorseMode, TAP};

use super::held_buffer::{AdmissionError, HeldKey, KeyState, OrderedPolicy, PendingEvent};
use super::{HeldKeyDecision, KeyBehaviorDecision, Keyboard};
use crate::event::{KeyboardEvent, KeyboardEventSource};

impl Keyboard<'_> {
    fn ordered_owner(&self) -> Option<HeldKey> {
        self.held_buffer
            .keys
            .iter()
            .filter(|key| key.state == KeyState::OrderedPending)
            .min_by_key(|key| key.capture.unwrap().down.order)
            .copied()
    }

    fn ordered_resolution(&self, owner: HeldKey, now: Instant) -> Option<Action> {
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
        let mut after = down.order;
        let mut first_release = None;
        loop {
            let edge = self
                .held_buffer
                .keys
                .iter()
                .filter_map(|key| {
                    let capture = key.capture?;
                    if capture.consumed || !capture.eligible || !key.event.source.is_physical() {
                        return None;
                    }
                    [Some((capture.down, true)), capture.up.map(|at| (at, false))]
                        .into_iter()
                        .flatten()
                        .filter(|(stamp, _)| stamp.order > after)
                        .min_by_key(|(stamp, _)| stamp.order)
                        .map(|(stamp, pressed)| (stamp, pressed, key.event.source, capture.down.order))
                })
                .min_by_key(|(stamp, _, _, _)| stamp.order);
            let Some((stamp, pressed, source, generation)) = edge else {
                break;
            };
            if stamp.at >= owner.timeout_time {
                break;
            }
            after = stamp.order;
            if generation == down.order && !pressed {
                return Some(tap);
            }
            if generation <= down.order {
                continue;
            }
            if pressed && mode == MorseMode::HoldOnOtherPress {
                return Some(hold);
            }
            if !pressed {
                let same_hand = match source {
                    KeyboardEventSource::Key(pos) => {
                        hand.is_same_side(self.keymap.hand_at(pos.row as usize, pos.col as usize))
                    }
                    _ => false,
                };
                first_release.get_or_insert(same_hand);
                if mode == MorseMode::PermissiveHold {
                    return Some(if unilateral && same_hand { tap } else { hold });
                }
            }
        }
        (now >= owner.timeout_time).then_some(if unilateral && first_release == Some(true) {
            tap
        } else {
            hold
        })
    }

    pub(super) fn ordered_next_deadline(&self) -> Option<Instant> {
        if !self.ordered_enabled {
            return None;
        }
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
        self.ordered_enabled
            && self.held_buffer.keys.iter().any(|key| {
                key.state == KeyState::OrderedPending || key.capture.is_some_and(|c| c.press_pending && !c.consumed)
            })
    }

    pub(super) async fn process_ordered_input(&mut self, event: KeyboardEvent, at: Instant) {
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
        let admitted = match self.held_buffer.admit(event, action, at, self.last_press_time) {
            Err(AdmissionError::Full) => {
                if let Some(owner) = self.ordered_owner() {
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
        self.held_buffer.admission = Some(generation);
        if !self.combo_on || self.process_combo(&action, event, at).await {
            self.submit_ordered_action(action, event, at).await;
        } else if let Some(index) = self.held_buffer.generation_index(generation) {
            let key = &mut self.held_buffer.keys[index];
            if key.capture.is_some_and(|capture| capture.consumed) {
                let capture = key.capture.as_mut().unwrap();
                capture.consumed = true;
                capture.press_pending = false;
                capture.release_pending = false;
            }
        }
        self.held_buffer.admission = None;
        self.drain_ordered_keys().await;
    }

    pub(super) async fn submit_ordered_action(&mut self, action: KeyAction, event: KeyboardEvent, at: Instant) {
        let generation = self
            .held_buffer
            .admission
            .filter(|id| {
                self.held_buffer
                    .generation_index(*id)
                    .is_some_and(|index| self.held_buffer.keys[index].event.source == event.source)
            })
            .or_else(|| {
                event
                    .pressed
                    .then(|| {
                        self.held_buffer
                            .keys
                            .iter()
                            .find(|key| {
                                key.event.source == event.source
                                    && key.capture.is_some_and(|c| c.press_pending && c.up.is_none())
                            })
                            .map(|key| key.capture.unwrap().down.order)
                    })
                    .flatten()
            });
        let generation = match generation {
            Some(id) => id,
            None => match self.held_buffer.admit(event, action, at, self.last_press_time) {
                Ok(Some(id)) => id,
                _ => {
                    error!("Unable to reserve synthetic action ownership");
                    return;
                }
            },
        };
        if let Some(index) = self.held_buffer.generation_index(generation) {
            let capture = self.held_buffer.keys[index].capture.as_mut().unwrap();
            capture.eligible = true;
            self.observe_pending_legacy_inputs().await;
        }
    }

    pub(super) async fn observe_ordered_event(
        &mut self,
        action: KeyAction,
        event: KeyboardEvent,
        at: Instant,
        order: u64,
    ) {
        while let Some(key) = self.held_buffer.next_timeout(|key| {
            key.action.is_morse()
                && matches!(
                    key.state,
                    KeyState::Pressed(_) | KeyState::Released(_) | KeyState::EarlyFired(_)
                )
                && key.timeout_time < at
                && key.capture.is_some_and(|capture| {
                    capture.policy.is_none()
                        && !capture.press_pending
                        && !(capture.release_pending && capture.up.is_some_and(|up| up.at <= key.timeout_time))
                })
        }) {
            self.logical_time = Some(key.timeout_time);
            self.handle_morse_timeout(&key).await;
            self.logical_time = None;
        }
        self.logical_time = Some(at);
        self.executing_order = Some(order);
        let (_, mut decisions) = self.make_decisions_for_keys(&action, event);
        decisions.retain(|(source, decision)| *source != event.source && *decision != HeldKeyDecision::Normal);
        self.fire_held_keys(KeyBehaviorDecision::Ignore, decisions).await;
        for key in &mut self.held_buffer.keys {
            if matches!(key.state, KeyState::Pressed(_) | KeyState::Released(_))
                && let Some(capture) = key.capture.as_mut()
                && !capture.press_pending
                && capture.down.order < order
            {
                capture.observed_through = capture.observed_through.max(order);
            }
        }
        self.logical_time = None;
        self.executing_order = None;
    }

    async fn observe_pending_legacy_inputs(&mut self) {
        loop {
            let next = self
                .held_buffer
                .keys
                .iter()
                .filter_map(|owner| {
                    let c = owner.capture?;
                    if c.press_pending
                        || c.policy.is_some()
                        || !owner.action.is_morse()
                        || !matches!(owner.state, KeyState::Pressed(_) | KeyState::Released(_))
                    {
                        return None;
                    }
                    self.held_buffer
                        .keys
                        .iter()
                        .filter_map(|input| {
                            let captured = input.capture?;
                            if captured.consumed || !captured.eligible || captured.down.order <= c.down.order {
                                return None;
                            }
                            [Some((captured.down, true)), captured.up.map(|at| (at, false))]
                                .into_iter()
                                .flatten()
                                .filter(|(stamp, _)| {
                                    stamp.order > c.observed_through && c.up.is_none_or(|up| stamp.order < up.order)
                                })
                                .min_by_key(|(stamp, _)| stamp.order)
                                .map(|(stamp, pressed)| {
                                    (
                                        stamp,
                                        KeyboardEvent {
                                            pressed,
                                            source: input.event.source,
                                        },
                                        captured.admission_action,
                                    )
                                })
                        })
                        .min_by_key(|(stamp, _, _)| stamp.order)
                })
                .min_by_key(|(stamp, _, _)| stamp.order);
            let Some((stamp, event, action)) = next else { break };
            self.observe_ordered_event(action, event, stamp.at, stamp.order).await;
        }
    }

    async fn resolve_ordered_owner(&mut self, owner: HeldKey, action: Action) {
        self.logical_time = Some(owner.capture.unwrap().down.at);
        self.process_key_action_normal(action, owner.event).await;
        if let Some(index) = self.held_buffer.generation_index(owner.capture.unwrap().down.order) {
            self.held_buffer.keys[index].state = KeyState::ProcessedButReleaseNotReportedYet(action);
        }
        self.logical_time = None;
    }

    pub(super) async fn ordered_combo_timeout(&mut self, key: HeldKey) {
        let mut release = key.event;
        release.pressed = false;
        self.trigger_delayed_combo(&key.action, release).await;
        if let Some(index) = self.held_buffer.generation_index(key.capture.unwrap().down.order)
            && self.held_buffer.keys[index].state == KeyState::WaitingCombo
        {
            self.held_buffer.keys[index].state = KeyState::Deferred;
            self.held_buffer.keys[index].capture.as_mut().unwrap().eligible = true;
            let capture = self.held_buffer.keys[index].capture.unwrap();
            self.observe_ordered_event(key.action, key.event, capture.down.at, capture.down.order)
                .await;
            self.keymap.with_combos_mut(|combos| {
                for combo in combos
                    .iter_mut()
                    .flatten()
                    .filter(|combo| !combo.is_triggered() && combo.config.contains(&key.action))
                {
                    combo.reset();
                }
            });
        }
    }

    pub(super) async fn fire_ordered_combo(&mut self, index: usize, at: Instant) {
        let Some((output, chord)) = self.keymap.with_combos(|combos| {
            combos[index]
                .as_ref()
                .map(|combo| (combo.config.output, combo.config.actions.clone()))
        }) else {
            return;
        };
        if self
            .held_buffer
            .admit(
                KeyboardEvent::combo(index as u8, true),
                output,
                at,
                self.last_press_time,
            )
            .is_err()
        {
            warn!("Held buffer full; dispatching combo members without triggering an unowned output");
            for key in &mut self.held_buffer.keys {
                if key.state == KeyState::WaitingCombo {
                    key.state = KeyState::Deferred;
                    key.capture.as_mut().unwrap().eligible = true;
                }
            }
            self.keymap.with_combos_mut(|combos| {
                for combo in combos.iter_mut().flatten().filter(|combo| !combo.is_triggered()) {
                    combo.reset();
                }
            });
            return;
        }
        self.keymap.with_combos_mut(|combos| {
            combos[index].as_mut().unwrap().trigger();
        });
        for key in &mut self.held_buffer.keys {
            if key.state == KeyState::WaitingCombo && chord.contains(&key.action) {
                key.state = KeyState::Deferred;
                let capture = key.capture.as_mut().unwrap();
                capture.consumed = true;
                capture.press_pending = false;
                capture.release_pending = false;
            }
        }
        self.reset_shadowed_combos(&chord);
        self.submit_ordered_action(output, KeyboardEvent::combo(index as u8, true), at)
            .await;
    }

    pub(super) async fn drain_ordered_keys(&mut self) {
        for _ in 0..16 {
            self.held_buffer
                .keys
                .retain(|key| !key.capture.is_some_and(|c| c.consumed && c.up.is_some()));
            self.observe_pending_legacy_inputs().await;
            let owner = self.ordered_owner();
            if let Some(owner) = owner
                && let Some(action) = self.ordered_resolution(owner, Instant::now())
            {
                self.resolve_ordered_owner(owner, action).await;
                continue;
            }
            let pending = if let Some(owner) = owner {
                self.held_buffer.keys.iter().filter_map(|key| {
                    let c = key.capture?;
                    if c.press_pending || !c.release_pending || c.down.order >= owner.capture.unwrap().down.order { return None; }
                    let ordinary = matches!(c.binding, Some(KeyAction::Single(Action::Key(KeyCode::Hid(code)))) if !code.is_modifier())
                        || matches!(key.state, KeyState::ProcessedButReleaseNotReportedYet(Action::Key(KeyCode::Hid(code)))
                            | KeyState::FlowTapped(Action::Key(KeyCode::Hid(code))) if !code.is_modifier());
                    let legacy_morse = c.policy.is_none() && key.action.is_morse()
                        && matches!(key.state, KeyState::Pressed(_) | KeyState::Holding(_) | KeyState::Released(_));
                    (ordinary || legacy_morse).then_some(PendingEvent {
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
            if let Some(key) = self.held_buffer.next_timeout(|key| {
                key.action.is_morse()
                    && matches!(
                        key.state,
                        KeyState::Pressed(_) | KeyState::Released(_) | KeyState::EarlyFired(_)
                    )
                    && key.timeout_time < pending.at
            }) {
                self.logical_time = Some(key.timeout_time);
                self.handle_morse_timeout(&key).await;
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

    async fn execute_ordered_event(&mut self, pending: PendingEvent) {
        let Some(index) = self.held_buffer.generation_index(pending.generation) else {
            return;
        };
        let mut key = self.held_buffer.keys[index];
        let mut capture = key.capture.unwrap();
        self.logical_time = Some(pending.at);
        self.executing_order = Some(pending.order);
        self.held_buffer.complete_pending_event(pending);
        if pending.event.pressed {
            self.held_buffer.keys.remove(index);
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
            capture.binding = Some(binding);
            capture.fork_bound = self
                .fork_states
                .iter()
                .flatten()
                .any(|fork| fork.source == pending.event.source);
            capture.press_pending = false;
            key.action = binding;
            let ordered = matches!(pending.event.source, KeyboardEventSource::Key(_))
                && matches!(binding, KeyAction::TapHold(_, _, profile) if self.keymap.morse_profile(profile).buffered_tap_hold()
                    .or_else(|| self.keymap.morse_default_profile().buffered_tap_hold()).unwrap_or(false));
            if ordered {
                let recent = [capture.last_simple_before, self.last_press_time]
                    .into_iter()
                    .filter(|at| *at <= pending.at)
                    .max();
                let flow = Self::is_flow_tap_enabled(self.keymap, &binding)
                    && recent.is_some_and(|at| {
                        pending.at.saturating_duration_since(at) < self.keymap.morse_prior_idle_time()
                    });
                let quick = self
                    .held_buffer
                    .find_source(pending.event.source)
                    .is_some_and(|previous| {
                        previous.action == binding
                            && matches!(previous.state, KeyState::EarlyFired(_))
                            && Self::quick_tap_window(self.keymap, &binding).is_some_and(|window| {
                                pending.at.saturating_duration_since(previous.press_time) <= window
                            })
                    });
                self.held_buffer.remove_if(|old| {
                    old.event.source == pending.event.source
                        && matches!(old.state, KeyState::EarlyFired(_) | KeyState::Released(_))
                });
                capture.policy = Some(OrderedPolicy {
                    mode: Self::tap_hold_mode(self.keymap, &binding),
                    unilateral: Self::is_unilateral_tap_enabled(self.keymap, &binding),
                    quick_tap_ms: Self::quick_tap_window(self.keymap, &binding)
                        .map_or(0, |window| window.as_millis() as u16),
                });
                key.capture = Some(capture);
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
                self.process_key_action_legacy(&binding, pending.event, pending.at)
                    .await;
                if let Some(existing) = self.held_buffer.find_source_mut(pending.event.source) {
                    existing.capture = Some(capture);
                } else {
                    key.capture = Some(capture);
                    key.state = KeyState::ProcessedButReleaseNotReportedYet(Action::No);
                    self.held_buffer.push(key);
                }
            }
        } else if let Some(policy) = capture.policy {
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
            let binding = capture.binding.unwrap();
            self.process_key_action_legacy(&binding, pending.event, pending.at)
                .await;
            if let Some(index) = self.held_buffer.generation_index(pending.generation)
                && !matches!(
                    self.held_buffer.keys[index].state,
                    KeyState::Released(_) | KeyState::EarlyFired(_)
                )
            {
                self.held_buffer.keys.remove(index);
            }
        }
        self.logical_time = None;
        self.executing_order = None;
        self.prepared_source = None;
    }
}
