//! Sticky key state machine.
//!
//! A sticky key postpones the release of the action it wraps until the next
//! input, so tapping `SK(LShift)` shifts the key that follows. It never
//! inspects the action: press and release both go through the ordinary
//! dispatch path, and the only thing this module decides is *when* the release
//! happens, and when the host is told about it.
//!
//! Both go through [`Keyboard::apply_action`], which changes the state without
//! reporting it. The effect's disappearance rides out on the next report
//! instead, which is what `release_on_next_press = false` means. Media, system
//! and mouse reports still go out from there: nothing later would carry them.
//!
//! `OSM(mod)` is `SK(Modifier(mod))` and `OSL(n)` is `SK(MO(n))`; there is no
//! separate one-shot code path.

use embassy_time::{Duration, Instant};
use rmk_types::action::Action;
use rmk_types::keycode::HidKeyCode;
use rmk_types::modifier::ModifierCombination;

use crate::event::{KeyboardEvent, KeyboardEventPos};
use crate::keyboard::Keyboard;

/// Where a sticky key is in its life cycle. The press is always dispatched
/// right away, so there is no "not yet pressed" state: `activate_on_press`
/// only decides whether the host is told about it now or on the next report.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub(crate) enum StickyPhase {
    /// The key is still down.
    Held,
    /// The key came up, waiting for the next input.
    Armed,
    /// An input already claimed it; the release waits for the key to come up.
    Used,
}

impl StickyPhase {
    /// Whether the key that triggered this sticky is still held down.
    fn key_down(self) -> bool {
        matches!(self, StickyPhase::Held | StickyPhase::Used)
    }
}

pub(crate) struct Sticky {
    /// Identity. Combo outputs arrive under the last pressed member's position
    /// but are released under the first one let go, so the position can't
    /// identify a record; the action can.
    pub(crate) action: Action,
    /// The position the press was dispatched under. HID slots are matched by
    /// position *and* keycode, so the postponed release has to reuse it or it
    /// would clear the wrong slot.
    pub(crate) pos: KeyboardEventPos,
    pub(crate) profile: u8,
    pub(crate) phase: StickyPhase,
    pub(crate) press_time: Instant,
    /// Only meaningful once the key is up; [`Instant::MAX`] means "no timeout".
    pub(crate) deadline: Instant,
    /// The `WM` modifiers this record moved into the held-modifier counts on
    /// press, to be taken back out on release. A `WM` modifier otherwise lasts
    /// only until the next key press, which is shorter than this record lives.
    pub(crate) owned_mods: ModifierCombination,
    /// Layers this record lit. Layers stay in the real layer state, since
    /// nothing reads them except the keymap lookup.
    pub(crate) layers: u32,
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

impl OutputSnapshot {
    /// The keycode this key added to the report, if any. Used to match the
    /// profile's `ignore` list.
    fn added_keycode(&self, before: &Self) -> Option<HidKeyCode> {
        self.keycodes
            .iter()
            .find(|k| **k != HidKeyCode::No && !before.keycodes.contains(k))
            .copied()
    }
}

/// When an armed record expires. A profile timeout of `0` means never.
fn sticky_deadline(timeout_ms: u16) -> Instant {
    if timeout_ms == 0 {
        Instant::MAX
    } else {
        Instant::now() + Duration::from_millis(timeout_ms as u64)
    }
}

impl<'a> Keyboard<'a> {
    pub(crate) fn output_snapshot(&self) -> OutputSnapshot {
        OutputSnapshot {
            keycodes: self.held_keycodes,
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
            let with_before = self.with_modifiers;
            let layers_before = self.keymap.layer_bits();
            if self.keymap.sticky_profile(profile_idx).flags.activate_on_press() {
                self.process_key_action_normal(action, event).await;
            } else {
                self.apply_action(action, event).await;
            }

            // Move whatever `WM` modifiers the action set into the held counts,
            // where they last as long as this record does. A plain sticky
            // modifier is already counted there and needs nothing.
            let owned_mods = self.with_modifiers & !with_before;
            self.with_modifiers = with_before;
            if owned_mods.into_bits() != 0 {
                self.hold_modifiers(owned_mods, true);
            }

            let _ = self.sticky.push(Sticky {
                action,
                pos: event.pos,
                profile: profile_idx,
                phase: StickyPhase::Held,
                press_time: event_time,
                deadline: Instant::MAX,
                owned_mods,
                layers: self.keymap.layer_bits() & !layers_before,
            });
        } else {
            let Some(i) = existing else {
                // Either the re-press already cancelled this record, or the
                // table was full and the press never happened. Both mean there
                // is nothing left to release.
                return;
            };

            // Reuses morse's "this counts as a hold" threshold rather than adding
            // a second time knob to the sticky profile.
            let hold_threshold = self.keymap.morse_default_profile().hold_timeout_ms().unwrap_or(250);
            let held_for = event_time.saturating_duration_since(self.sticky[i].press_time);

            // Held past the "this is a hold" threshold, or already claimed by an
            // input: either way the user is done with it.
            if self.sticky[i].phase == StickyPhase::Used || held_for >= Duration::from_millis(hold_threshold as u64) {
                self.release_sticky_reported(i).await;
                return;
            }

            let deadline = sticky_deadline(self.keymap.sticky_profile(self.sticky[i].profile).timeout_ms);
            let s = &mut self.sticky[i];
            s.phase = StickyPhase::Armed;
            s.deadline = deadline;
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
    ///
    /// The dispatch clears the `with_modifiers` register and the layers whole,
    /// including what a plain key or another record is holding, so what it had
    /// no business touching is put back. The register is restored outright:
    /// this record's own `WM` modifiers left it at press time, and are given
    /// back to the held counts here. Layers were never moved, so only the ones
    /// this record alone lit actually go out.
    async fn release_sticky(&mut self, idx: usize) {
        let s = self.sticky.swap_remove(idx);
        let release = KeyboardEvent {
            pos: s.pos,
            pressed: false,
        };
        let with_before = self.with_modifiers;
        let layers_before = self.keymap.layer_bits();
        self.apply_action(s.action, release).await;

        self.with_modifiers = with_before;
        if s.owned_mods.into_bits() != 0 {
            self.hold_modifiers(s.owned_mods, false);
        }

        let still_lit = self.sticky.iter().fold(0, |mask, r| mask | r.layers);
        let mut restore = layers_before & !self.keymap.layer_bits() & !(s.layers & !still_lit);
        while restore != 0 {
            self.keymap.activate_layer(restore.trailing_zeros() as u8);
            restore &= restore - 1;
        }
    }

    /// Called after a key action has been dispatched, when it turned out to
    /// produce input. `before` is the snapshot taken right before it ran, and
    /// `origin` is the action dispatched, so a sticky key that produces input
    /// doesn't claim itself.
    pub(crate) async fn claim_sticky(&mut self, before: OutputSnapshot, origin: Action) {
        if self.sticky.is_empty() {
            return;
        }
        let after = self.output_snapshot();
        if after == before {
            // No content went to the host, so this key is not "the next input":
            // modifiers, layer keys and other sticky keys all land here.
            return;
        }
        // The silent release counts on this key's own report carrying the
        // change, which only holds when the input went into the keyboard
        // report. A mouse button or a media key has no such follow-up.
        self.claim_sticky_inner(after.added_keycode(&before), after.keycodes != before.keycodes, origin)
            .await;
    }

    /// Claim on behalf of something whose output doesn't survive a snapshot: a
    /// macro presses and releases inside one execution, so comparing before and
    /// after shows nothing even though it plainly sent input.
    pub(crate) async fn claim_sticky_unconditionally(&mut self) {
        self.claim_sticky_inner(None, true, Action::No).await;
    }

    async fn claim_sticky_inner(&mut self, keycode: Option<HidKeyCode>, wrote_keyboard: bool, origin: Action) {
        let visible_before = self.held_modifiers();

        // Whether a record that asked for the catch-up report changed something
        // the host can see. Releasing a sticky layer changes nothing, and a
        // record that didn't ask shouldn't trigger one either.
        let mut report_now = false;
        let mut i = 0;
        while i < self.sticky.len() {
            let profile = self.keymap.sticky_profile(self.sticky[i].profile);

            // The ignore list refills the timeout instead of claiming, which is
            // what lets Alt+Tab cycle.
            if let Some(k) = keycode
                && profile.ignore.contains(&k)
            {
                if !self.sticky[i].phase.key_down() {
                    self.sticky[i].deadline = sticky_deadline(profile.timeout_ms);
                }
                i += 1;
                continue;
            }

            // A sticky key that puts content on the wire claims the others but
            // never itself.
            if self.sticky[i].action == origin {
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
            let one_before = self.held_modifiers();
            self.release_sticky(i).await;
            report_now |= quick && self.held_modifiers() != one_before;
            // `release_sticky` swap-removed, so don't advance.
        }

        // One catch-up report for the whole batch: sending one per record would
        // let the host watch the modifiers disappear one at a time.
        let visible_changed = self.held_modifiers() != visible_before;
        if report_now || (visible_changed && !wrote_keyboard) {
            self.send_keyboard_report_if_changed().await;
        }
    }

    /// The earliest sticky expiry, for `run()`'s deadline race.
    pub(crate) fn sticky_next_deadline(&self) -> Option<Instant> {
        self.sticky
            .iter()
            .map(|s| s.deadline)
            .filter(|d| *d != Instant::MAX)
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
            if self.sticky[i].deadline <= now {
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
            let flags = self.keymap.sticky_profile(self.sticky[i].profile).flags;
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
