//! Sticky Key: defer an ordinary [`Action`]'s release to a later trigger.
//!
//! A latch stores the action it must release and the event identity to replay it
//! with. Nothing here inspects what the action does, so any action can be sticky.
//! One-shot modifiers and layers are just latches over `Action::Modifier` and
//! `Action::LayerOn`.
//!
//! The action is applied when the Sticky key is pressed, because a latched layer
//! has to be active before the next key is resolved against the keymap. What
//! `activate_on_keypress` defers is the *report*, not the effect: without it the
//! host first sees the effect on the report of the key that consumes it.

use embassy_time::{Duration, Instant};
use rmk_types::action::Action;
use rmk_types::keycode::KeyCode;

use crate::event::KeyboardEvent;
use crate::keyboard::Keyboard;
use crate::keymap::StickyKeyPolicy;

/// Latches that may be active at once. Each is independent: they neither
/// combine nor exclude one another.
const MAX_ACTIVE: usize = 4;

#[derive(Clone, Copy, Debug)]
struct Latch {
    /// The action to release later, and the event identity to replay it with.
    action: Action,
    event: KeyboardEvent,
    /// A combo output is pressed by the member key that completes the chord and
    /// released by the last one up, so it is matched by action, not position.
    from_combo: bool,
    policy: StickyKeyPolicy,
    /// Whether a report carrying this effect has reached the host. Undoing a
    /// latch the host never saw must stay silent, and so must its siblings.
    visible: bool,
    /// Foreign keys this latch has served, bounded by `policy.max_repeat`.
    uses: u16,
    state: LatchState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LatchState {
    /// The Sticky key itself is still down. `peers` counts the other keys that
    /// were already down when it was pressed, and `used` records that a foreign
    /// key consumed the effect. Either makes it an ordinary held key.
    Down { since: Instant, peers: u16, used: bool },
    /// The Sticky key is up and the effect waits for a consumer.
    Armed(Option<Instant>),
    /// A foreign key claimed the latch; the next foreign key-up resolves it.
    Bound,
}

pub(crate) struct StickyKeys {
    latches: [Option<Latch>; MAX_ACTIVE],
}

impl Default for StickyKeys {
    fn default() -> Self {
        Self {
            latches: [None; MAX_ACTIVE],
        }
    }
}

impl StickyKeys {
    /// The earliest armed timeout. Only `Armed` latches schedule a wakeup: a
    /// latch bound to a held key ends with that key, not with the clock.
    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.latches
            .iter()
            .flatten()
            .filter_map(|latch| match latch.state {
                LatchState::Armed(deadline) => deadline,
                _ => None,
            })
            .min()
    }

    /// Every live latch contributed to the report that just went out.
    pub(crate) fn mark_reported(&mut self) {
        for latch in self.latches.iter_mut().flatten() {
            latch.visible = true;
        }
    }

    fn find(&self, action: Action, event: KeyboardEvent) -> Option<usize> {
        let at_pos = self
            .latches
            .iter()
            .position(|latch| latch.is_some_and(|latch| latch.event.pos == event.pos));
        if at_pos.is_some() || event.pressed {
            // A press only ever re-presses the key that made the latch. Matching
            // by action here would let a second Sticky key of the same action
            // cancel the first.
            return at_pos;
        }
        self.latches
            .iter()
            .position(|latch| latch.is_some_and(|latch| latch.from_combo && latch.action == action))
    }
}

fn timeout_deadline(policy: StickyKeyPolicy) -> Option<Instant> {
    (policy.timeout != Duration::MAX).then(|| Instant::now() + policy.timeout)
}

/// Whether `action` is one of the keys this profile ends its latch on. An empty
/// list means every key ends it. Matching is by the keycode an action produces,
/// so a `keep_keys` entry of `Tab` also covers `WM(Tab, LShift)`.
fn ends_latch(policy: StickyKeyPolicy, action: Action) -> bool {
    if policy.keys.is_empty() {
        return true;
    }
    let key = match action {
        Action::Key(key) => Some(key),
        Action::KeyWithModifier(key, _) => Some(KeyCode::Hid(key)),
        _ => None,
    };
    key.is_some_and(|key| policy.keys.contains(&key)) != policy.keys_keep
}

impl Keyboard<'_> {
    /// The Sticky key's own press and release.
    pub(crate) async fn process_sticky_key(
        &mut self,
        action: Action,
        profile: u8,
        event: KeyboardEvent,
        from_combo: bool,
        pressed_at: Instant,
    ) {
        let existing = self.sticky.find(action, event);

        if !event.pressed {
            let Some(index) = existing else { return };
            let Some(latch) = self.sticky.latches[index] else {
                return;
            };
            let LatchState::Down { since, peers, used } = latch.state else {
                return;
            };
            let held = latch
                .policy
                .release_after_hold
                .duration()
                .is_some_and(|threshold| since.elapsed() >= threshold);
            // The count no longer includes this key, so exceeding `peers` means a
            // key pressed after this one is still down. That is a chord, not a
            // latch, and it counts even while that key is still buffered.
            let chorded = self.physical_keys_down > peers;
            if used || held || chorded {
                if self.release_latch(index).await {
                    self.settle_report().await;
                }
            } else if let Some(latch) = self.sticky.latches[index].as_mut() {
                latch.state = LatchState::Armed(timeout_deadline(latch.policy));
            }
            return;
        }

        let policy = self.keymap.sticky_key_profile(profile);
        // A re-press ends the running latch; `double_tap` stops there instead of
        // starting a new one.
        if let Some(index) = existing {
            let latched = !matches!(
                self.sticky.latches[index].map(|latch| latch.state),
                Some(LatchState::Down { .. })
            );
            if self.release_latch(index).await {
                self.settle_report().await;
            }
            if latched && policy.release_mode.double_tap() {
                return;
            }
        }

        let Some(slot) = self.sticky.latches.iter().position(Option::is_none) else {
            warn!("No free Sticky Key slot, ignoring {:?}", action);
            return;
        };
        self.sticky.latches[slot] = Some(Latch {
            action,
            event,
            from_combo,
            policy,
            visible: false,
            uses: 0,
            state: LatchState::Down {
                // The physical press, which a combo or morse decision may have
                // buffered well before this dispatch.
                since: pressed_at,
                // A combo output owns no physical key, so every key still down
                // when it ends is a peer.
                peers: if from_combo {
                    0
                } else {
                    self.physical_keys_down.saturating_sub(1)
                },
                used: false,
            },
        });

        // A latched layer must be active before the next key is resolved, so the
        // effect always applies now; only its report waits for a consumer.
        self.coalesce_report = !policy.activate_on_keypress;
        self.dispatch_action(action, event).await;
        self.coalesce_report = false;
    }

    /// Before a foreign action is dispatched: claim latches on a press, and end
    /// the ones whose consuming key is going up, so both halves land in the
    /// report that action is about to send. Returns whether any latch ended.
    pub(crate) async fn sticky_before(&mut self, action: Action, event: KeyboardEvent) -> bool {
        // A modifier never consumes a latch, so Sticky modifiers stack with each
        // other and with physically held modifier keys.
        let is_modifier = match action {
            Action::Modifier(_) => true,
            Action::Key(KeyCode::Hid(key)) => key.is_modifier(),
            _ => false,
        };
        if is_modifier {
            return false;
        }

        let mut released = false;
        for index in 0..MAX_ACTIVE {
            let Some(latch) = self.sticky.latches[index] else {
                continue;
            };
            if !ends_latch(latch.policy, action) {
                // A key this profile keeps: it uses the effect and refreshes the
                // idle window, but never claims or ends the latch.
                if event.pressed {
                    if let Some(latch) = self.sticky.latches[index].as_mut() {
                        latch.uses = latch.uses.saturating_add(1);
                        if let LatchState::Armed(_) = latch.state {
                            latch.state = LatchState::Armed(timeout_deadline(latch.policy));
                        }
                    }
                } else if latch.policy.max_repeat != 0 && latch.uses >= latch.policy.max_repeat {
                    released |= self.release_latch(index).await;
                }
                continue;
            }
            if event.pressed && latch.policy.release_mode.before_other_key() {
                // The key must not receive the effect, so undo it first.
                released |= self.release_latch(index).await;
                continue;
            }
            if event.pressed {
                let Some(latch) = self.sticky.latches[index].as_mut() else {
                    continue;
                };
                match latch.state {
                    LatchState::Down { since, peers, .. } => {
                        latch.state = LatchState::Down {
                            since,
                            peers,
                            used: true,
                        }
                    }
                    LatchState::Armed(_) | LatchState::Bound => {
                        latch.uses = latch.uses.saturating_add(1);
                        latch.state = LatchState::Bound;
                    }
                }
                continue;
            }
            // A consuming key is going up: release now so that key's report shows
            // the key and the effect leaving together.
            if latch.state != LatchState::Bound {
                continue;
            }
            let repeats_exhausted = latch.policy.max_repeat != 0 && latch.uses >= latch.policy.max_repeat;
            if latch.policy.release_mode.other_key_release() || repeats_exhausted {
                released |= self.release_latch(index).await;
            }
        }
        released
    }

    /// After a foreign action is dispatched: end every latch whose remaining
    /// trigger fired, and re-arm the ones that survived their consumer.
    /// `layers_before` is the layer mask captured just before the dispatch.
    pub(crate) async fn sticky_after(&mut self, event: KeyboardEvent, layers_before: u32, mut released: bool) {
        let layers_now = self.keymap.layer_mask();
        let entered = layers_now & !layers_before != 0;
        let exited = layers_before & !layers_now != 0;

        for index in 0..MAX_ACTIVE {
            let Some(latch) = self.sticky.latches[index] else {
                continue;
            };
            let mode = latch.policy.release_mode;
            let by_layer = (entered && mode.layer_enter()) || (exited && mode.layer_exit());
            if by_layer || (event.pressed && latch.state == LatchState::Bound && mode.other_key_press()) {
                released |= self.release_latch(index).await;
                continue;
            }
            if !event.pressed
                && latch.state == LatchState::Bound
                && let Some(latch) = self.sticky.latches[index].as_mut()
            {
                // The consumer is gone and no key trigger fired: keep the effect
                // and restart its timeout.
                latch.state = LatchState::Armed(timeout_deadline(latch.policy));
            }
        }
        if released {
            self.settle_report().await;
        }
    }

    /// Release every latch whose armed timeout has passed.
    pub(crate) async fn sticky_expire(&mut self) {
        let now = Instant::now();
        let mut released = false;
        for index in 0..MAX_ACTIVE {
            let expired = self.sticky.latches[index]
                .is_some_and(|latch| matches!(latch.state, LatchState::Armed(Some(at)) if at <= now));
            if expired {
                released |= self.release_latch(index).await;
            }
        }
        if released {
            self.settle_report().await;
        }
    }

    /// Undo one latch without reporting, so a caller ending several at once
    /// sends a single report through [`Self::settle_report`]. The result says
    /// whether the host had seen this effect and may therefore need that report.
    async fn release_latch(&mut self, index: usize) -> bool {
        let Some(latch) = self.sticky.latches[index].take() else {
            return false;
        };
        self.coalesce_report = true;
        self.dispatch_action(
            latch.action,
            KeyboardEvent {
                pressed: false,
                ..latch.event
            },
        )
        .await;
        self.coalesce_report = false;
        latch.visible
    }

    /// Report if undoing the latches left the host's view stale. It stays silent
    /// when the surrounding action already sent a report reflecting the change.
    async fn settle_report(&mut self) {
        let report = self.build_keyboard_report(false);
        if (report.modifier, self.held_keycodes) != self.last_report {
            self.send_keyboard_report_with_resolved_modifiers(false).await;
        }
    }
}
