//! Sticky key lifetimes and their keyboard actions.

use embassy_time::{Duration, Instant};
use rmk_types::action::Action;
use rmk_types::keycode::{HidKeyCode, KeyCode};
use rmk_types::modifier::ModifierCombination;

use crate::event::{ActionEvent, KeyboardEvent, KeyboardEventPos};
use crate::keyboard::{Keyboard, key_identity};

/// Lifecycle of a sticky key.
#[derive(Clone, Copy)]
enum Phase {
    /// The sticky key has been pressed & held.
    Held {
        source: KeyboardEventPos,
        since: Instant, // Press time, used to calculate `hold_timeout`.
        consumed: bool, // A non-ignored input consumed the sticky while held.
    },
    /// The sticky key has been released. It remains activated and waits for releasing.
    Waiting {
        deadline: Option<Instant>, // None disables timeout.
    },
    /// Another key is pressed and it consumes the sticky key.
    /// The sticky key remains activated and will be released until the consuming input is released.
    Consuming {
        // Consumer key's position.
        consumer: KeyboardEventPos,
        // Consumer key's keycode.
        consumer_key: (KeyCode, ModifierCombination),
    },
}

/// An active sticky key.
struct ActiveStickyKey {
    action: Action,
    // Preserve the press-time target for Again, Repeat and GraveEscape.
    resolved_action: Action,
    profile_id: u8,
    phase: Phase,
}

/// The list of active sticky keys.
pub(super) struct StickyKeyState {
    stickies: [Option<ActiveStickyKey>; crate::STICKY_MAX_ACTIVE],
}

impl StickyKeyState {
    pub(super) fn new() -> Self {
        Self {
            stickies: [const { None }; crate::STICKY_MAX_ACTIVE],
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.stickies
            .iter()
            .flatten()
            .filter_map(|sticky| match sticky.phase {
                Phase::Waiting { deadline } => deadline,
                _ => None,
            })
            .min()
    }

    fn take_release(&mut self, slot: usize) -> Option<ActionEvent> {
        self.stickies[slot].take().map(|sticky| ActionEvent {
            action: sticky.resolved_action,
            keyboard_event: KeyboardEvent {
                pos: KeyboardEventPos::Sticky(slot as u8),
                pressed: false,
            },
        })
    }
}

impl Keyboard<'_> {
    pub(super) async fn process_sticky_key(&mut self, action: Action, profile: u8, event: KeyboardEvent, at: Instant) {
        //
        if self.drain_sticky_changes().await {
            self.send_keyboard_report_if_changed(self.resolve_modifiers(false))
                .await;
        }
        let slot = if event.pressed {
            // A second press cancels a waiting/consumed instance of the same binding.
            if let Some(slot) = self.sticky.stickies.iter().position(|entry| {
                matches!(entry, Some(sticky)
                    if sticky.action == action && sticky.profile_id == profile
                        && matches!(sticky.phase, Phase::Waiting { .. } | Phase::Consuming { .. }))
            }) {
                slot
            } else {
                let Some(slot) = self.sticky.stickies.iter().position(Option::is_none) else {
                    warn!("Sticky key table full, ignoring {:?}", action);
                    return;
                };
                let resolved_action = self.resolve_action(action);
                self.sticky.stickies[slot] = Some(ActiveStickyKey {
                    action,
                    resolved_action,
                    profile_id: profile,
                    phase: Phase::Held {
                        source: event.pos,
                        since: at,
                        consumed: false,
                    },
                });
                self.process_action(
                    resolved_action,
                    KeyboardEvent {
                        pos: KeyboardEventPos::Sticky(slot as u8),
                        pressed: true,
                    },
                )
                .await;
                return;
            }
        } else {
            let Some((slot, sticky, since, consumed)) =
                self.sticky.stickies.iter_mut().enumerate().find_map(|(slot, entry)| {
                    let sticky = entry.as_mut()?;
                    match sticky.phase {
                        Phase::Held {
                            source,
                            since,
                            consumed,
                        } if source == event.pos && sticky.action == action => Some((slot, sticky, since, consumed)),
                        _ => None,
                    }
                })
            else {
                return;
            };
            let profile = self.keymap.sticky_profile_or_default(sticky.profile_id);
            let held_timeout = profile.hold_timeout_ms != 0
                && at.saturating_duration_since(since) >= Duration::from_millis(profile.hold_timeout_ms as u64);
            if !consumed && !held_timeout {
                sticky.phase = Phase::Waiting {
                    deadline: (profile.wait_timeout_ms != 0)
                        .then(|| Instant::now() + Duration::from_millis(profile.wait_timeout_ms as u64)),
                };
                return;
            }
            slot
        };
        if let Some(release) = self.sticky.take_release(slot) {
            let mut keyboard_report_pending = self.execute_action(release.action, release.keyboard_event).await;
            keyboard_report_pending |= self.drain_sticky_changes().await;
            if keyboard_report_pending {
                self.send_keyboard_report_if_changed(self.resolve_modifiers(false))
                    .await;
            }
        }
    }

    /// Finish before-press releases and their cascades before the input executes.
    pub(super) async fn prepare_sticky_input(&mut self, input: ActionEvent) {
        if !input.keyboard_event.pressed {
            return;
        }
        let Some(key) = action_key_and_modifiers(input.action) else {
            return;
        };
        let mut keyboard_report_pending = false;
        loop {
            let slot = self.sticky.stickies.iter().position(|entry| {
                let Some(sticky) = entry else { return false };
                let profile = self.keymap.sticky_profile_or_default(sticky.profile_id);
                matches!(sticky.phase, Phase::Waiting { .. })
                    && profile.release_on.before_next_press()
                    && is_consuming_input(key, &profile.ignore)
            });
            let Some(release) = slot.and_then(|slot| self.sticky.take_release(slot)) else {
                break;
            };
            keyboard_report_pending |= self.execute_action(release.action, release.keyboard_event).await;
            keyboard_report_pending |= self.drain_sticky_changes().await;
        }
        if keyboard_report_pending {
            self.send_keyboard_report_if_changed(self.resolve_modifiers(false))
                .await;
        }
    }

    /// Observe the executed input once; text macros supply their own report modifiers.
    pub(super) async fn finish_sticky_input(&mut self, input: ActionEvent, modifiers: Option<ModifierCombination>) {
        let input_key = action_key_and_modifiers(input.action);
        if input.keyboard_event.pressed
            && let Some(key) = input_key
            && is_consuming_input(key, &[])
        {
            for (slot, entry) in self.sticky.stickies.iter_mut().enumerate() {
                let Some(sticky) = entry else { continue };
                if input.keyboard_event.pos == KeyboardEventPos::Sticky(slot as u8) {
                    continue;
                }
                let profile = self.keymap.sticky_profile_or_default(sticky.profile_id);
                let consumes = is_consuming_input(key, &profile.ignore);
                match &mut sticky.phase {
                    Phase::Held { consumed, .. } if consumes => *consumed = true,
                    Phase::Waiting { deadline } if !consumes => {
                        *deadline = (profile.wait_timeout_ms != 0)
                            .then(|| Instant::now() + Duration::from_millis(profile.wait_timeout_ms as u64));
                    }
                    Phase::Waiting { .. }
                        if profile.release_on.after_next_release() && !profile.release_on.after_next_press() =>
                    {
                        sticky.phase = Phase::Consuming {
                            consumer: input.keyboard_event.pos,
                            consumer_key: key_identity(key),
                        };
                    }
                    _ => {}
                }
            }
        }

        let mut keyboard_report_pending = false;
        loop {
            let slot = self.sticky.stickies.iter().position(|entry| {
                let Some(sticky) = entry else { return false };
                match sticky.phase {
                    Phase::Waiting { .. } => {
                        let profile = self.keymap.sticky_profile_or_default(sticky.profile_id);
                        input.keyboard_event.pressed
                            && profile.release_on.after_next_press()
                            && input_key.is_some_and(|key| is_consuming_input(key, &profile.ignore))
                    }
                    Phase::Consuming { consumer, consumer_key } => {
                        !input.keyboard_event.pressed
                            && input.keyboard_event.pos == consumer
                            && (consumer != KeyboardEventPos::Macro
                                || input_key.map(key_identity) == Some(consumer_key))
                    }
                    Phase::Held { .. } => false,
                }
            });
            let Some(release) = slot.and_then(|slot| self.sticky.take_release(slot)) else {
                break;
            };
            keyboard_report_pending |= self.execute_action(release.action, release.keyboard_event).await;
            keyboard_report_pending |= self.drain_sticky_changes().await;
        }
        keyboard_report_pending |= self.drain_sticky_changes().await;
        if keyboard_report_pending {
            self.send_keyboard_report_if_changed(modifiers.unwrap_or_else(|| self.resolve_modifiers(false)))
                .await;
        }
    }

    pub(super) async fn fire_sticky_timeouts(&mut self) {
        if !self.sticky.next_deadline().is_some_and(|at| at <= Instant::now()) {
            return;
        }

        if self.drain_sticky_changes().await {
            self.send_keyboard_report_if_changed(self.resolve_modifiers(false))
                .await;
        }
        let mut keyboard_report_pending = false;
        loop {
            let now = Instant::now();
            let release = self
                .sticky
                .stickies
                .iter()
                .position(|entry| {
                    matches!(entry, Some(sticky)
                    if matches!(sticky.phase, Phase::Waiting { deadline: Some(at) } if at <= now))
                })
                .and_then(|slot| self.sticky.take_release(slot));
            let Some(release) = release else { break };
            keyboard_report_pending |= self.execute_action(release.action, release.keyboard_event).await;
            keyboard_report_pending |= self.drain_sticky_changes().await;
        }
        if keyboard_report_pending {
            self.send_keyboard_report_if_changed(self.resolve_modifiers(false))
                .await;
        }
    }

    /// Drain layer/consumer cascades without treating them as a new input or timeout.
    pub(super) async fn drain_sticky_changes(&mut self) -> bool {
        let mut changes = (false, false);
        let mut keyboard_report_pending = false;
        loop {
            let pending = self.keymap.take_layer_changes();
            changes.0 |= pending.0;
            changes.1 |= pending.1;
            let slot = self.sticky.stickies.iter().position(|entry| {
                let Some(sticky) = entry else { return false };
                if matches!(sticky.phase, Phase::Held { .. }) {
                    return false;
                }
                let profile = self.keymap.sticky_profile_or_default(sticky.profile_id);
                let consumer_gone = matches!(sticky.phase,
                        Phase::Consuming { consumer: KeyboardEventPos::Sticky(slot), .. }
                            if self.sticky.stickies.get(slot as usize).is_none_or(Option::is_none));
                consumer_gone
                    || (changes.0 && profile.release_on.layer_activate())
                    || (changes.1 && profile.release_on.layer_deactivate())
            });
            let Some(release) = slot.and_then(|slot| self.sticky.take_release(slot)) else {
                break;
            };
            keyboard_report_pending |= self.execute_action(release.action, release.keyboard_event).await;
        }
        keyboard_report_pending
    }
}

/// Own key and modifiers, with HID aliases and modifier keys normalized.
fn action_key_and_modifiers(action: Action) -> Option<(KeyCode, ModifierCombination)> {
    let (key, modifiers) = match action {
        Action::Key(key) => (key, ModifierCombination::new()),
        Action::KeyWithModifier(key, mods) => (KeyCode::Hid(key), mods),
        Action::Modifier(mods) | Action::LayerOnWithModifier(_, mods) => (KeyCode::Hid(HidKeyCode::No), mods),
        _ => return None,
    };
    Some(match key {
        KeyCode::Hid(key) if key.is_modifier() => (KeyCode::Hid(HidKeyCode::No), modifiers | key.to_hid_modifiers()),
        KeyCode::Hid(key) => (
            key.process_as_consumer()
                .map(KeyCode::Consumer)
                .or_else(|| key.process_as_system_control().map(KeyCode::SystemControl))
                .unwrap_or(KeyCode::Hid(key)),
            modifiers,
        ),
        _ => (key, modifiers),
    })
}

/// Compare the action's own outputs, never modifiers held by unrelated keys.
fn is_consuming_input((key, mut modifiers): (KeyCode, ModifierCombination), ignore: &[KeyCode]) -> bool {
    let mut consumes = match key {
        KeyCode::Hid(HidKeyCode::No) => false,
        KeyCode::Hid(key) if key.is_mouse_key() => (HidKeyCode::MouseBtn1..=HidKeyCode::MouseBtn8).contains(&key),
        _ => true,
    };
    for (ignored_key, ignored_modifiers) in ignore
        .iter()
        .filter_map(|key| action_key_and_modifiers(Action::Key(*key)))
    {
        consumes &= key != ignored_key;
        modifiers &= !ignored_modifiers;
    }
    consumes || modifiers.into_bits() != 0
}
