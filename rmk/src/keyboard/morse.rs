use embassy_time::{Duration, Instant};
use rmk_types::action::{Action, KeyAction};
use rmk_types::morse::{HOLD, MorseMode, MorsePattern, TAP};

use crate::event::KeyboardEvent;
use crate::keyboard::Keyboard;
use crate::keyboard::held_buffer::{HeldKey, KeyState};
use crate::keymap::KeyMap;

// 'morse' is an alias for the superset of tap dance and tap hold keys, since their handling have many similarities
impl<'a> Keyboard<'a> {
    // When a morse key reaches timeout after press / release
    pub(crate) async fn handle_morse_timeout(&mut self, key: &HeldKey) {
        assert!(key.action.is_morse());

        match key.state {
            KeyState::Pressed(pattern) => {
                // The time since the key press is longer than the timeout,
                // if there is no possibility for longer morse patterns, trigger the action:
                let pattern = pattern.followed_by_hold();
                debug!("pattern while holding: {:?}", pattern);
                let final_action = Self::try_predict_final_action(self.keymap, &key.action, pattern);
                if let Some(action) = final_action {
                    debug!("hold prediction {:?} -> {:?}", pattern, action);
                    self.process_key_action_normal(action, key.event).await;
                    if let Some(k) = self.held_buffer.find_pos_mut(key.event.pos) {
                        k.state = KeyState::ProcessedButReleaseNotReportedYet(action);
                    }
                } else {
                    // Expect a possible longer morse pattern (or idle timeout after release), so can not finish yet...
                    // Update the state so this test will not run again until the next keypress.
                    if let Some(k) = self.held_buffer.find_pos_mut(key.event.pos) {
                        k.state = KeyState::Holding(pattern);
                    }
                }
            }
            KeyState::Released(pattern) => {
                // The time since the key release is longer than the timeout, trigger the action
                let action = Self::action_from_pattern(self.keymap, &key.action, pattern);
                self.process_key_action_tap(action, key.event).await;
                let _ = self.held_buffer.remove(key.event.pos);
            }
            KeyState::EarlyFired(_) => {
                // Tap was already fired early, just clean up
                let _ = self.held_buffer.remove(key.event.pos);
            }
            _ => unreachable!(),
        };

        // If there's still an unresolved morse key in the held buffer, don't fire normal keys.
        if self.has_unresolved_morse_key() {
            return;
        }

        self.fire_held_non_morse_keys().await;
    }

    pub(crate) async fn process_key_action_morse(
        &mut self,
        key_action: &KeyAction,
        event: KeyboardEvent,
        event_time: Instant,
    ) {
        debug!("Processing morse keys: {:?}", event);
        assert!(key_action.is_morse());

        // Process the morse key
        if event.pressed {
            // Pressed, check the held buffer, update the tap state
            let timeout_time = event_time + Self::morse_timeout(self.keymap, key_action, true);
            match self.held_buffer.find_pos_mut(event.pos) {
                Some(k) => {
                    // The current key is already in the buffer, update its state
                    match k.state {
                        KeyState::Released(pattern) | KeyState::EarlyFired(pattern) => {
                            // `k.press_time` holds the *release* time while in Released/EarlyFired,
                            // so the subtraction below measures "time since last release".
                            if !pattern.is_empty()
                                && pattern.is_all_taps()
                                && let Some(window) = Self::quick_tap_window(self.keymap, key_action)
                                && event_time.saturating_duration_since(k.press_time) <= window
                            {
                                let tap_action = Self::action_from_pattern(self.keymap, key_action, TAP);
                                if tap_action != Action::No {
                                    debug!("Quick-tap fire: {:?}", tap_action);
                                    k.state = KeyState::ProcessedButReleaseNotReportedYet(tap_action);
                                    k.press_time = event_time;
                                    k.timeout_time = timeout_time;
                                    self.process_key_action_normal(tap_action, event).await;
                                    return;
                                }
                            }

                            k.state = KeyState::Pressed(pattern);
                            k.press_time = event_time;
                            k.timeout_time = timeout_time;
                        }
                        _ => {}
                    }
                }
                None => {
                    // Add to buffer
                    self.held_buffer.push(HeldKey::new(
                        event,
                        *key_action,
                        KeyState::Pressed(MorsePattern::default()),
                        event_time,
                        timeout_time,
                    ));
                }
            }
        } else {
            // Release a morse key, which is in the held buffer
            // If there's no possible longer morse pattern, trigger it immediately
            // Otherwise, update the state, wait for the either the next press event or the idle timeout
            if let Some(k) = self.held_buffer.find_pos_mut(event.pos) {
                debug!("Releasing morse key: {:?}", k);
                match k.state {
                    KeyState::Pressed(pattern) => {
                        let released_time = Instant::now(); // TODO? It would be better if the event would carry the real timestamp of the release event!

                        let hold = released_time >= k.timeout_time;

                        let pattern = if hold {
                            debug!("pattern after hold release: {:?}", pattern);
                            pattern.followed_by_hold()
                        } else {
                            debug!("pattern after tap release: {:?}", pattern);
                            pattern.followed_by_tap()
                        };

                        // If the computed pattern is beyond all configured patterns (not in
                        // actions and no configured pattern extends it), reset to base tap/hold.
                        // This handles re-tapping after an early-fired tap when the continuation
                        // pattern (e.g., double_tap) isn't configured.
                        let pattern = match &k.action {
                            KeyAction::Morse(idx) => {
                                if let Some(morse) = self.keymap.get_morse(*idx as usize) {
                                    if !morse.has_pattern_or_continuation(pattern) {
                                        if hold { HOLD } else { TAP }
                                    } else {
                                        pattern
                                    }
                                } else {
                                    pattern
                                }
                            }
                            _ => pattern,
                        };

                        let final_action = Self::try_predict_final_action(self.keymap, &k.action, pattern);

                        if !hold
                            && pattern.is_all_taps()
                            && let Some(action) = final_action
                            && action != Action::No
                            && let Some(window) = Self::quick_tap_window(self.keymap, &k.action)
                        {
                            let stashed_action = k.action;
                            debug!("Stash for quick-tap, fire {:?} immediately", action);
                            let mut press_event = event;
                            press_event.pressed = true;
                            self.process_key_action_tap(action, press_event).await;
                            let gap = Self::morse_timeout(self.keymap, &stashed_action, false);
                            let keep_alive = gap.max(window);
                            if let Some(k) = self.held_buffer.find_pos_mut(event.pos) {
                                k.state = KeyState::EarlyFired(pattern);
                                k.press_time = released_time;
                                k.timeout_time = released_time + keep_alive;
                            }
                            self.held_buffer.keys.sort_unstable_by_key(|k| k.timeout_time);
                            if !self.has_unresolved_morse_key() {
                                self.fire_held_non_morse_keys().await;
                            }
                        } else if let Some(action) = final_action {
                            debug!("released prediction {:?} -> {:?}", pattern, action);
                            // Reached the longest configured morse pattern, trigger the corresponding action immediately
                            self.held_buffer.remove(event.pos); // Remove the key from the held buffer, is like setting to an idle state

                            debug!(
                                "Reached the longest configured morse pattern, trigger corresponding action {:?} immediately",
                                action
                            );

                            // Trigger the morse action immediately
                            let mut press_event = event;
                            press_event.pressed = true;
                            self.process_key_action_tap(action, press_event).await;
                            self.held_buffer.remove(event.pos); // Remove the key from the held buffer, is like setting to an idle state
                        } else {
                            // Expect a possible longer morse pattern (or idle timeout), update the state
                            let early_action = Self::check_early_fire(self.keymap, &k.action, pattern);

                            k.state = KeyState::Released(pattern);
                            // Use current release time for `IdleAfterTap` state
                            k.press_time = released_time; // Use release time as the "press_time"
                            let timeout = Self::morse_timeout(self.keymap, &k.action, false);
                            k.timeout_time = k.press_time + timeout;

                            // Fire the tap immediately if the hold continuation has the same action
                            if let Some(action) = early_action {
                                debug!("Early fire {:?} -> {:?}", pattern, action);
                                let mut press_event = event;
                                press_event.pressed = true;
                                self.process_key_action_tap(action, press_event).await;
                                // Mark as early-fired so fire_held_keys won't re-fire
                                if let Some(k) = self.held_buffer.find_pos_mut(event.pos) {
                                    k.state = KeyState::EarlyFired(pattern);
                                }

                                if !self.has_unresolved_morse_key() {
                                    self.fire_held_non_morse_keys().await;
                                }
                            }
                        }
                    }
                    KeyState::Holding(pattern) => {
                        // The try_predict_final_action => None is already decided, when we entered in Holding mode
                        // So, just expect a possible longer morse pattern (or idle timeout), update the state
                        let released_time = Instant::now(); // TODO? It would be better if the event would carry the real timestamp of the release event!
                        k.state = KeyState::Released(pattern);
                        // Use current release time for `IdleAfterTap` state
                        k.press_time = released_time; // Use release time as the "press_time"
                        k.timeout_time = k.press_time + Self::morse_timeout(self.keymap, &k.action, false);
                    }
                    KeyState::ProcessedButReleaseNotReportedYet(action) => {
                        // Releasing a tap-hold action whose pressed HID report is already sent
                        info!("Releasing a morse action whose pressed action is already triggered");
                        let _ = self.held_buffer.remove(event.pos);
                        // Process the release action
                        debug!("[morse] Releasing morse key: {:?}", event);
                        self.process_key_action_normal(action, event).await;
                    }
                    KeyState::FlowTapped(action) => {
                        // Flow-tap fired the tap action and is holding it down; release it now.
                        debug!("[morse] Releasing flow-tapped morse key: {:?}", event);
                        self.process_key_action_normal(action, event).await;
                        // If the key has a hold-after-tap action, keep it in the buffer as if it
                        // had been early-fired so a re-press within the gap timeout continues into
                        // hold-after-tap (the tap-then-hold repeat). Without this a flow-tapped
                        // tap leaves no trace and the next press-and-hold resolves as a fresh hold.
                        if Self::action_from_pattern(self.keymap, key_action, TAP.followed_by_hold()) != Action::No {
                            let now = Instant::now();
                            let timeout = Self::morse_timeout(self.keymap, key_action, false);
                            if let Some(k) = self.held_buffer.find_pos_mut(event.pos) {
                                k.state = KeyState::EarlyFired(TAP);
                                k.press_time = now;
                                k.timeout_time = now + timeout;
                            }
                        } else {
                            let _ = self.held_buffer.remove(event.pos);
                        }
                    }
                    _ => {}
                };
            }
        }
    }

    pub(crate) async fn fire_held_non_morse_keys(&mut self) {
        self.held_buffer.keys.sort_unstable_by_key(|k| k.press_time);

        // Trigger all non morse keys in the buffer
        while let Some(key) = self.held_buffer.remove_if(|k| !k.action.is_morse()) {
            debug!("Trigger non-morse key: {:?}", key);
            let action = self.keymap.get_action_with_layer_cache(key.event);
            match action {
                KeyAction::Single(action) => self.process_key_action_normal(action, key.event).await,
                KeyAction::Tap(action) => self.process_key_action_tap(action, key.event).await,
                _ => (),
            }
        }

        self.held_buffer.keys.sort_unstable_by_key(|k| k.timeout_time);
    }

    fn has_unresolved_morse_key(&self) -> bool {
        self.held_buffer.keys.iter().any(|k| {
            k.action.is_morse()
                && matches!(
                    k.state,
                    KeyState::Pressed(_) | KeyState::Holding(_) | KeyState::Released(_)
                )
        })
    }

    pub fn action_from_pattern(keymap: &KeyMap, keyAction: &KeyAction, pattern: MorsePattern) -> Action {
        match keyAction {
            KeyAction::TapHold(tap_action, hold_action, _) => match pattern {
                TAP => *tap_action,
                HOLD => *hold_action,
                _ => Action::No,
            },
            KeyAction::Morse(idx) => keymap
                .get_morse(*idx as usize)
                .map(|morse| morse.get(pattern).unwrap_or(Action::No))
                .unwrap_or(Action::No),
            _ => Action::No,
        }
    }

    pub fn quick_tap_window(keymap: &KeyMap, key_action: &KeyAction) -> Option<Duration> {
        let per_key = match key_action {
            KeyAction::TapHold(_, _, idx) => keymap.morse_profile(*idx).quick_tap_timeout_ms(),
            KeyAction::Morse(idx) => keymap
                .get_morse(*idx as usize)
                .and_then(|m| m.profile.quick_tap_timeout_ms()),
            _ => None,
        };
        let timeout = per_key.or_else(|| keymap.morse_default_profile().quick_tap_timeout_ms());
        timeout.filter(|&t| t > 0).map(|t| Duration::from_millis(t as u64))
    }

    pub fn morse_timeout(keymap: &KeyMap, key_action: &KeyAction, hold_timeout_needed: bool) -> Duration {
        // Check per-key profile config first
        match key_action {
            KeyAction::TapHold(_, _, idx) => {
                let profile = keymap.morse_profile(*idx);
                let timeout = if hold_timeout_needed {
                    profile.hold_timeout_ms()
                } else {
                    profile.gap_timeout_ms()
                };

                if let Some(timeout) = timeout {
                    return Duration::from_millis(timeout as u64);
                }
            }
            KeyAction::Morse(index) => {
                if let Some(morse) = keymap.get_morse(*index as usize) {
                    let timeout = if hold_timeout_needed {
                        morse.profile.hold_timeout_ms()
                    } else {
                        morse.profile.gap_timeout_ms()
                    };

                    if let Some(timeout) = timeout {
                        return Duration::from_millis(timeout as u64);
                    }
                }
            }
            _ => {}
        }

        // If no per-key config, use the global default profile
        let default_profile = keymap.morse_default_profile();
        let timeout = if hold_timeout_needed {
            default_profile.hold_timeout_ms()
        } else {
            default_profile.gap_timeout_ms()
        }
        .unwrap_or(250u16);

        Duration::from_millis(if timeout == 0 { 250u16 } else { timeout } as u64)
    }

    /// Decides and returns the morse mode
    /// based on configuration for the given key action / key position
    pub fn tap_hold_mode(keymap: &KeyMap, key_action: &KeyAction) -> MorseMode {
        // Check per-key profile config first
        match key_action {
            KeyAction::TapHold(_, _, idx) => {
                if let Some(mode) = keymap.morse_profile(*idx).mode() {
                    return mode;
                }
            }
            KeyAction::Morse(index) => {
                if let Some(morse) = keymap.get_morse(*index as usize)
                    && let Some(mode) = morse.profile.mode()
                {
                    return mode;
                }
            }
            _ => {}
        }

        // If no per-key config, use the global default profile
        keymap.morse_default_profile().mode().unwrap_or(MorseMode::Normal)
    }

    /// Decides and returns the morse mode
    /// based on configuration for the given key action / key position
    pub fn is_unilateral_tap_enabled(keymap: &KeyMap, key_action: &KeyAction) -> bool {
        // try to look for a per-key profile config
        match key_action {
            KeyAction::TapHold(_, _, idx) => {
                if let Some(enabled) = keymap.morse_profile(*idx).unilateral_tap() {
                    return enabled;
                }
            }
            KeyAction::Morse(index) => {
                if let Some(morse) = keymap.get_morse(*index as usize)
                    && let Some(enabled) = morse.profile.unilateral_tap()
                {
                    return enabled;
                }
            }
            _ => {}
        }

        // Use the global default
        keymap.morse_default_profile().unilateral_tap().unwrap_or(false)
    }

    pub fn is_flow_tap_enabled(keymap: &KeyMap, key_action: &KeyAction) -> bool {
        let per_key = match key_action {
            KeyAction::TapHold(_, _, idx) => keymap.morse_profile(*idx).enable_flow_tap(),
            KeyAction::Morse(index) => keymap
                .get_morse(*index as usize)
                .and_then(|morse| morse.profile.enable_flow_tap()),
            _ => None,
        };
        per_key
            .or_else(|| keymap.morse_default_profile().enable_flow_tap())
            .unwrap_or_else(|| keymap.morse_enable_flow_tap())
    }

    /// Checks if the given pattern can fire its action early even though longer
    /// continuations exist. Returns Some(action) when the hold continuation has
    /// the same action and the tap continuation is not configured.
    pub fn check_early_fire(keymap: &KeyMap, key_action: &KeyAction, pattern: MorsePattern) -> Option<Action> {
        match key_action {
            KeyAction::Morse(idx) => {
                let morse = keymap.get_morse(*idx as usize)?;
                if morse.can_fire_early(pattern) {
                    morse.get(pattern)
                } else {
                    None
                }
            }
            _ => None, // TapHold already handles prediction in try_predict_final_action
        }
    }

    //returns Some(action) if the ending of the given pattern can be "predicted" (unique)
    pub fn try_predict_final_action(
        keymap: &KeyMap,
        keyAction: &KeyAction,
        pattern_start: MorsePattern,
    ) -> Option<Action> {
        match keyAction {
            KeyAction::TapHold(tap_action, hold_action, _) => {
                if pattern_start.last_is_hold() {
                    Some(*hold_action)
                } else {
                    Some(*tap_action)
                }
            }
            KeyAction::Morse(idx) => keymap
                .get_morse(*idx as usize)
                .and_then(|td| td.try_predict_final_action(pattern_start)),
            _ => None,
        }
    }
}
