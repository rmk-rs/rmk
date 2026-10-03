//! Sticky key state machine.
//!
//! A sticky key postpones the release of the action it wraps until the next
//! input, so tapping `SK(LShift)` shifts the key that follows. It never
//! inspects the action: press and release both go through the ordinary
//! dispatch path, and the only thing this module decides is *when* the release
//! happens, and when the host is told about it.
//!
//! `OSM(mod)` is `SK(Modifier(mod))` and `OSL(n)` is `SK(MO(n))`; there is no
//! separate one-shot code path.

use embassy_time::{Duration, Instant};
use rmk_types::action::{Action, KeyAction};
use rmk_types::keycode::HidKeyCode;

use crate::event::{KeyboardEvent, KeyboardEventPos};
use crate::keyboard::Keyboard;

/// Where a sticky key is in its life cycle, carrying the instant that phase
/// cares about. The press is always dispatched right away, so there is no "not
/// yet pressed" state: `activate_on_press` only decides whether the host is
/// told about it now or on the next report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum StickyPhase {
    /// The key is still down, since this instant.
    Held(Instant),
    /// The key came up, waiting for the next input until this deadline.
    /// `None` means the profile set no timeout.
    Armed(Option<Instant>),
    /// An input already claimed it; the release waits for the key to come up.
    Used,
}

impl StickyPhase {
    /// Whether the key that triggered this sticky is still held down.
    fn key_down(self) -> bool {
        matches!(self, StickyPhase::Held(_) | StickyPhase::Used)
    }
}

pub(crate) struct Sticky {
    /// Keys with the same action and profile share one sticky toggle.
    pub(crate) action: Action,
    /// Reuse the original input position when dispatching the postponed release.
    pub(crate) pos: KeyboardEventPos,
    pub(crate) profile: u8,
    pub(crate) phase: StickyPhase,
}

/// What the keyboard is currently sending to the host. Comparing this across a
/// key's dispatch answers "did this key actually produce input", which is what
/// a sticky key waits for. Modifiers are deliberately absent: they ride along
/// with content rather than being content, and counting them would let one
/// sticky modifier claim another.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutputSnapshot {
    keycodes: [HidKeyCode; 6],
    media: u16,
    system: u8,
    mouse_buttons: u8,
}

/// When an armed record expires. A profile timeout of `0` means never.
fn sticky_deadline(timeout_ms: u16) -> Option<Instant> {
    (timeout_ms != 0).then(|| Instant::now() + Duration::from_millis(timeout_ms as u64))
}

impl<'a> Keyboard<'a> {
    pub(crate) fn output_snapshot(&self) -> OutputSnapshot {
        OutputSnapshot {
            keycodes: self.held_keycodes(),
            media: self.media_report.usage_id,
            system: self.system_control_report.usage_id,
            mouse_buttons: self.mouse.report.buttons,
        }
    }

    /// Press or release of a sticky key itself.
    pub(crate) async fn process_key_action_sticky(
        &mut self,
        action: Action,
        profile_idx: u8,
        event: KeyboardEvent,
        event_time: Instant,
    ) {
        let existing = self
            .sticky
            .iter()
            .position(|s| s.action == action && s.profile == profile_idx);

        if event.pressed {
            // Pressing the same sticky again cancels it. This is the only
            // explicit undo a sticky key has.
            if let Some(i) = existing {
                self.release_sticky_reported(i).await;
                return;
            }
            if self.sticky.is_full() {
                // Dropping the press outright is safer than dispatching one the
                // release path can no longer recognise, which would strand the
                // action on the host.
                warn!("Sticky key table full, ignoring {:?}", action);
                return;
            }

            // The press is dispatched now either way: a layer has to be lit
            // before the next key is looked up, and a macro has to run exactly
            // once. `activate_on_press` only decides whether the host hears
            // about it now or on whatever report comes next.
            self.process_action(action, event, self.keymap.sticky_flags(profile_idx).activate_on_press())
                .await;

            if let Some(key) = self.registered.iter_mut().find(|key| key.pos == event.pos) {
                key.hold_mods = true;
                self.fork_keep_mask |= key.mods;
                self.refresh_held_modifiers();
            }

            let _ = self.sticky.push(Sticky {
                action,
                pos: event.pos,
                profile: profile_idx,
                phase: StickyPhase::Held(event_time),
            });
        } else {
            let Some(i) = existing else {
                // Either the re-press already cancelled this record, or the
                // table was full and the press never happened. Both mean there
                // is nothing left to release.
                return;
            };

            // Held past the "this is a hold" threshold, or already claimed by an
            // input: either way the user is done with it. The threshold is
            // morse's, rather than a second time knob on the sticky profile.
            let done = match self.sticky[i].phase {
                StickyPhase::Held(press_time) => {
                    event_time.saturating_duration_since(press_time)
                        >= Self::morse_timeout(self.keymap, &KeyAction::No, true)
                }
                _ => true,
            };
            if done {
                self.release_sticky_reported(i).await;
                return;
            }

            self.sticky[i].phase =
                StickyPhase::Armed(sticky_deadline(self.keymap.sticky_timeout(self.sticky[i].profile)));
        }
    }

    /// Release and let the host see it right away. Every path except the
    /// next-input claim uses this: nothing else has a report coming that the
    /// effect's disappearance could ride out on.
    async fn release_sticky_reported(&mut self, idx: usize) {
        self.release_sticky(idx).await;
        // Releasing a sticky layer, or one the host was never told about,
        // changes nothing it can see, and the report is skipped.
        self.send_keyboard_report_if_changed().await;
    }

    /// Release one record: dispatch the wrapped action's release under the
    /// position it was pressed with, then drop it.
    async fn release_sticky(&mut self, idx: usize) {
        let s = self.sticky.swap_remove(idx);
        let release = KeyboardEvent {
            pos: s.pos,
            pressed: false,
        };
        self.apply_action(s.action, release).await;
    }

    /// Consume a pending sticky when a press changes the keyboard, media, system or mouse output.
    pub(crate) async fn claim_sticky(&mut self, before: OutputSnapshot, pos: KeyboardEventPos) {
        let after = self.output_snapshot();
        if after == before {
            return;
        }
        let wrote_keyboard = after.keycodes != before.keycodes;
        let added = wrote_keyboard
            .then(|| {
                self.registered
                    .iter()
                    .rfind(|key| key.pos == pos && key.keycode != HidKeyCode::No)
                    .map(|key| key.keycode)
            })
            .flatten();
        // Whether a record that asked for the catch-up report changed something
        // the host can see. Releasing a sticky layer changes nothing, and a
        // record that didn't ask shouldn't trigger one either.
        let mut report_now = false;
        let mut i = 0;
        while i < self.sticky.len() {
            let profile = self.keymap.sticky_profile(self.sticky[i].profile);

            // The ignore list refills the timeout instead of claiming, which is
            // what lets Alt+Tab cycle.
            if let Some(k) = added
                && profile.ignore.contains(&k)
            {
                if let StickyPhase::Armed(deadline) = &mut self.sticky[i].phase {
                    *deadline = sticky_deadline(profile.timeout_ms);
                }
                i += 1;
                continue;
            }

            if self.sticky[i].phase.key_down() {
                // The key is still down, so it keeps acting like a normal key
                // until the finger comes up.
                self.sticky[i].phase = StickyPhase::Used;
                i += 1;
                continue;
            }

            let quick = profile.flags.release_on_next_press();
            let one_before = (self.held_modifiers(), self.held_keycodes());
            self.release_sticky(i).await;
            report_now |= (quick || !wrote_keyboard) && (self.held_modifiers(), self.held_keycodes()) != one_before;
            // `release_sticky` swap-removed, so don't advance.
        }

        // One catch-up report for the whole batch: sending one per record would
        // let the host watch the modifiers disappear one at a time.
        if report_now {
            self.send_keyboard_report_if_changed().await;
        }
    }

    /// The earliest sticky expiry, for `run()`'s deadline race.
    pub(crate) fn sticky_next_deadline(&self) -> Option<Instant> {
        self.sticky
            .iter()
            .filter_map(|s| match s.phase {
                StickyPhase::Armed(deadline) => deadline,
                _ => None,
            })
            .min()
    }

    /// Release every record whose timeout has passed, under one report: one
    /// per record would let the host watch the modifiers disappear in turn.
    pub(crate) async fn fire_sticky_timeout(&mut self) {
        let now = Instant::now();
        let mut released = false;
        // Descending, so the swap-remove inside only ever moves a record that
        // has already been looked at.
        for i in (0..self.sticky.len()).rev() {
            if matches!(self.sticky[i].phase, StickyPhase::Armed(Some(d)) if d <= now) {
                self.release_sticky(i).await;
                released = true;
            }
        }
        if released {
            self.send_keyboard_report_if_changed().await;
        }
    }

    /// Release records configured to end on a layer transition. Only records
    /// whose key is already up are affected: otherwise `SK(MO(1))` would be
    /// killed by the very layer it just turned on.
    pub(crate) async fn sticky_on_layer_change(&mut self, entered: bool, exited: bool) {
        let mut released = false;
        for i in (0..self.sticky.len()).rev() {
            let flags = self.keymap.sticky_flags(self.sticky[i].profile);
            let wanted = (entered && flags.release_on_layer_enter()) || (exited && flags.release_on_layer_exit());
            if wanted && !self.sticky[i].phase.key_down() {
                self.release_sticky(i).await;
                released = true;
            }
        }
        if released {
            self.send_keyboard_report_if_changed().await;
        }
    }
}
