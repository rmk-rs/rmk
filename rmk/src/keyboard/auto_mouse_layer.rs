//! Auto mouse layer
//!
//! Subscribes to [`PointingEvent`]s coming from pointing devices (e.g. PMW3610)
//! and activates a configured layer whenever cursor motion (X/Y axis) is
//! detected.
//!
//! Multiple entries can be configured to give each pointing device its own
//! target_layer/threshold/timeout. For each event the entry whose `device_id` matches
//! the event's `device_id` is selected first; otherwise the entry with
//! `device_id == None` (if any) is used as a fallback. Events that match
//! neither are ignored.
//!
//! The `deactivate_on_key` / `reset_timeout_on_key` options are
//! driven by [`ActionEvent`]s, published when a key action resolves. Tap-hold,
//! morse, and tap dance keys are therefore classified by their final result;
//! see [`keypress_step`] for the actions that cannot be classified.

use embassy_futures::select::{Either, Either3, select, select3};
use embassy_time::{Duration, Instant, Timer};
use heapless::Vec;
use rmk_macro::processor;
use rmk_types::action::Action;
use rmk_types::keycode::{HidKeyCode, KeyCode};
use rmk_types::modifier::ModifierCombination;

use crate::AUTO_MOUSE_LAYER_MAX_NUM;
use crate::config::AutoMouseLayerConfig;
use crate::core_traits::Runnable;
use crate::event::{
    ActionEvent, Axis, AxisValType, EventSubscriber, LayerChangeEvent, PointingEvent, SubscribableEvent,
};
use crate::keymap::KeyMap;
use crate::processor::Processor;

/// [`Runnable`] for the auto mouse layer task.
///
/// Always subscribes to [`PointingEvent`] and [`LayerChangeEvent`]; additionally subscribes
/// to [`ActionEvent`] only when an entry sets `deactivate_on_key` or `reset_timeout_on_key`,
/// so `[event.action].subs` (default `0`) only needs to be raised to `1` when opting into those options.
/// If those options are set while `[event.action].subs` resolves to `0`, startup panics; raise
/// it via a `keyboard.toml` pointed to by `KEYBOARD_TOML_PATH`.
/// Construct with [`AutoMouseLayerRunner::new`] and pass to `run_all!`. If the keymap has no
/// auto mouse layer configured (or every entry's layer is out of range), [`Runnable::run`] parks
/// forever on [`core::future::pending`] so it can sit alongside the other tasks without doing anything.
#[processor(subscribe = [PointingEvent, LayerChangeEvent])]
#[::rmk::macros::runnable_generated]
pub struct AutoMouseLayerRunner<'a, 'k> {
    keymap: &'a KeyMap<'k>,
    entries: Vec<EntryState, AUTO_MOUSE_LAYER_MAX_NUM>,
    /// `true` if any entry has `deactivate_on_key` or `reset_timeout_on_key` set,
    /// so action events must be inspected.
    any_action_event_configured: bool,
}

impl<'a, 'k> AutoMouseLayerRunner<'a, 'k> {
    /// Build the runner from the keymap's `[behavior.auto_mouse_layer]` config.
    pub fn new(keymap: &'a KeyMap<'k>) -> Self {
        let num_layer = keymap.num_layer();
        let configs = keymap.auto_mouse_layer_configs();
        let mut entries: Vec<EntryState, AUTO_MOUSE_LAYER_MAX_NUM> = Vec::new();
        let mut any_action_event_configured = false;
        for config in configs.iter().cloned() {
            if (config.target_layer as usize) >= num_layer {
                warn!(
                    "auto_mouse_layer: configured target_layer {} is out of range (keymap has {} layers); \
                     entry for device_id {:?} will be ignored",
                    config.target_layer, num_layer, config.device_id
                );
                continue;
            }
            // threshold == 0 would short-circuit motion detection — guard against
            // a misconfigured Rust-API caller bypassing AutoMouseLayerConfig::new.
            let mut config = config;
            config.threshold = config.threshold.max(1);
            any_action_event_configured |= config.deactivate_on_key || config.reset_timeout_on_key;
            let device_id = config.device_id;
            if entries
                .push(EntryState {
                    config,
                    self_activated: false,
                    deadline: None,
                    overlap_warned: false,
                })
                .is_err()
            {
                warn!(
                    "auto_mouse_layer: too many entries configured (max {}); entry for device_id {:?} is dropped",
                    AUTO_MOUSE_LAYER_MAX_NUM, device_id
                );
            }
        }
        Self {
            keymap,
            entries,
            any_action_event_configured,
        }
    }

    async fn on_pointing_event(&mut self, event: PointingEvent) {
        let Some(idx) = match_entry(&self.entries, event.device_id) else {
            return;
        };
        if !is_cursor_motion(&event, self.entries[idx].config.threshold) {
            return;
        }
        let target_layer = self.entries[idx].config.target_layer;
        let activated_by_us = self.keymap.activate_layer_if_inactive(target_layer);
        if pointing_step(&mut self.entries, idx, Instant::now(), activated_by_us) == PointingOutcome::OverlapFirstSeen {
            warn!(
                "auto_mouse_layer: layer {} is already active when motion was detected; \
                 the layer is likely driven by another key (MO/TG). The auto mouse layer \
                 will not be deactivated on timeout while overlap holds.",
                target_layer
            );
        }
    }

    async fn on_layer_change_event(&mut self, LayerChangeEvent(top): LayerChangeEvent) {
        // Layer turned off externally (MO/TG key etc.) — release our hold.
        let keymap = self.keymap;
        for entry in self.entries.iter_mut() {
            if entry.self_activated && !keymap.is_layer_active(entry.config.target_layer) {
                entry.self_activated = false;
                entry.deadline = None;
                trace!(
                    "auto_mouse_layer: cleared tracking for layer {} (top now {})",
                    entry.config.target_layer, top
                );
            }
        }
    }

    async fn on_action_event(&mut self, event: ActionEvent) {
        if !event.keyboard_event.pressed || !self.any_action_event_configured {
            return;
        }
        if !self.entries.iter().any(|e| e.self_activated) {
            return;
        }
        for layer in keypress_step(&mut self.entries, event.action, Instant::now()) {
            self.keymap.deactivate_layer_if_active(layer);
        }
    }
}

/// Per-entry runtime state.
#[derive(Clone)]
struct EntryState {
    config: AutoMouseLayerConfig,
    /// `true` while this entry is holding the layer active. Multiple entries
    /// may hold the same layer simultaneously when they share `target_layer`.
    self_activated: bool,
    /// Set when the entry is self-activated; the layer is deactivated when this
    /// time is reached unless further motion pushes the deadline forward.
    deadline: Option<Instant>,
    /// Whether we have already warned about the entry's layer overlapping a
    /// manually-activated layer.
    overlap_warned: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum PointingOutcome {
    Holding,
    OverlapFirstSeen,
    Idle,
}

impl AutoMouseLayerRunner<'_, '_> {
    fn deadline(&self) -> Option<Instant> {
        earliest_deadline(&self.entries)
    }

    async fn on_deadline(&mut self) {
        for layer in timeout_step(&mut self.entries, Instant::now()) {
            self.keymap.deactivate_layer_if_active(layer);
        }
    }
}

impl Runnable for AutoMouseLayerRunner<'_, '_> {
    async fn run(&mut self) -> ! {
        if self.entries.is_empty() {
            core::future::pending().await
        }
        let mut sub = <Self as Processor>::subscriber();
        assert!(
            !self.any_action_event_configured || crate::ACTION_EVENT_SUB_SIZE != 0,
            "auto_mouse_layer: deactivate_on_key / reset_timeout_on_key need `[event.action] subs = 1`"
        );
        let mut action_sub = self.any_action_event_configured.then(ActionEvent::subscriber);
        loop {
            let action_fut = async {
                match action_sub.as_mut() {
                    Some(action_sub) => action_sub.next_event().await,
                    None => core::future::pending().await,
                }
            };
            match self.deadline() {
                Some(deadline) => match select3(Timer::at(deadline), sub.next_event(), action_fut).await {
                    Either3::First(_) => self.on_deadline().await,
                    Either3::Second(event) => self.process(event).await,
                    Either3::Third(event) => self.on_action_event(event).await,
                },
                None => match select(sub.next_event(), action_fut).await {
                    Either::First(event) => self.process(event).await,
                    Either::Second(event) => self.on_action_event(event).await,
                },
            }
        }
    }
}

fn earliest_deadline(entries: &[EntryState]) -> Option<Instant> {
    entries.iter().filter_map(|e| e.deadline).min()
}

/// Find the entry that should handle an event from `device_id`.
///
/// Exact `device_id` match wins; otherwise the first entry with
/// `device_id == None` is used as a fallback. Returns `None` if the event
/// matches neither.
fn match_entry(entries: &[EntryState], device_id: u8) -> Option<usize> {
    if let Some(i) = entries.iter().position(|e| e.config.device_id == Some(device_id)) {
        return Some(i);
    }
    entries.iter().position(|e| e.config.device_id.is_none())
}

fn layer_still_held(entries: &[EntryState], layer: u8) -> bool {
    entries
        .iter()
        .any(|e| e.self_activated && e.config.target_layer == layer)
}

/// Whether some entry other than `idx` self-holds `layer`.
fn layer_shared_with_other(entries: &[EntryState], idx: usize, layer: u8) -> bool {
    entries
        .iter()
        .enumerate()
        .any(|(i, e)| i != idx && e.self_activated && e.config.target_layer == layer)
}

fn timeout_step(entries: &mut [EntryState], now: Instant) -> Vec<u8, AUTO_MOUSE_LAYER_MAX_NUM> {
    let mut released: Vec<u8, AUTO_MOUSE_LAYER_MAX_NUM> = Vec::new();
    for i in 0..entries.len() {
        let expired = entries[i].self_activated && entries[i].deadline.is_some_and(|d| d <= now);
        if !expired {
            continue;
        }
        entries[i].self_activated = false;
        entries[i].deadline = None;
        let layer = entries[i].config.target_layer;
        if !layer_still_held(entries, layer) {
            let _ = released.push(layer);
        }
    }
    released
}

fn pointing_step(entries: &mut [EntryState], idx: usize, now: Instant, activated_by_us: bool) -> PointingOutcome {
    let target_layer = entries[idx].config.target_layer;
    let shared_with_other = layer_shared_with_other(entries, idx, target_layer);
    let entry = &mut entries[idx];
    if activated_by_us || shared_with_other {
        entry.self_activated = true;
        entry.overlap_warned = false;
        entry.deadline = Some(now + entry.config.timeout);
        PointingOutcome::Holding
    } else if entry.self_activated {
        entry.deadline = Some(now + entry.config.timeout);
        PointingOutcome::Idle
    } else if !entry.overlap_warned {
        entry.overlap_warned = true;
        PointingOutcome::OverlapFirstSeen
    } else {
        PointingOutcome::Idle
    }
}

/// Handle a key press for opt-in entries: release entries whose layer should deactivate,
/// or extend the deadline for entries opted into `reset_timeout_on_key`. Returns the
/// layers that no sibling still holds after this step.
///
/// Actions that emit no single keycode/modifier set (layer switches, macros,
/// `Again`/`Repeat`, `GraveEscape`, ...) are unclassifiable and never deactivate;
/// the timeout path clears the layer instead.
fn keypress_step(entries: &mut [EntryState], action: Action, now: Instant) -> Vec<u8, AUTO_MOUSE_LAYER_MAX_NUM> {
    let mut released: Vec<u8, AUTO_MOUSE_LAYER_MAX_NUM> = Vec::new();
    for i in 0..entries.len() {
        if !entries[i].self_activated {
            continue;
        }
        let cfg = &entries[i].config;
        // Classify: does this key press cause deactivation for this entry?
        // Only meaningful when `deactivate_on_key` is set.
        let causes_deactivation = cfg.deactivate_on_key
            && match action {
                // The repeated keycode is unknown here; treat as unclassifiable
                // so a repeated mouse key is not misclassified as non-mouse.
                Action::Key(KeyCode::Hid(HidKeyCode::Again)) => false,
                Action::Key(kc) => match kc {
                    KeyCode::Hid(hid) if hid.is_mouse_key() => false,
                    _ => !cfg.extra_mouse_keys.contains(&kc),
                },
                Action::KeyWithModifier(hid, _) | Action::OneShotKey(hid) => {
                    if hid.is_mouse_key() {
                        false
                    } else {
                        !cfg.extra_mouse_keys.contains(&KeyCode::Hid(hid))
                    }
                }
                // A modifier-only action (e.g. MT hold) deactivates unless every
                // contained modifier is covered by a modifier keycode listed in
                // `extra_mouse_keys` — mirroring how plain modifier keys behave.
                Action::Modifier(modifiers) => {
                    let covered = cfg
                        .extra_mouse_keys
                        .iter()
                        .fold(ModifierCombination::new(), |acc, kc| match kc {
                            KeyCode::Hid(hid) => acc | hid.to_hid_modifiers(),
                            _ => acc,
                        });
                    (modifiers & !covered).into_bits() != 0
                }
                // Unclassifiable (layer switches, macros, Again/Repeat, GraveEscape, ...):
                // leave the layer intact; the timeout path handles it.
                _ => false,
            };
        if causes_deactivation {
            entries[i].self_activated = false;
            entries[i].deadline = None;
            entries[i].overlap_warned = false;
            let layer = entries[i].config.target_layer;
            if !layer_still_held(entries, layer) {
                let _ = released.push(layer);
            }
        } else if cfg.reset_timeout_on_key {
            let timeout = cfg.timeout;
            extend_deadline(&mut entries[i], now, timeout);
        }
    }
    released
}

/// Push `entry.deadline` forward to `now + timeout` if it would extend, not shorten, the current deadline.
fn extend_deadline(entry: &mut EntryState, now: Instant, timeout: Duration) {
    let new_deadline = now + timeout;
    match entry.deadline {
        Some(current) if current >= new_deadline => {}
        _ => entry.deadline = Some(new_deadline),
    }
}

/// Only relative X/Y axis deltas count as cursor motion. Scroll-only events
/// (Z/H/V) do not activate the layer.
///
/// Absolute-position axes ([`AxisValType::Abs`], e.g. analogue joysticks) are
/// also ignored here: their `value` reports the current position rather than a
/// delta, so a stick held off-centre would keep the layer pinned on forever.
/// Absolute pointing devices need to be converted to relative deltas upstream.
fn is_cursor_motion(event: &PointingEvent, threshold: u16) -> bool {
    event.axes.iter().any(|axis| {
        matches!(axis.typ, AxisValType::Rel)
            && matches!(axis.axis, Axis::X | Axis::Y)
            && axis.value.unsigned_abs() >= threshold
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AxisEvent, AxisValType};

    fn axis(axis: Axis, value: i16) -> AxisEvent {
        AxisEvent {
            typ: AxisValType::Rel,
            axis,
            value,
        }
    }

    fn abs_axis(axis: Axis, value: i16) -> AxisEvent {
        AxisEvent {
            typ: AxisValType::Abs,
            axis,
            value,
        }
    }

    fn event(axes: [AxisEvent; 3]) -> PointingEvent {
        event_for(0, axes)
    }

    fn event_for(device_id: u8, axes: [AxisEvent; 3]) -> PointingEvent {
        PointingEvent { device_id, axes }
    }

    fn entry(device_id: Option<u8>) -> EntryState {
        EntryState {
            config: AutoMouseLayerConfig {
                device_id,
                target_layer: 0,
                timeout: embassy_time::Duration::from_millis(100),
                threshold: 1,
                deactivate_on_key: false,
                extra_mouse_keys: &[],
                reset_timeout_on_key: false,
            },
            self_activated: false,
            deadline: None,
            overlap_warned: false,
        }
    }

    #[test]
    fn is_cursor_motion_detects_x_or_y() {
        let zero = axis(Axis::Z, 0);
        assert!(is_cursor_motion(&event([axis(Axis::X, 5), zero, zero]), 1u16));
        assert!(is_cursor_motion(&event([zero, axis(Axis::Y, -3), zero]), 1u16));
    }

    #[test]
    fn is_cursor_motion_ignores_scroll_axes() {
        let zero = axis(Axis::X, 0);
        assert!(!is_cursor_motion(&event([zero, zero, axis(Axis::Z, 100)]), 1u16));
        assert!(!is_cursor_motion(&event([zero, zero, axis(Axis::V, 100)]), 1u16));
        assert!(!is_cursor_motion(&event([zero, zero, axis(Axis::H, 100)]), 1u16));
    }

    #[test]
    fn is_cursor_motion_applies_threshold() {
        let zero = axis(Axis::Y, 0);
        assert!(!is_cursor_motion(&event([axis(Axis::X, 2), zero, zero]), 3u16));
        assert!(is_cursor_motion(&event([axis(Axis::X, 3), zero, zero]), 3u16));
    }

    #[test]
    fn is_cursor_motion_detects_when_both_x_and_y_present() {
        // X and Y in the same event are checked independently; either one
        // crossing the threshold triggers motion.
        let zero_z = axis(Axis::Z, 0);
        assert!(is_cursor_motion(
            &event([axis(Axis::X, 4), axis(Axis::Y, 7), zero_z]),
            3u16
        ));
        // Sub-threshold X paired with above-threshold Y still triggers.
        assert!(is_cursor_motion(
            &event([axis(Axis::X, 1), axis(Axis::Y, 5), zero_z]),
            3u16
        ));
        // Both sub-threshold: deltas are NOT summed, so no motion.
        assert!(!is_cursor_motion(
            &event([axis(Axis::X, 2), axis(Axis::Y, 2), zero_z]),
            3u16
        ));
    }

    #[test]
    fn is_cursor_motion_ignores_absolute_component_when_mixed_with_relative() {
        // Above-threshold Abs X must not trigger even when paired with a
        // sub-threshold Rel Y: only Rel deltas count.
        let zero_z = axis(Axis::Z, 0);
        assert!(!is_cursor_motion(
            &event([abs_axis(Axis::X, i16::MAX), axis(Axis::Y, 1), zero_z]),
            3u16
        ));
        // ... but an above-threshold Rel Y in the same event does trigger.
        assert!(is_cursor_motion(
            &event([abs_axis(Axis::X, i16::MAX), axis(Axis::Y, 5), zero_z]),
            3u16
        ));
    }

    #[test]
    fn is_cursor_motion_ignores_absolute_axes() {
        // A joystick reporting an off-centre position via absolute X/Y must not
        // be treated as motion — otherwise the layer would stick on forever
        // while the stick is held.
        let zero = axis(Axis::Y, 0);
        assert!(!is_cursor_motion(
            &event([abs_axis(Axis::X, i16::MAX), zero, zero]),
            1u16
        ));
        assert!(!is_cursor_motion(&event([zero, abs_axis(Axis::Y, -32000), zero]), 1u16));
    }

    #[test]
    fn match_entry_prefers_exact_device_id() {
        // Order intentionally puts the fallback first to confirm the exact
        // match wins regardless of position.
        let entries = [entry(None), entry(Some(1)), entry(Some(2))];
        assert_eq!(match_entry(&entries, 1), Some(1));
        assert_eq!(match_entry(&entries, 2), Some(2));
    }

    #[test]
    fn match_entry_falls_back_to_no_device_id() {
        let entries = [entry(Some(1)), entry(None), entry(Some(2))];
        // device_id 7 is configured nowhere → fallback entry (index 1).
        assert_eq!(match_entry(&entries, 7), Some(1));
    }

    #[test]
    fn match_entry_returns_none_when_no_match_and_no_fallback() {
        let entries = [entry(Some(1)), entry(Some(2))];
        assert_eq!(match_entry(&entries, 7), None);
    }

    fn active_entry(device_id: Option<u8>, target_layer: u8) -> EntryState {
        let mut e = entry(device_id);
        e.config.target_layer = target_layer;
        e.self_activated = true;
        e
    }

    #[test]
    fn layer_still_held_true_when_any_entry_self_holds_the_layer() {
        let entries = [active_entry(Some(1), 3), entry(Some(2))];
        assert!(layer_still_held(&entries, 3));
    }

    #[test]
    fn layer_still_held_false_when_no_entry_active_on_that_layer() {
        let entries = [entry(Some(1)), active_entry(Some(2), 5)];
        assert!(!layer_still_held(&entries, 3));
    }

    #[test]
    fn layer_still_held_ignores_entries_not_self_activated() {
        let mut e = entry(Some(1));
        e.config.target_layer = 2;
        assert!(!layer_still_held(&[e], 2));
    }

    #[test]
    fn layer_shared_with_other_true_when_another_self_activated_entry_holds_same_layer() {
        let entries = [active_entry(Some(1), 2), active_entry(Some(2), 2)];
        assert!(layer_shared_with_other(&entries, 0, 2));
    }

    #[test]
    fn layer_shared_with_other_excludes_self_index() {
        let entries = [active_entry(Some(1), 2), entry(Some(2))];
        assert!(!layer_shared_with_other(&entries, 0, 2));
    }

    #[test]
    fn layer_shared_with_other_ignores_other_layers() {
        let entries = [active_entry(Some(1), 2), active_entry(Some(2), 5)];
        assert!(!layer_shared_with_other(&entries, 0, 2));
    }

    fn at(millis: u64) -> Instant {
        Instant::from_millis(millis)
    }

    fn entry_with_layer(device_id: Option<u8>, target_layer: u8) -> EntryState {
        let mut e = entry(device_id);
        e.config.target_layer = target_layer;
        e
    }

    #[test]
    fn timeout_step_releases_layer_when_last_holder_expires() {
        let mut entries = [entry_with_layer(Some(1), 3)];
        entries[0].self_activated = true;
        entries[0].deadline = Some(at(100));

        let released = timeout_step(&mut entries, at(150));

        assert_eq!(released.as_slice(), &[3]);
        assert!(!entries[0].self_activated);
        assert!(entries[0].deadline.is_none());
    }

    #[test]
    fn timeout_step_keeps_shared_layer_alive_while_a_sibling_still_holds_it() {
        let mut entries = [entry_with_layer(Some(1), 2), entry_with_layer(Some(2), 2)];
        entries[0].self_activated = true;
        entries[0].deadline = Some(at(50));
        entries[1].self_activated = true;
        entries[1].deadline = Some(at(500));

        let released = timeout_step(&mut entries, at(100));

        assert!(released.is_empty());
        assert!(!entries[0].self_activated);
        assert!(entries[0].deadline.is_none());
        assert!(entries[1].self_activated);
        assert_eq!(entries[1].deadline, Some(at(500)));
    }

    #[test]
    fn timeout_step_releases_shared_layer_when_all_holders_expire_simultaneously() {
        let mut entries = [entry_with_layer(Some(1), 4), entry_with_layer(Some(2), 4)];
        entries[0].self_activated = true;
        entries[0].deadline = Some(at(50));
        entries[1].self_activated = true;
        entries[1].deadline = Some(at(80));

        let released = timeout_step(&mut entries, at(100));

        assert_eq!(released.as_slice(), &[4]);
        assert!(!entries[0].self_activated);
        assert!(!entries[1].self_activated);
    }

    #[test]
    fn timeout_step_ignores_entries_that_are_not_self_activated() {
        let mut entries = [entry_with_layer(Some(1), 1)];
        entries[0].self_activated = false;
        entries[0].deadline = Some(at(10));

        let released = timeout_step(&mut entries, at(500));

        assert!(released.is_empty());
    }

    #[test]
    fn pointing_step_holds_layer_when_activation_succeeds() {
        let mut entries = [entry_with_layer(Some(1), 2)];

        let outcome = pointing_step(&mut entries, 0, at(1000), true);

        assert_eq!(outcome, PointingOutcome::Holding);
        assert!(entries[0].self_activated);
        assert_eq!(entries[0].deadline, Some(at(1000) + entries[0].config.timeout));
        assert!(!entries[0].overlap_warned);
    }

    #[test]
    fn pointing_step_piggybacks_on_a_sibling_that_already_holds_the_shared_layer() {
        let mut entries = [entry_with_layer(Some(1), 2), entry_with_layer(Some(2), 2)];
        entries[0].self_activated = true;
        entries[0].deadline = Some(at(500));

        let outcome = pointing_step(&mut entries, 1, at(1000), false);

        assert_eq!(outcome, PointingOutcome::Holding);
        assert!(entries[1].self_activated);
        assert_eq!(entries[1].deadline, Some(at(1000) + entries[1].config.timeout));
        assert!(entries[0].self_activated);
        assert_eq!(entries[0].deadline, Some(at(500)));
    }

    #[test]
    fn pointing_step_warns_once_when_layer_is_externally_active() {
        let mut entries = [entry_with_layer(Some(1), 2)];

        let first = pointing_step(&mut entries, 0, at(1000), false);
        assert_eq!(first, PointingOutcome::OverlapFirstSeen);
        assert!(!entries[0].self_activated);
        assert!(entries[0].deadline.is_none());
        assert!(entries[0].overlap_warned);

        let second = pointing_step(&mut entries, 0, at(1100), false);
        assert_eq!(second, PointingOutcome::Idle);
    }

    #[test]
    fn pointing_step_extends_deadline_on_repeated_motion_from_holding_entry() {
        let mut entries = [entry_with_layer(Some(1), 3)];
        entries[0].self_activated = true;
        entries[0].deadline = Some(at(200));

        let outcome = pointing_step(&mut entries, 0, at(1000), false);

        assert_eq!(outcome, PointingOutcome::Idle);
        assert!(entries[0].self_activated);
        assert_eq!(entries[0].deadline, Some(at(1000) + entries[0].config.timeout));
    }

    #[test]
    fn pointing_step_resets_overlap_warned_when_we_regain_hold() {
        let mut entries = [entry_with_layer(Some(1), 2)];

        let first = pointing_step(&mut entries, 0, at(1000), false);
        assert_eq!(first, PointingOutcome::OverlapFirstSeen);
        assert!(entries[0].overlap_warned);

        let second = pointing_step(&mut entries, 0, at(1100), true);
        assert_eq!(second, PointingOutcome::Holding);
        assert!(entries[0].self_activated);
        assert!(!entries[0].overlap_warned);
        assert_eq!(entries[0].deadline, Some(at(1100) + entries[0].config.timeout));
    }

    #[test]
    fn timeout_step_releases_multiple_distinct_layers_that_expire_together() {
        let mut entries = [entry_with_layer(Some(1), 3), entry_with_layer(Some(2), 5)];
        entries[0].self_activated = true;
        entries[0].deadline = Some(at(100));
        entries[1].self_activated = true;
        entries[1].deadline = Some(at(150));

        let released = timeout_step(&mut entries, at(200));

        assert_eq!(released.len(), 2);
        assert!(released.contains(&3));
        assert!(released.contains(&5));
        assert!(!entries[0].self_activated);
        assert!(!entries[1].self_activated);
    }

    #[test]
    fn timeout_step_expires_entry_when_deadline_equals_now() {
        let mut entries = [entry_with_layer(Some(1), 3)];
        entries[0].self_activated = true;
        entries[0].deadline = Some(at(100));

        let released = timeout_step(&mut entries, at(100));

        assert_eq!(released.as_slice(), &[3]);
        assert!(!entries[0].self_activated);
    }

    // ── deactivate_on_key ────────────────────────────────────────

    fn holding_entry_with_deactivate(target_layer: u8, exceptions: &'static [KeyCode]) -> EntryState {
        let mut e = entry_with_layer(Some(1), target_layer);
        e.config.deactivate_on_key = true;
        e.config.extra_mouse_keys = exceptions;
        e.self_activated = true;
        e.deadline = Some(at(1000));
        e
    }

    #[test]
    fn keypress_step_releases_layer_when_non_mouse_non_exception_key_pressed() {
        // Sample across the non-mouse HID range (letter, digit, whitespace, control, punctuation)
        // to guard against `is_mouse_key()` accidentally classifying non-mouse keys as mouse.
        let non_mouse_keys = [
            HidKeyCode::A,
            HidKeyCode::Z,
            HidKeyCode::Kc0,
            HidKeyCode::Space,
            HidKeyCode::Enter,
            HidKeyCode::Escape,
            HidKeyCode::Semicolon,
        ];
        for hid in non_mouse_keys {
            let mut entries = [holding_entry_with_deactivate(3, &[])];

            let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(hid)), at(2000));

            assert_eq!(released.as_slice(), &[3], "key {:?} should release layer", hid);
            assert!(!entries[0].self_activated, "key {:?} should clear self_activated", hid);
            assert!(entries[0].deadline.is_none(), "key {:?} should clear deadline", hid);
        }
    }

    #[test]
    fn keypress_step_keeps_layer_active_for_all_mouse_key_variants() {
        // Guards against silent divergence if HidKeyCode's mouse range (MouseUp..=MouseAccel2) is extended.
        let mouse_keys = [
            HidKeyCode::MouseUp,
            HidKeyCode::MouseDown,
            HidKeyCode::MouseLeft,
            HidKeyCode::MouseRight,
            HidKeyCode::MouseBtn1,
            HidKeyCode::MouseBtn2,
            HidKeyCode::MouseBtn3,
            HidKeyCode::MouseBtn4,
            HidKeyCode::MouseBtn5,
            HidKeyCode::MouseBtn6,
            HidKeyCode::MouseBtn7,
            HidKeyCode::MouseBtn8,
            HidKeyCode::MouseWheelUp,
            HidKeyCode::MouseWheelDown,
            HidKeyCode::MouseWheelLeft,
            HidKeyCode::MouseWheelRight,
            HidKeyCode::MouseAccel0,
            HidKeyCode::MouseAccel1,
            HidKeyCode::MouseAccel2,
        ];
        for hid in mouse_keys {
            assert!(
                hid.is_mouse_key(),
                "HidKeyCode::{:?} must be classified as a mouse key",
                hid
            );
            let mut entries = [holding_entry_with_deactivate(3, &[])];

            let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(hid)), at(2000));

            assert!(
                released.is_empty(),
                "mouse key {:?} unexpectedly released the layer",
                hid
            );
            assert!(
                entries[0].self_activated,
                "mouse key {:?} unexpectedly deactivated the entry",
                hid
            );
        }
    }

    #[test]
    fn keypress_step_keeps_layer_active_for_exception_key() {
        let mut entries = [holding_entry_with_deactivate(3, &[KeyCode::Hid(HidKeyCode::LCtrl)])];

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::LCtrl)), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
        // Exception keys do NOT extend the deadline; timeout still applies.
        assert_eq!(entries[0].deadline, Some(at(1000)));
    }

    #[test]
    fn keypress_step_releases_layer_for_consumer_and_system_control_keys() {
        use rmk_types::keycode::{ConsumerKey, SystemControlKey};
        // Non-Hid KeyCode variants are not mouse keys and (unless explicitly listed in
        // extra_mouse_keys) should trigger deactivation just like other non-mouse keys.
        let non_hid_keys = [
            KeyCode::Consumer(ConsumerKey::VolumeIncrement),
            KeyCode::Consumer(ConsumerKey::PlayPause),
            KeyCode::SystemControl(SystemControlKey::Sleep),
        ];
        for kc in non_hid_keys {
            let mut entries = [holding_entry_with_deactivate(3, &[])];

            let released = keypress_step(&mut entries, Action::Key(kc), at(2000));

            assert_eq!(released.as_slice(), &[3], "{:?} should release layer", kc);
            assert!(!entries[0].self_activated, "{:?} should clear self_activated", kc);
        }
    }

    #[test]
    fn keypress_step_ignores_entries_that_did_not_opt_in() {
        let mut entries = [entry_with_layer(Some(1), 3)];
        entries[0].self_activated = true;
        entries[0].deadline = Some(at(1000));
        // deactivate_on_key stays false, even a non-mouse key press
        // must NOT release the layer for this entry.

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
    }

    #[test]
    fn keypress_step_keeps_shared_layer_alive_when_sibling_still_holds_it() {
        let mut entries = [holding_entry_with_deactivate(4, &[]), entry_with_layer(Some(2), 4)];
        // Sibling is a plain (non-opt-in) auto-mouse entry that also self-holds
        // layer 4; releasing the opt-in entry must leave the physical layer on.
        entries[1].self_activated = true;
        entries[1].deadline = Some(at(2000));

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        assert!(released.is_empty());
        assert!(!entries[0].self_activated);
        assert!(entries[1].self_activated);
    }

    #[test]
    fn keypress_step_releases_both_opt_in_entries_sharing_a_layer() {
        // Two opt-in entries hold the same physical layer. A non-mouse keypress
        // must release both, and the layer only ends up in `released` once.
        let mut entries = [
            holding_entry_with_deactivate(4, &[]),
            holding_entry_with_deactivate(4, &[]),
        ];
        entries[1].config.device_id = Some(2);

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        assert_eq!(released.as_slice(), &[4]);
        assert!(!entries[0].self_activated);
        assert!(!entries[1].self_activated);
    }

    #[test]
    fn keypress_step_holds_shared_layer_when_a_sibling_opt_in_entry_still_holds_it() {
        // Two opt-in entries share layer 4 but only one gets triggered.
        // The second is bound to a different device_id and matches a different key.
        // We simulate this by leaving the second one active and pressing a key that
        // is in its exceptions but not the first one's.
        const CTRL: KeyCode = KeyCode::Hid(HidKeyCode::LCtrl);
        let mut entries = [
            holding_entry_with_deactivate(4, &[]),
            holding_entry_with_deactivate(4, &[CTRL]),
        ];
        entries[1].config.device_id = Some(2);

        let released = keypress_step(&mut entries, Action::Key(CTRL), at(2000));

        // First entry deactivates (LCtrl not in its exceptions), but layer 4
        // must not be released because the second entry (with LCtrl in exceptions) still holds it.
        assert!(released.is_empty());
        assert!(!entries[0].self_activated);
        assert!(entries[1].self_activated);
    }

    #[test]
    fn keypress_step_does_nothing_when_no_entry_is_self_activated() {
        // Opt-in entries exist but none are currently holding the layer.
        let mut entries = [holding_entry_with_deactivate(3, &[])];
        entries[0].self_activated = false;
        entries[0].deadline = None;

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        assert!(released.is_empty());
        assert!(!entries[0].self_activated);
    }

    #[test]
    fn keypress_step_handles_mixed_opt_in_and_non_opt_in_across_layers() {
        // 4-entry mix: two opt-in on layer 5, one non-opt-in on layer 5, one non-opt-in on layer 6.
        let mut entries: [EntryState; 4] = [
            holding_entry_with_deactivate(5, &[]),
            holding_entry_with_deactivate(5, &[]),
            entry_with_layer(Some(3), 5),
            entry_with_layer(Some(4), 6),
        ];
        entries[1].config.device_id = Some(2);
        entries[2].self_activated = true;
        entries[2].deadline = Some(at(3000));
        entries[3].self_activated = true;
        entries[3].deadline = Some(at(4000));

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        // Layer 5 stays because the non-opt-in entry still holds it; layer 6 is untouched.
        assert!(released.is_empty());
        assert!(!entries[0].self_activated);
        assert!(!entries[1].self_activated);
        assert!(entries[2].self_activated);
        assert!(entries[3].self_activated);
    }

    #[test]
    fn keypress_step_honours_full_exceptions_list() {
        // Verify contains() walks the whole slice, not just the prefix: place the
        // matching key at the very end of a long list.
        const EXCEPTIONS: &[KeyCode] = &[
            KeyCode::Hid(HidKeyCode::A),
            KeyCode::Hid(HidKeyCode::B),
            KeyCode::Hid(HidKeyCode::C),
            KeyCode::Hid(HidKeyCode::D),
            KeyCode::Hid(HidKeyCode::E),
            KeyCode::Hid(HidKeyCode::F),
            KeyCode::Hid(HidKeyCode::G),
            KeyCode::Hid(HidKeyCode::LCtrl),
        ];
        let last = *EXCEPTIONS.last().unwrap();
        let mut entries = [holding_entry_with_deactivate(3, EXCEPTIONS)];

        let released = keypress_step(&mut entries, Action::Key(last), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
    }

    // ── unclassifiable actions ─────────────────────────────────────────────

    #[test]
    fn keypress_step_keeps_layer_active_for_non_key_actions() {
        // Layer switches, macros, and one-shot modifiers emit no keycode and
        // must never deactivate; the timeout path handles clearing.
        for action in [
            Action::LayerOn(2),
            Action::TriggerMacro(0),
            Action::OneShotModifier(ModifierCombination::LCTRL),
        ] {
            let mut entries = [holding_entry_with_deactivate(3, &[])];
            let released = keypress_step(&mut entries, action, at(2000));
            assert!(released.is_empty(), "{:?} should not release layer", action);
            assert!(
                entries[0].self_activated,
                "{:?} should not clear self_activated",
                action
            );
        }
    }

    #[test]
    fn keypress_step_treats_repeat_style_keys_as_unclassifiable() {
        use rmk_types::keycode::SpecialKey;
        // The repeated keycode is unknown here; `Again` especially must not be
        // misclassified as a non-mouse key.
        for action in [
            Action::Key(KeyCode::Hid(HidKeyCode::Again)),
            Action::Special(SpecialKey::Repeat),
            Action::Special(SpecialKey::GraveEscape),
        ] {
            let mut entries = [holding_entry_with_deactivate(3, &[])];
            let released = keypress_step(&mut entries, action, at(2000));
            assert!(released.is_empty(), "{:?} should not release layer", action);
            assert!(
                entries[0].self_activated,
                "{:?} should not clear self_activated",
                action
            );
        }
    }

    // ── modifier-only actions (e.g. MT hold) ─────────────────────────────

    #[test]
    fn keypress_step_releases_layer_for_modifier_action_not_in_exceptions() {
        let mut entries = [holding_entry_with_deactivate(3, &[])];

        let released = keypress_step(&mut entries, Action::Modifier(ModifierCombination::LSHIFT), at(2000));

        assert_eq!(released.as_slice(), &[3]);
        assert!(!entries[0].self_activated);
    }

    #[test]
    fn keypress_step_keeps_layer_active_for_modifier_action_fully_covered_by_exceptions() {
        let mut entries = [holding_entry_with_deactivate(
            3,
            &[KeyCode::Hid(HidKeyCode::LCtrl), KeyCode::Hid(HidKeyCode::LShift)],
        )];

        let released = keypress_step(
            &mut entries,
            Action::Modifier(ModifierCombination::LCTRL | ModifierCombination::LSHIFT),
            at(2000),
        );

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
        // Like exception keys, covered modifiers do NOT extend the deadline.
        assert_eq!(entries[0].deadline, Some(at(1000)));
    }

    #[test]
    fn keypress_step_releases_layer_for_modifier_action_partially_covered_by_exceptions() {
        // LCtrl is excepted but the action also contains LShift — deactivate.
        let mut entries = [holding_entry_with_deactivate(3, &[KeyCode::Hid(HidKeyCode::LCtrl)])];

        let released = keypress_step(
            &mut entries,
            Action::Modifier(ModifierCombination::LCTRL | ModifierCombination::LSHIFT),
            at(2000),
        );

        assert_eq!(released.as_slice(), &[3]);
        assert!(!entries[0].self_activated);
    }

    #[test]
    fn keypress_step_ignores_side_mismatch_between_modifier_action_and_exceptions() {
        // Left/right variants are distinct: RCtrl is not covered by LCtrl.
        let mut entries = [holding_entry_with_deactivate(3, &[KeyCode::Hid(HidKeyCode::LCtrl)])];

        let released = keypress_step(&mut entries, Action::Modifier(ModifierCombination::RCTRL), at(2000));

        assert_eq!(released.as_slice(), &[3]);
        assert!(!entries[0].self_activated);
    }

    #[test]
    fn keypress_step_keeps_layer_active_for_empty_modifier_action() {
        let mut entries = [holding_entry_with_deactivate(3, &[])];

        let released = keypress_step(&mut entries, Action::Modifier(ModifierCombination::new()), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
    }

    #[test]
    fn keypress_step_extends_deadline_on_covered_modifier_action_when_both_opt_in() {
        let mut e = holding_entry_with_deactivate(3, &[KeyCode::Hid(HidKeyCode::LCtrl)]);
        e.config.reset_timeout_on_key = true;
        e.config.timeout = embassy_time::Duration::from_millis(500);
        let mut entries = [e];

        let released = keypress_step(&mut entries, Action::Modifier(ModifierCombination::LCTRL), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
        assert_eq!(
            entries[0].deadline,
            Some(at(2000) + embassy_time::Duration::from_millis(500))
        );
    }

    // ── reset_timeout_on_key ─────────────────────────────────────────────

    fn holding_entry_with_extend(target_layer: u8, timeout_ms: u64) -> EntryState {
        let mut e = entry_with_layer(Some(1), target_layer);
        e.config.reset_timeout_on_key = true;
        e.config.timeout = embassy_time::Duration::from_millis(timeout_ms);
        e.self_activated = true;
        e.deadline = Some(at(1000));
        e
    }

    #[test]
    fn keypress_step_extends_deadline_on_any_key_when_extend_opt_in_alone() {
        // No `deactivate_on_key`: every key press must extend the deadline.
        let mut entries = [holding_entry_with_extend(3, 500)];

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
        assert_eq!(
            entries[0].deadline,
            Some(at(2000) + embassy_time::Duration::from_millis(500))
        );
    }

    #[test]
    fn keypress_step_extends_deadline_on_mouse_key_when_extend_opt_in() {
        let mut entries = [holding_entry_with_extend(3, 500)];

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::MouseBtn1)), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
        assert_eq!(
            entries[0].deadline,
            Some(at(2000) + embassy_time::Duration::from_millis(500))
        );
    }

    #[test]
    fn keypress_step_extends_deadline_on_composite_action_when_extend_opt_in() {
        // Unclassifiable actions leave the layer intact; extend still applies.
        let mut entries = [holding_entry_with_extend(3, 500)];

        let released = keypress_step(&mut entries, Action::LayerOn(0), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
        assert_eq!(
            entries[0].deadline,
            Some(at(2000) + embassy_time::Duration::from_millis(500))
        );
    }

    #[test]
    fn keypress_step_extends_deadline_on_exception_key_when_both_opt_in() {
        // deactivate_on_key=true, reset_timeout_on_key=true, LCtrl in exceptions:
        // LCtrl does NOT deactivate; deadline is extended.
        let mut e = holding_entry_with_deactivate(3, &[KeyCode::Hid(HidKeyCode::LCtrl)]);
        e.config.reset_timeout_on_key = true;
        e.config.timeout = embassy_time::Duration::from_millis(500);
        let mut entries = [e];

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::LCtrl)), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
        assert_eq!(
            entries[0].deadline,
            Some(at(2000) + embassy_time::Duration::from_millis(500))
        );
    }

    #[test]
    fn keypress_step_does_not_extend_deadline_on_deactivating_key() {
        // Both flags on: a non-mouse, non-exception key deactivates. No extension should occur
        // because the entry is torn down.
        let mut e = holding_entry_with_deactivate(3, &[]);
        e.config.reset_timeout_on_key = true;
        e.config.timeout = embassy_time::Duration::from_millis(500);
        let mut entries = [e];

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        assert_eq!(released.as_slice(), &[3]);
        assert!(!entries[0].self_activated);
        assert!(entries[0].deadline.is_none());
    }

    #[test]
    fn keypress_step_does_not_shorten_deadline_when_extending() {
        // Extending must never shorten a further-out deadline that motion set.
        let mut entries = [holding_entry_with_extend(3, 100)];
        // A pointing event pushed the deadline much further out than key-press would.
        entries[0].deadline = Some(at(10_000));

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].self_activated);
        assert_eq!(entries[0].deadline, Some(at(10_000)));
    }

    #[test]
    fn keypress_step_ignores_extend_for_entries_not_self_activated() {
        let mut entries = [holding_entry_with_extend(3, 500)];
        entries[0].self_activated = false;
        entries[0].deadline = None;

        let released = keypress_step(&mut entries, Action::Key(KeyCode::Hid(HidKeyCode::A)), at(2000));

        assert!(released.is_empty());
        assert!(entries[0].deadline.is_none());
    }
}
