//! Sticky key lifetimes and their keyboard actions.

use embassy_time::{Duration, Instant};
use rmk_types::action::Action;
use rmk_types::keycode::{HidKeyCode, KeyCode};
use rmk_types::modifier::ModifierCombination;

use crate::event::{ActionEvent, KeyboardEvent, KeyboardEventSource};
use crate::keyboard::{Keyboard, same_key};

enum Phase {
    Held {
        source: KeyboardEventSource,
        since: Instant, // Hold duration matters only until a nonignored input uses it.
    },
    HeldConsumed(KeyboardEventSource),
    Waiting {
        deadline: Option<Instant>, // None disables the waiting timeout.
    },
    Consuming {
        consumer: KeyboardEventSource,
        key: (KeyCode, ModifierCombination),
    },
    Releasing,
}

/// One held output; the array slot supplies its stable output identity.
pub(super) struct StickyKeyState {
    trigger: Action,
    target: Action,
    profile: u8,
    phase: Phase,
}

impl Keyboard<'_> {
    pub(super) fn next_sticky_deadline(&self) -> Option<Instant> {
        self.stickies
            .iter()
            .filter_map(|entry| match entry.as_ref()?.phase {
                Phase::Waiting { deadline } => deadline,
                _ => None,
            })
            .min()
    }

    /// Execute a source edge, then keep its output until a later input releases it.
    pub(super) async fn process_sticky_key(&mut self, action: Action, profile: u8, event: KeyboardEvent, at: Instant) {
        self.release_stickies(None).await;
        let binding = self.stickies.iter_mut().enumerate().find_map(|(slot, entry)| {
            let sticky = entry.as_mut()?;
            (sticky.trigger == action
                && match sticky.phase {
                    Phase::Held { source, .. } | Phase::HeldConsumed(source) => {
                        !event.pressed && source == event.source
                    }
                    _ => event.pressed && sticky.profile == profile,
                })
            .then_some((slot, sticky))
        });
        if let Some((slot, sticky)) = binding {
            if let Phase::Held { since, .. } = sticky.phase {
                let profile = self.keymap.sticky_profile(sticky.profile);
                if profile.hold_timeout_ms == 0
                    || at.saturating_duration_since(since) < Duration::from_millis(profile.hold_timeout_ms as u64)
                {
                    sticky.phase = Phase::Waiting {
                        deadline: (profile.wait_timeout_ms != 0)
                            .then(|| Instant::now() + Duration::from_millis(profile.wait_timeout_ms as u64)),
                    };
                    return;
                }
            }
            let release = self.take_sticky(slot);
            self.process_action(release.action, release.keyboard_event).await;
        } else if event.pressed {
            let Some(slot) = self.stickies.iter().position(Option::is_none) else {
                warn!("Sticky key table full, ignoring {:?}", action);
                return;
            };
            let target = self.resolve_action(action);
            let output = KeyboardEvent {
                source: KeyboardEventSource::Sticky(slot as u8),
                pressed: true,
            };
            self.process_action(target, output).await;
            // The press only drains releases, so this slot is still free.
            // Arm afterward so the source cannot consume its own output.
            self.stickies[slot] = Some(StickyKeyState {
                trigger: action,
                target,
                profile,
                phase: Phase::Held {
                    source: event.source,
                    since: at,
                },
            });
        }
    }

    fn take_sticky(&mut self, slot: usize) -> ActionEvent {
        ActionEvent {
            action: self.stickies[slot].take().unwrap().target,
            keyboard_event: KeyboardEvent {
                source: KeyboardEventSource::Sticky(slot as u8),
                pressed: false,
            },
        }
    }

    pub(super) async fn expire_stickies(&mut self) {
        self.release_stickies(None).await;
        while let Some(slot) = self.stickies.iter().position(|entry| {
            entry.as_ref().is_some_and(
                |sticky| matches!(sticky.phase, Phase::Waiting { deadline: Some(at) } if at <= Instant::now()),
            )
        }) {
            let input = self.take_sticky(slot);
            self.process_action(input.action, input.keyboard_event).await;
        }
    }

    /// Mark before-next-press releases before the input executes.
    pub(super) fn before_sticky_input(&mut self, input: ActionEvent) {
        let key = action_key(input.action).filter(|_| input.keyboard_event.pressed);
        for sticky in self.stickies.iter_mut().flatten() {
            let profile = self.keymap.sticky_profile(sticky.profile);
            if matches!(sticky.phase, Phase::Waiting { .. })
                && profile.release_on.before_next_press()
                && key.is_some_and(|key| is_consuming_input(key, &profile.ignore))
            {
                sticky.phase = Phase::Releasing;
            }
        }
    }

    /// Bind consumption and releases to this completed input.
    pub(super) fn after_sticky_input(&mut self, input: ActionEvent) {
        let input_key = action_key(input.action);
        for sticky in self.stickies.iter_mut().flatten() {
            let profile = self.keymap.sticky_profile(sticky.profile);
            if input.keyboard_event.pressed {
                let Some(key) = input_key.filter(|key| is_consuming_input(*key, &[])) else {
                    continue;
                };
                let consumes = is_consuming_input(key, &profile.ignore);
                sticky.phase = match sticky.phase {
                    Phase::Held { source, .. } if consumes => Phase::HeldConsumed(source),
                    Phase::Waiting { .. } if !consumes => Phase::Waiting {
                        deadline: (profile.wait_timeout_ms != 0)
                            .then(|| Instant::now() + Duration::from_millis(profile.wait_timeout_ms as u64)),
                    },
                    Phase::Waiting { .. } if profile.release_on.after_next_press() => Phase::Releasing,
                    Phase::Waiting { .. } if profile.release_on.after_next_release() => Phase::Consuming {
                        consumer: input.keyboard_event.source,
                        key,
                    },
                    _ => continue,
                };
            } else if let Phase::Consuming { consumer, key } = sticky.phase
                && input.keyboard_event.source == consumer
                && (consumer != KeyboardEventSource::Macro
                    || input_key.is_some_and(|input_key| same_key(input_key, key)))
            {
                sticky.phase = Phase::Releasing;
            }
        }
    }

    /// Drain input releases and layer/consumer cascades in slot order.
    pub(super) async fn release_stickies(&mut self, modifiers: Option<ModifierCombination>) {
        let mut keyboard_report = false;
        let mut changes = (false, false);
        loop {
            let pending = self.keymap.take_layer_changes();
            changes.0 |= pending.0;
            changes.1 |= pending.1;
            let slot = self.stickies.iter().position(|entry| {
                let Some(sticky) = entry else { return false };
                let profile = self.keymap.sticky_profile(sticky.profile);
                matches!(sticky.phase, Phase::Releasing)
                    || (matches!(sticky.phase, Phase::Waiting { .. } | Phase::Consuming { .. })
                        && ((changes.0 && profile.release_on.layer_enter())
                            || (changes.1 && profile.release_on.layer_exit())))
            });
            let Some(slot) = slot else { break };
            let release = self.take_sticky(slot);
            keyboard_report |= self.execute_action(release).await;
            self.after_sticky_input(release);
        }
        if keyboard_report {
            let modifiers = modifiers.unwrap_or_else(|| self.resolve_modifiers(false));
            self.send_keyboard_report(modifiers, true).await;
        }
    }
}

/// Own key and modifiers, with HID aliases and modifier keys normalized.
pub(super) fn action_key(action: Action) -> Option<(KeyCode, ModifierCombination)> {
    let (key, modifiers) = match action {
        Action::Key(key) => (key, ModifierCombination::new()),
        Action::KeyWithModifier(key, mods) => (KeyCode::Hid(key), mods),
        Action::Modifier(mods) | Action::LayerOnWithModifier(_, mods) => (KeyCode::Hid(HidKeyCode::No), mods),
        _ => return None,
    };
    Some(match key {
        KeyCode::Hid(key) if key.is_modifier() => (KeyCode::Hid(HidKeyCode::No), modifiers | key.to_hid_modifiers()),
        KeyCode::Hid(hid) => (
            hid.process_as_consumer()
                .map(KeyCode::Consumer)
                .or_else(|| hid.process_as_system_control().map(KeyCode::SystemControl))
                .unwrap_or(key),
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
    for (ignored_key, ignored_modifiers) in ignore.iter().filter_map(|key| action_key(Action::Key(*key))) {
        consumes &= key != ignored_key;
        modifiers &= !ignored_modifiers;
    }
    consumes || modifiers.into_bits() != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hid_alias_has_the_same_sticky_identity_and_ignore_behavior() {
        for hid in (u8::MIN..=u8::MAX).filter_map(HidKeyCode::from_repr) {
            let native = hid
                .process_as_consumer()
                .map(KeyCode::Consumer)
                .or_else(|| hid.process_as_system_control().map(KeyCode::SystemControl));
            let Some(native) = native else { continue };
            let alias = KeyCode::Hid(hid);
            let alias_input = action_key(Action::Key(alias)).unwrap();
            let native_input = action_key(Action::Key(native)).unwrap();
            assert_eq!(alias_input, native_input, "{hid:?}");
            assert!(!is_consuming_input(alias_input, &[native]), "{hid:?}");
            assert!(!is_consuming_input(native_input, &[alias]), "{hid:?}");
            let shift = ModifierCombination::new().with_left_shift(true);
            let shifted = action_key(Action::KeyWithModifier(hid, shift)).unwrap();
            assert!(same_key(shifted, native_input), "{hid:?}");
            assert!(is_consuming_input(shifted, &[native]), "{hid:?}");
            assert!(!is_consuming_input(
                shifted,
                &[native, KeyCode::Hid(HidKeyCode::LShift)]
            ));
        }
    }
}
