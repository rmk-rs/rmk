//! Sticky key lifetimes and their keyboard actions.

use embassy_time::{Duration, Instant};
use rmk_types::action::Action;
use rmk_types::keycode::{HidKeyCode, KeyCode};
use rmk_types::modifier::ModifierCombination;
use rmk_types::sticky::StickyProfile;

use crate::event::{ActionEvent, KeyboardEvent, KeyboardEventPos};
use crate::keyboard::{Keyboard, key_identity};

#[derive(Clone, Copy)]
enum Phase {
    Held {
        source: KeyboardEventPos,
        since: Instant,
        used: bool,
    },
    Waiting {
        deadline: Option<Instant>,
    },
    Consuming {
        consumer: KeyboardEventPos,
        key: (KeyCode, ModifierCombination),
    },
}

#[derive(Clone, Copy)]
struct StickyEntry {
    // A dynamic trigger such as Again must release its original resolved target.
    trigger: Action,
    target: Action,
    profile: u8,
    phase: Phase,
}

/// Slots identify held outputs, independently of the physical source position.
pub(super) struct StickyKeyState {
    entries: [Option<StickyEntry>; crate::STICKY_MAX_ACTIVE],
}

impl StickyKeyState {
    pub(super) fn new() -> Self {
        Self {
            entries: [None; crate::STICKY_MAX_ACTIVE],
        }
    }

    pub(super) fn next_deadline(&self) -> Option<Instant> {
        self.entries
            .iter()
            .flatten()
            .filter_map(|s| match s.phase {
                Phase::Waiting { deadline } => deadline,
                _ => None,
            })
            .min()
    }

    fn take_release(&mut self, slot: usize) -> Option<ActionEvent> {
        self.entries[slot].take().map(|s| ActionEvent {
            action: s.target,
            keyboard_event: KeyboardEvent {
                pos: KeyboardEventPos::Sticky(slot as u8),
                pressed: false,
            },
        })
    }
}

impl Keyboard<'_> {
    pub(super) async fn process_sticky_key(&mut self, action: Action, profile: u8, event: KeyboardEvent, at: Instant) {
        // Clear layer changes and dependents before reusing a slot.
        self.update_stickies(None, None).await;
        let slot = if event.pressed {
            if let Some(slot) = self.sticky.entries.iter().position(|entry| {
                entry.is_some_and(|sticky| {
                    sticky.trigger == action && sticky.profile == profile && !matches!(sticky.phase, Phase::Held { .. })
                })
            }) {
                slot
            } else {
                let Some(slot) = self.sticky.entries.iter().position(Option::is_none) else {
                    warn!("Sticky key table full, ignoring {:?}", action);
                    return;
                };
                let target = self.resolve_action(action);
                self.sticky.entries[slot] = Some(StickyEntry {
                    trigger: action,
                    target,
                    profile,
                    phase: Phase::Held {
                        source: event.pos,
                        since: at,
                        used: false,
                    },
                });
                self.process_action(
                    target,
                    KeyboardEvent {
                        pos: KeyboardEventPos::Sticky(slot as u8),
                        pressed: true,
                    },
                )
                .await;
                return;
            }
        } else {
            let Some((slot, sticky, since, used)) =
                self.sticky.entries.iter_mut().enumerate().find_map(|(slot, entry)| {
                    let sticky = entry.as_mut()?;
                    match sticky.phase {
                        Phase::Held { source, since, used } if source == event.pos && sticky.trigger == action => {
                            Some((slot, sticky, since, used))
                        }
                        _ => None,
                    }
                })
            else {
                return;
            };
            let profile = self.keymap.sticky_profile(sticky.profile);
            if !used
                && (profile.hold_timeout_ms == 0
                    || at.saturating_duration_since(since) < Duration::from_millis(profile.hold_timeout_ms as u64))
            {
                sticky.phase = Phase::Waiting {
                    deadline: (profile.wait_timeout_ms != 0)
                        .then(|| Instant::now() + Duration::from_millis(profile.wait_timeout_ms as u64)),
                };
                return;
            }
            slot
        };
        if let Some(release) = self.sticky.take_release(slot) {
            self.execute_sticky_release(release).await;
        }
    }

    pub(super) async fn expire_stickies(&mut self) {
        self.update_stickies(None, None).await;
        while let Some(slot) = self.sticky.entries.iter().position(|entry| {
            entry.is_some_and(
                |sticky| matches!(sticky.phase, Phase::Waiting { deadline: Some(at) } if at <= Instant::now()),
            )
        }) {
            if let Some(release) = self.sticky.take_release(slot) {
                self.execute_sticky_release(release).await;
            }
        }
    }

    /// Apply before-next-press releases before executing the input action.
    pub(super) async fn prepare_stickies(&mut self, input: Option<ActionEvent>) {
        let input_key = input.and_then(|input| action_key(input.action));
        self.release_stickies(
            |slot, sticky, profile| {
                matches!(sticky.phase, Phase::Waiting { .. })
                    && profile.release_on.before_next_press()
                    && input.is_some_and(|input| {
                        input.keyboard_event.pressed
                            && input.keyboard_event.pos != KeyboardEventPos::Sticky(slot as u8)
                            && input_key.is_some_and(|key| is_consuming_input(key, &profile.ignore))
                    })
            },
            None,
        )
        .await;
    }

    /// Observe a completed action; literal text supplies its own report modifiers.
    /// Without an input, check only layer changes and released consumer slots.
    pub(super) async fn update_stickies(&mut self, input: Option<ActionEvent>, modifiers: Option<ModifierCombination>) {
        let input_key = input.and_then(|input| action_key(input.action));
        if let Some(input) = input
            && input.keyboard_event.pressed
            && let Some(key) = input_key
            && is_consuming_input(key, &[])
        {
            for (slot, sticky) in self.sticky.entries.iter_mut().enumerate() {
                let Some(sticky) = sticky else { continue };
                if input.keyboard_event.pos == KeyboardEventPos::Sticky(slot as u8) {
                    continue;
                }
                let profile = self.keymap.sticky_profile(sticky.profile);
                let consumes = is_consuming_input(key, &profile.ignore);
                match &mut sticky.phase {
                    Phase::Held { used, .. } if consumes => *used = true,
                    Phase::Waiting { deadline } if !consumes => {
                        *deadline = (profile.wait_timeout_ms != 0)
                            .then(|| Instant::now() + Duration::from_millis(profile.wait_timeout_ms as u64));
                    }
                    Phase::Waiting { .. }
                        if profile.release_on.after_next_release() && !profile.release_on.after_next_press() =>
                    {
                        sticky.phase = Phase::Consuming {
                            consumer: input.keyboard_event.pos,
                            key: key_identity(key),
                        };
                    }
                    _ => {}
                }
            }
        }
        self.release_stickies(
            |slot, sticky, profile| {
                input.is_some_and(|input| match sticky.phase {
                    Phase::Waiting { .. } => {
                        profile.release_on.after_next_press()
                            && input.keyboard_event.pressed
                            && input.keyboard_event.pos != KeyboardEventPos::Sticky(slot as u8)
                            && input_key.is_some_and(|key| is_consuming_input(key, &profile.ignore))
                    }
                    Phase::Consuming { consumer, key } => {
                        // Macro outputs share a position, so also match their key identity.
                        !input.keyboard_event.pressed
                            && input.keyboard_event.pos == consumer
                            && (consumer != KeyboardEventPos::Macro || input_key.map(key_identity) == Some(key))
                    }
                    Phase::Held { .. } => false,
                })
            },
            modifiers,
        )
        .await;
    }

    /// Releasing a layer or consumer can release further entries in the same batch.
    async fn release_stickies(
        &mut self,
        matches_input: impl Fn(usize, &StickyEntry, &StickyProfile) -> bool,
        modifiers: Option<ModifierCombination>,
    ) {
        let mut entered = false;
        let mut exited = false;
        let mut keyboard_report = false;
        loop {
            let changes = self.keymap.take_layer_changes();
            entered |= changes.0;
            exited |= changes.1;
            let slot = self.sticky.entries.iter().enumerate().find_map(|(slot, entry)| {
                let sticky = entry.as_ref()?;
                if matches!(sticky.phase, Phase::Held { .. }) {
                    return None;
                }
                let profile = self.keymap.sticky_profile(sticky.profile);
                let consumer_gone = matches!(sticky.phase,
                    Phase::Consuming { consumer: KeyboardEventPos::Sticky(source), .. }
                        if self.sticky.entries.get(source as usize).is_none_or(Option::is_none));
                (matches_input(slot, sticky, &profile)
                    || consumer_gone
                    || (entered && profile.release_on.layer_enter())
                    || (exited && profile.release_on.layer_exit()))
                .then_some(slot)
            });
            let Some(release) = slot.and_then(|slot| self.sticky.take_release(slot)) else {
                break;
            };
            keyboard_report |= self.execute_action(release.action, release.keyboard_event).await;
        }
        if keyboard_report {
            self.send_keyboard_report_if_changed(modifiers.unwrap_or_else(|| self.resolve_modifiers(false)))
                .await;
        }
    }

    // Release the saved target without resolving Repeat/GraveEscape again.
    async fn execute_sticky_release(&mut self, release: ActionEvent) {
        let keyboard_report = self.execute_action(release.action, release.keyboard_event).await;
        if keyboard_report {
            self.send_keyboard_report_if_changed(self.resolve_modifiers(false))
                .await;
        }
        self.update_stickies(Some(release), None).await;
    }
}

/// Own key and modifiers, with HID aliases and modifier keys normalized.
fn action_key(action: Action) -> Option<(KeyCode, ModifierCombination)> {
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
    for (ignored_key, ignored_modifiers) in ignore.iter().filter_map(|key| action_key(Action::Key(*key))) {
        consumes &= key != ignored_key;
        modifiers &= !ignored_modifiers;
    }
    consumes || modifiers.into_bits() != 0
}
