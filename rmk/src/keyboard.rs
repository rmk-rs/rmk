use core::fmt::Debug;

use embassy_futures::select::{Either, select};
use embassy_futures::yield_now;
#[cfg(feature = "_ble")]
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, with_deadline};
use heapless::Vec;
use rmk_types::action::{Action, KeyAction, KeyboardAction};
use rmk_types::fork::StateBits;
use rmk_types::keyboard_macros::MacroOp;
use rmk_types::keycode::{HidKeyCode, KeyCode, SpecialKey, from_ascii};
use rmk_types::led_indicator::LedIndicator;
use rmk_types::modifier::ModifierCombination;
use rmk_types::morse::{MorseMode, MorsePattern, TAP};
use rmk_types::mouse_button::MouseButtons;
use usbd_hid::descriptor::{MediaKeyboardReport, SystemControlReport};

#[cfg(feature = "_ble")]
use crate::ble::sleep::report_activity;
use crate::channel::send_hid_report;
use crate::core_traits::Runnable;
#[cfg(all(feature = "split", feature = "_ble"))]
use crate::event::ClearPeerEvent;
use crate::event::{
    ActionEvent, KeyboardEvent, KeyboardEventSource, LayerChangeEvent, ModifierEvent, SubscribableEvent, publish_event,
    publish_event_async,
};
use crate::hid::{KeyboardReport, Report};
use crate::keyboard::combo::Combo;
use crate::keyboard::fork::ActiveFork;
use crate::keyboard::held_buffer::{HeldBuffer, HeldKey, KeyState};
use crate::keyboard::mouse::{MouseAction, MouseState};
use crate::keyboard::sticky::{StickyKeyState, action_key};
use crate::keymap::KeyMap;
use crate::{COMBO_MAX_NUM, FORK_MAX_NUM, boot};

pub(crate) mod auto_mouse_layer;
pub mod combo;
pub(crate) mod fork;
pub(crate) mod held_buffer;
pub(crate) mod macros;
pub(crate) mod morse;
pub(crate) mod mouse;
#[cfg(feature = "steno")]
pub(crate) mod steno;
pub(crate) mod sticky;

use crate::keymap::HOLD_BUFFER_SIZE;

// Timestamp of the last key action, the value is the number of seconds since the boot
#[cfg(feature = "_ble")]
pub(crate) static LAST_KEY_TIMESTAMP: Signal<crate::RawMutex, u32> = Signal::new();

/// Led states for the keyboard hid report (its value is received by by the light service in a hid report)
/// LedIndicator type would be nicer, but that does not have const expr constructor
pub(crate) static LOCK_LED_STATES: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0u8);

/// Read the current host-driven lock LED state as a typed [`LedIndicator`].
///
/// Updated by `run_led_reader` whenever the host sends a SET_REPORT for LEDs;
/// host services read it synchronously instead of subscribing to
/// [`LedIndicatorEvent`](crate::event::LedIndicatorEvent).
pub(crate) fn current_led_indicator() -> LedIndicator {
    LedIndicator::from_bits(LOCK_LED_STATES.load(core::sync::atomic::Ordering::Relaxed))
}

/// State machine for Caps Word
#[derive(Debug, Default)]
enum CapsWordState {
    /// Caps Word is activated (but may have timed out thus becoming inactive)
    Activated {
        /// Time since last key press
        timer: Instant,
        /// Whether the current key should be shifted
        shift_current: bool,
    },
    /// Caps Word is deactivated
    #[default]
    Deactivated,
}

impl CapsWordState {
    /// Caps Word timeout duration
    const TIMEOUT: Duration = Duration::from_secs(5);

    /// Activate Caps Word
    fn activate(&mut self) {
        *self = CapsWordState::Activated {
            timer: Instant::now(),
            shift_current: false,
        };
    }

    /// Deactivate Caps Word
    fn deactivate(&mut self) {
        *self = CapsWordState::Deactivated;
    }

    /// Toggle Caps Word
    fn toggle(&mut self) {
        if self.is_active() {
            self.deactivate();
        } else {
            self.activate();
        }
    }

    /// Return whether Caps Word is active (and has not timed out)
    fn is_active(&self) -> bool {
        if let CapsWordState::Activated { timer, .. } = self {
            timer.elapsed() < Self::TIMEOUT
        } else {
            false
        }
    }

    /// Return whether the current key pressed is to be shifted
    fn is_shift_current(&self) -> bool {
        if let CapsWordState::Activated { shift_current, .. } = self {
            *shift_current
        } else {
            false
        }
    }

    /// Check whether to shift the given key, and update the state accordingly
    ///
    /// Note that this function does not check the CapsWord key itself.
    fn check(&mut self, key: HidKeyCode) {
        if let CapsWordState::Activated { timer, shift_current } = self {
            if key.is_caps_word_continue_key() && timer.elapsed() < Self::TIMEOUT {
                *timer = Instant::now();
                *shift_current = key.is_caps_word_shifted_key();
            } else {
                self.deactivate();
            }
        }
    }
}

impl Runnable for Keyboard<'_> {
    /// Main keyboard processing task, it receives input devices result, processes keys.
    /// The report is sent using `send_report`.
    async fn run(&mut self) -> ! {
        let mut layer_events = LayerChangeEvent::subscriber();
        loop {
            // Wait for the next event, but wake up at the earliest pending deadline.
            // `with_deadline` polls the subscriber first, so a queued event is handled first.
            let deadline = self.next_deadline();
            let input = async {
                match deadline {
                    Some(deadline) => with_deadline(deadline, self.keyboard_event_subscriber.next_message_pure())
                        .await
                        .ok(),
                    None => Some(self.keyboard_event_subscriber.next_message_pure().await),
                }
            };
            match select(layer_events.next_message_pure(), input).await {
                Either::First(_) => self.release_stickies(None).await,
                Either::Second(Some(event)) => self.process_inner(event).await,
                Either::Second(None) => self.fire_expired().await,
            }
        }
    }
}

pub struct Keyboard<'a> {
    /// Keymap
    pub(crate) keymap: &'a KeyMap<'a>,

    /// Keyboard event subscriber - single instance to receive all keyboard events
    keyboard_event_subscriber: embassy_sync::pubsub::Subscriber<
        'static,
        crate::RawMutex,
        KeyboardEvent,
        { crate::KEYBOARD_EVENT_CHANNEL_SIZE },
        { crate::KEYBOARD_EVENT_SUB_SIZE },
        { crate::KEYBOARD_EVENT_PUB_SIZE },
    >,

    /// Buffered held keys
    pub held_buffer: HeldBuffer,

    /// Record the timestamp of last **simple key** press.
    /// It's used in tap-hold prior-idle-time check.
    last_press_time: Instant,

    /// stores the last KeyCode executed, to be repeated if the repeat key os pressed
    /// Used in repeat-key
    last_key_code: HidKeyCode,

    /// Sticky keys waiting for the next input. `OSM`/`OSL` live here too.
    stickies: [Option<StickyKeyState>; crate::STICKY_MAX_ACTIVE],

    /// Deduplicate sticky release reports; ordinary repeated reports still pass through.
    last_keyboard_report: (u8, [u8; 6]),

    /// The pending User-key hold gesture: when it fires, and the id of the held key.
    /// Any key event cancels it.
    #[cfg(feature = "_ble")]
    user_hold: Option<(Instant, u8)>,

    /// Caps Word state machine
    caps_word: CapsWordState,

    /// When the next macro op may run.
    macro_due: Instant,

    /// The real state before fork activations is stored here
    fork_states: [Option<ActiveFork>; FORK_MAX_NUM], // chosen replacement key of the currently triggered forks and the related modifier suppression
    fork_keep_mask: ModifierCombination, // aggregate here the explicit modifiers pressed since the last fork activations

    /// Current registered keys, ordered by the press time
    registered: Vec<RegisteredKey, 16>,

    /// Mouse state (report, acceleration, repeat counters, repeat deadlines)
    mouse: MouseState,

    /// Used for temporarily disabling combos
    combo_on: bool,

    /// Plover HID stenography chord accumulator
    #[cfg(feature = "steno")]
    steno: crate::keyboard::steno::StenoChord,

    /// Passkey entry state for BLE pairing
    #[cfg(feature = "passkey_entry")]
    passkey_entry_state: crate::ble::passkey::PasskeyEntryState,
}

impl<'a> Keyboard<'a> {
    pub fn new(keymap: &'a KeyMap<'a>) -> Self {
        Keyboard {
            keymap,
            keyboard_event_subscriber: KeyboardEvent::subscriber(),
            last_press_time: Instant::now(),
            stickies: [const { None }; crate::STICKY_MAX_ACTIVE],
            // The host starts out seeing an empty report.
            last_keyboard_report: (0, [0; 6]),
            #[cfg(feature = "_ble")]
            user_hold: None,
            caps_word: CapsWordState::default(),
            macro_due: Instant::from_ticks(0),
            fork_states: [None; FORK_MAX_NUM],
            fork_keep_mask: ModifierCombination::default(),
            held_buffer: HeldBuffer::new(),
            registered: Vec::new(),
            mouse: MouseState::new(),
            last_key_code: HidKeyCode::No,
            combo_on: true,
            #[cfg(feature = "steno")]
            steno: crate::keyboard::steno::StenoChord::new(),
            #[cfg(feature = "passkey_entry")]
            passkey_entry_state: crate::ble::passkey::PasskeyEntryState::new(),
        }
    }

    /// Send a keyboard report to the host.
    async fn send_report(&mut self, report: Report) {
        // Do not report keypresses to Host in passkey mode
        #[cfg(feature = "passkey_entry")]
        if self.passkey_entry_state.is_active() {
            return;
        }

        if let Report::KeyboardReport(r) = &report {
            self.last_keyboard_report = (r.modifier, r.keycodes);
        }

        send_hid_report(report).await;
    }

    /// A copy of the buffered key that times out first: a combo key waiting for
    /// its partners, or a morse key waiting out its timeout.
    pub fn next_buffered_key(&self) -> Option<HeldKey> {
        self.held_buffer.next_timeout(|k| {
            matches!(k.state, KeyState::WaitingCombo)
                || (k.action.is_morse()
                    && matches!(
                        k.state,
                        KeyState::Pressed(_) | KeyState::Released(_) | KeyState::EarlyFired(_)
                    ))
        })
    }

    /// The earliest time `run()` must wake up. Every deadline returned here has to be
    /// cleared or moved forward by `fire_expired`, otherwise `run()` busy-loops on it.
    fn next_deadline(&self) -> Option<Instant> {
        let buffered = self.next_buffered_key().map(|k| k.timeout_time);
        // A buffered key may still use the sticky it was pressed under, so the
        // sticky can only expire once the buffer is empty.
        let sticky = if buffered.is_some() {
            None
        } else {
            self.next_sticky_deadline()
        };
        [
            sticky,
            #[cfg(feature = "_ble")]
            self.user_hold.map(|(at, _)| at),
            buffered,
            self.mouse.next_deadline(),
            self.keymap.macros(|m| m.is_playing()).then_some(self.macro_due),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Handle every deadline that is due. Each step checks its own deadline, so
    /// calling this too early does nothing.
    async fn fire_expired(&mut self) {
        match self.next_buffered_key() {
            // `next_deadline` hides the sticky deadline while a key is buffered,
            // so at most one of these two can be due.
            Some(key) => self.fire_buffered_key_timeout(key).await,
            None => self.expire_stickies().await,
        }
        #[cfg(feature = "_ble")]
        self.fire_user_hold().await;
        self.fire_mouse_repeat().await;
        self.fire_macro().await;
    }

    /// Resolve `key` if its timeout has passed: dispatch the combo it waits on, or
    /// hand it to the morse timeout. Only one key per call, so `run()` can handle a
    /// queued event before the next timeout.
    async fn fire_buffered_key_timeout(&mut self, key: HeldKey) {
        if key.timeout_time > Instant::now() {
            return;
        }
        match key.state {
            KeyState::WaitingCombo => {
                debug!("[Combo] Timeout, dispatch combo");
                // A timeout is not an interrupting key press: only a delayed
                // combo containing this key may fire.
                let mut timeout_event = key.event;
                timeout_event.pressed = false;
                self.trigger_delayed_combo(&key.action, timeout_event).await;

                if let Some(key) = self
                    .held_buffer
                    .remove_if(|k| k.event.source == key.event.source && k.state == KeyState::WaitingCombo)
                {
                    self.keymap.with_combos_mut(|combos| {
                        combos
                            .iter_mut()
                            .flatten()
                            .filter(|combo| !combo.is_triggered() && combo.config.contains(&key.action))
                            .for_each(Combo::reset);
                    });
                    self.process_key_action(&key.action, key.event, key.press_time()).await;
                }
            }
            _ => {
                debug!("Buffered morse key timeout");
                self.handle_morse_timeout(&key).await;
            }
        }
    }

    /// Process an input source's press or release.
    pub async fn process_inner(&mut self, event: KeyboardEvent) {
        self.release_stickies(None).await;
        // A User-key hold gesture needs 5s without any key event, so cancel it here.
        #[cfg(feature = "_ble")]
        {
            self.user_hold = None;
        }

        // Check for mode transitions (e.g., entering/exiting passkey entry)
        #[cfg(feature = "passkey_entry")]
        self.passkey_entry_state.check_mode_transition();

        #[cfg(feature = "host_lock")]
        self.keymap.update_matrix_state(&event);

        // Report activity for sleep management
        #[cfg(feature = "_ble")]
        report_activity();

        if let KeyboardEventSource::KeyCode(keycode) = event.source {
            self.process_action(Action::Key(KeyCode::Hid(keycode)), event).await;
            return;
        }

        // Capture the event time once per event and thread it through.
        let event_time = Instant::now();

        // Process key
        let key_action = &self.keymap.get_action_with_layer_cache(event);

        if !self.combo_on || self.process_combo(key_action, event, event_time).await {
            self.process_key_action(key_action, event, event_time).await
        }
    }

    async fn process_key_action(&mut self, key_action: &KeyAction, event: KeyboardEvent, event_time: Instant) {
        // First, make the decision for current key and held keys
        let (decision_for_current_key, decisions) = self.make_decisions_for_keys(key_action, event);

        // Clean up early-fired keys that belong to a different position — their tap was already sent,
        // so they should not linger in the buffer when new events arrive.
        // Keys at the same position are kept to allow hold_after_tap on re-press.
        self.held_buffer
            .keys
            .retain(|k| !matches!(k.state, KeyState::EarlyFired(_)) || k.event.source == event.source);

        // Fire held keys if needed
        let (keyboard_state_updated, updated_decision_for_cur_key) =
            self.fire_held_keys(decision_for_current_key, decisions).await;

        // Process current key action after all held keys are resolved
        let key_action = if updated_decision_for_cur_key != KeyBehaviorDecision::Buffer
            && keyboard_state_updated
            && event.source.is_physical()
        {
            // The key_action needs to be updated due to the morse key might be triggered
            &self.keymap.get_action_with_layer_cache(event)
        } else {
            key_action
        };
        match updated_decision_for_cur_key {
            KeyBehaviorDecision::CleanBuffer | KeyBehaviorDecision::Release => {
                debug!("Clean buffer, then process current key normally");
                self.process_key_action_inner(key_action, event, event_time).await
            }
            KeyBehaviorDecision::Buffer => {
                debug!("Current key is buffered");
                if key_action.is_morse() {
                    // A morse key may already have an entry here, and the morse press path
                    // continues that pattern. Pushing would leave two entries for one key.
                    self.process_key_action_morse(key_action, event, event_time).await;
                } else {
                    self.held_buffer.push(HeldKey::new(
                        event,
                        *key_action,
                        KeyState::Pressed(MorsePattern::default()),
                        event_time,
                        event_time,
                    ));
                }
            }
            KeyBehaviorDecision::Ignore => {
                debug!("Current key is ignored or not buffered, process normally: {:?}", event);
                // Process current key normally
                self.process_key_action_inner(key_action, event, event_time).await
            }
            KeyBehaviorDecision::FlowTap => {
                if !key_action.is_morse() || !Self::is_flow_tap_enabled(self.keymap, key_action) {
                    self.process_key_action_inner(key_action, event, event_time).await;
                    return;
                }

                let action = Self::action_from_pattern(self.keymap, key_action, TAP); //tap action
                self.process_action(action, event).await;
                // Drop any existing held entry at this position (e.g. an EarlyFired entry left by
                // a previous quick tap) before inserting the new one. The held buffer assumes one
                // entry per position; without this the release handler would find the stale entry
                // first via find_source_mut, skip the release report, and leave the key stuck down.
                self.held_buffer.remove_if(|k| k.event.source == event.source);
                let now = Instant::now();
                let time_out = now + Self::morse_timeout(self.keymap, key_action, true);
                self.held_buffer.push(HeldKey::new(
                    event,
                    *key_action,
                    KeyState::FlowTapped(action),
                    now,
                    time_out,
                ));
            }
        }
    }

    /// Fire held keys according to their decisions.
    ///
    /// This function fires held keys according to their decisions, and returns
    /// whether the keyboard state is updated after firing those keys and
    /// the updated decision for current key.
    async fn fire_held_keys(
        &mut self,
        mut decision_for_current_key: KeyBehaviorDecision,
        decisions: Vec<(KeyboardEventSource, HeldKeyDecision), 16>,
    ) -> (bool, KeyBehaviorDecision) {
        let mut keyboard_state_updated = false;
        // Fire buffered keys
        for (source, decision) in decisions {
            // Some decisions of held keys have been made, fire those keys
            // debug!("✅ Decision for held key: {:?}: {:?}", source, decision)
            match decision {
                HeldKeyDecision::UnilateralTap | HeldKeyDecision::FlowTap => {
                    if let Some(mut held_key) = self.held_buffer.remove_if(|k| k.event.source == source)
                        && held_key.action.is_morse()
                    {
                        // Unilateral tap of the held key is triggered
                        debug!("Cleaning buffered morse key due to unilateral tap or flow tap");
                        match held_key.state {
                            KeyState::Pressed(_) | KeyState::Holding(_) => {
                                // In this state pattern is not surely finished,
                                // however an other key is pressed so terminate the sequence
                                // with a tap due to UnilateralTap decision; try to resolve as is
                                let pattern = match held_key.state {
                                    KeyState::Pressed(pattern) => pattern.followed_by_tap(), // The HeldKeyDecision turned this into tap!
                                    KeyState::Holding(pattern) => pattern,
                                    _ => unreachable!(),
                                };
                                debug!("Pattern after unilateral tap or flow tap: {:?}", pattern);
                                let action = Self::action_from_pattern(self.keymap, &held_key.action, pattern);
                                self.process_action(action, held_key.event).await;
                                held_key.state = KeyState::ProcessedButReleaseNotReportedYet(action);
                                // Push back after triggered tap
                                self.held_buffer.push(held_key);
                            }
                            KeyState::Released(pattern) => {
                                // In this state pattern is not surely finished,
                                // however an other key is pressed so terminate the sequence, try to resolve as is
                                debug!("Pattern after released, unilateral tap or flow tap: {:?}", pattern);
                                let action = Self::action_from_pattern(self.keymap, &held_key.action, pattern);
                                held_key.event.pressed = true;
                                self.process_key_action_tap(action, held_key.event).await;
                                // The tap is fully fired, don't push it back to buffer again
                                // Removing from the held buffer is like setting to an idle state
                            }
                            _ => (),
                        }
                    }
                }
                HeldKeyDecision::PermissiveHold | HeldKeyDecision::HoldOnOtherKeyPress => {
                    if let Some(mut held_key) = self.held_buffer.remove_if(|k| k.event.source == source) {
                        let action = if held_key.event.source.is_physical() {
                            self.keymap.get_action_with_layer_cache(held_key.event)
                        } else {
                            held_key.action
                        };

                        if action.is_morse() {
                            // Permissive hold of held key is triggered
                            debug!("Cleaning buffered morse key due to permissive hold or hold on other key press");
                            match held_key.state {
                                KeyState::Pressed(_) | KeyState::Holding(_) => {
                                    // In this state pattern is not surely finished,
                                    // however an other key is pressed so terminate the sequence
                                    // with a hold due to PermissiveHold/HoldOnOtherKeyPress decision; try to resolve as is
                                    let pattern = match held_key.state {
                                        KeyState::Pressed(pattern) => pattern.followed_by_hold(), // The HeldKeyDecision turned this into hold!
                                        KeyState::Holding(pattern) => pattern,
                                        _ => unreachable!(),
                                    };
                                    keyboard_state_updated = true;
                                    debug!("pattern after permissive hold: {:?}", pattern);
                                    let action = Self::action_from_pattern(self.keymap, &action, pattern);
                                    self.process_action(action, held_key.event).await;
                                    held_key.state = KeyState::ProcessedButReleaseNotReportedYet(action);
                                    // Push back after triggered hold
                                    self.held_buffer.push(held_key);
                                }
                                KeyState::Released(pattern) => {
                                    debug!("pattern after released, permissive hold: {:?}", pattern);
                                    let action = Self::action_from_pattern(self.keymap, &action, pattern);
                                    held_key.event.pressed = true;
                                    self.process_key_action_tap(action, held_key.event).await;
                                    // The tap is fully fired, don't push it back to buffer again
                                    // Removing from the held buffer is like setting to an idle state
                                }
                                _ => (),
                            }
                        }
                    }
                }
                HeldKeyDecision::Release => {
                    // Releasing the current key, will always be tapping, because timeout isn't here
                    let mut resolved = false;
                    if let Some(mut held_key) = self.held_buffer.remove_if(|k| k.event.source == source) {
                        // Always re-evaluate a physical key based on current layer state.
                        // A prior layer change (e.g. permissive hold activating a layer)
                        // may have changed what action this key maps to.
                        let key_action = if held_key.event.source.is_physical() {
                            self.keymap.get_action_with_layer_cache(held_key.event)
                        } else {
                            held_key.action
                        };
                        if key_action != held_key.action {
                            keyboard_state_updated = true;
                        }
                        debug!("Processing current key before releasing: {:?}", held_key.event);
                        if !key_action.is_morse() {
                            self.process_non_morse_key(key_action, held_key.event, held_key.press_time())
                                .await;
                            resolved = true;
                        } else {
                            match held_key.state {
                                KeyState::Pressed(_) | KeyState::Holding(_) => {
                                    debug!("Cleaning buffered Release key");

                                    let pattern = match held_key.state {
                                        KeyState::Pressed(pattern) => pattern.followed_by_tap(), // TODO? should we double check the timeout with Instant::now() >= held_key.timeout_time?
                                        KeyState::Holding(pattern) => pattern,
                                        _ => unreachable!(),
                                    };

                                    debug!("pattern by decided tap release: {:?}", pattern);

                                    let final_action =
                                        Self::try_predict_final_action(self.keymap, &key_action, pattern);
                                    let defer_for_quick_tap = matches!(held_key.state, KeyState::Pressed(_))
                                        && !pattern.is_empty()
                                        && pattern.is_all_taps()
                                        && Self::quick_tap_window(self.keymap, &key_action).is_some();
                                    if let Some(action) = final_action
                                        && !defer_for_quick_tap
                                    {
                                        debug!("tap prediction {:?} -> {:?}", pattern, action);
                                        self.process_action(action, held_key.event).await;
                                        held_key.state = KeyState::ProcessedButReleaseNotReportedYet(action);
                                        resolved = true;
                                    }
                                }
                                _ => {} // For morse, the releasing will not be processed immediately, so just ignore it
                            }
                            // Push back after triggered hold
                            self.held_buffer.push(held_key);
                        }
                    }

                    // Only clean the buffer (fire buffered normal keys) when the releasing key
                    // was actually resolved. If prediction failed for a morse key, it still needs
                    // time to resolve (via gap timeout), so normal keys must stay buffered to
                    // preserve correct key ordering.
                    if resolved {
                        decision_for_current_key = KeyBehaviorDecision::CleanBuffer;
                    }
                }
                HeldKeyDecision::Normal => {
                    // Check if the normal keys in the buffer should be triggered.
                    let trigger_normal = matches!(decision_for_current_key, KeyBehaviorDecision::CleanBuffer);

                    if trigger_normal && let Some(held_key) = self.held_buffer.remove_if(|k| k.event.source == source) {
                        debug!("Cleaning buffered normal key");
                        let action = if keyboard_state_updated && held_key.event.source.is_physical() {
                            self.keymap.get_action_with_layer_cache(held_key.event)
                        } else {
                            held_key.action
                        };

                        // Note: Morse like actions are not expected here.
                        assert!(!action.is_morse());
                        debug!("Tap Key {:?} now press down, action: {:?}", held_key.event, action);
                        self.process_key_action_inner(&action, held_key.event, held_key.press_time())
                            .await;
                    }
                }
                _ => (),
            }
        }
        (keyboard_state_updated, decision_for_current_key)
    }

    /// Make decisions for current key and each held key.
    ///
    /// This function iterates all held keys and makes decision for them if a special mode is triggered, such as permissive hold, etc.
    fn make_decisions_for_keys(
        &mut self,
        key_action: &KeyAction,
        event: KeyboardEvent,
    ) -> (
        KeyBehaviorDecision,
        Vec<(KeyboardEventSource, HeldKeyDecision), HOLD_BUFFER_SIZE>,
    ) {
        // Decision of current key and held keys
        let mut decision_for_current_key = KeyBehaviorDecision::Ignore;
        let mut decisions: Vec<(_, HeldKeyDecision), HOLD_BUFFER_SIZE> = Vec::new();

        // When pressing a morse key, check flow tap first.
        if event.pressed
            && key_action.is_morse()
            && Self::is_flow_tap_enabled(self.keymap, key_action)
            && self.last_press_time.elapsed() < self.keymap.morse_prior_idle_time()
        {
            // It's in key streak, trigger the first tap action
            debug!("Flow tap detected, trigger tap action for current morse key");

            decision_for_current_key = KeyBehaviorDecision::FlowTap;
        }

        // Whether the held buffer needs to be checked.
        let check_held_buffer = event.pressed
            || self
                .held_buffer
                .find_source(event.source)
                .is_some_and(|k| matches!(k.state, KeyState::Pressed(_) | KeyState::Released(_)));

        if check_held_buffer {
            // Check all unresolved held keys, calculate their decision one-by-one
            for held_key in self
                .held_buffer
                .keys
                .iter()
                .filter(|k| matches!(k.state, KeyState::Pressed(_) | KeyState::Released(_)))
            {
                // Releasing a key is already buffered
                if !event.pressed && held_key.event.source == event.source {
                    debug!("Releasing a held key: {:?}", event);
                    let _ = decisions.push((held_key.event.source, HeldKeyDecision::Release));
                    decision_for_current_key = KeyBehaviorDecision::Release;
                    continue;
                }

                // Buffered normal keys should be added to the decision list,
                // they will be processed later according to the decision of current key
                if !held_key.action.is_morse() && matches!(held_key.state, KeyState::Pressed(_)) {
                    let _ = decisions.push((held_key.event.source, HeldKeyDecision::Normal));
                    continue;
                }

                // The remaining keys are not same as the current key, check only morse keys
                if held_key.event.source != event.source && held_key.action.is_morse() {
                    let mode = Self::tap_hold_mode(self.keymap, &held_key.action);

                    if event.pressed {
                        // The current key is being pressed

                        if decision_for_current_key == KeyBehaviorDecision::FlowTap
                            && matches!(held_key.state, KeyState::Pressed(_))
                            && Self::is_flow_tap_enabled(self.keymap, &held_key.action)
                        {
                            debug!("Flow tap triggered, resolve buffered morse key as tapping");
                            // If flow tap of current key is triggered, tapping all held keys
                            let _ = decisions.push((held_key.event.source, HeldKeyDecision::FlowTap));
                            continue;
                        }

                        // Check morse key mode
                        match mode {
                            MorseMode::PermissiveHold => {
                                // Permissive hold mode checks key releases, so push current key press into buffer.
                                decision_for_current_key = KeyBehaviorDecision::Buffer;
                            }
                            MorseMode::HoldOnOtherPress => {
                                debug!(
                                    "Trigger morse key due to hold on other key press: {:?}",
                                    held_key.action
                                );
                                let _ = decisions.push((held_key.event.source, HeldKeyDecision::HoldOnOtherKeyPress));
                                decision_for_current_key = KeyBehaviorDecision::CleanBuffer;
                            }
                            MorseMode::Normal => {
                                // Normal mode: resolve a same-hand HRM as tap on press when
                                // unilateral_tap is enabled, so the roll fires in the correct
                                // order (HRM tap first, then the new key).
                                let unilateral_tap = Self::is_unilateral_tap_enabled(self.keymap, &held_key.action);
                                if unilateral_tap
                                    && matches!(held_key.state, KeyState::Pressed(_))
                                    && let KeyboardEventSource::Key(pos1) = held_key.event.source
                                    && let KeyboardEventSource::Key(pos2) = event.source
                                {
                                    let hand1 = self.keymap.hand_at(pos1.row as usize, pos1.col as usize);
                                    let hand2 = self.keymap.hand_at(pos2.row as usize, pos2.col as usize);
                                    if hand1.is_same_side(hand2) {
                                        debug!(
                                            "Unilateral tap on press (Normal mode): resolving HRM as tap for correct roll order"
                                        );
                                        let _ = decisions.push((held_key.event.source, HeldKeyDecision::UnilateralTap));
                                        decision_for_current_key = KeyBehaviorDecision::CleanBuffer;
                                        continue;
                                    }
                                }
                            }
                        }
                    } else {
                        let unilateral_tap = Self::is_unilateral_tap_enabled(self.keymap, &held_key.action);

                        // 1. Check unilateral tap of held key
                        // Note: `decision for current key == Release` means that current held key is pressed AFTER the current releasing key,
                        // releasing a key should not trigger unilateral tap of keys which are pressed AFTER the released key
                        if unilateral_tap
                            && event.source != held_key.event.source
                            && decision_for_current_key != KeyBehaviorDecision::Release
                            && let KeyboardEventSource::Key(pos1) = held_key.event.source
                            && let KeyboardEventSource::Key(pos2) = event.source
                        {
                            let hand1 = self.keymap.hand_at(pos1.row as usize, pos1.col as usize);
                            let hand2 = self.keymap.hand_at(pos2.row as usize, pos2.col as usize);

                            if hand1.is_same_side(hand2) {
                                debug!("Unilateral tap triggered, resolve morse key as tapping");
                                let _ = decisions.push((held_key.event.source, HeldKeyDecision::UnilateralTap));
                                continue;
                            }
                        }

                        // The current key is being released, check only the held key in permissive hold mode
                        if decision_for_current_key != KeyBehaviorDecision::Release && mode == MorseMode::PermissiveHold
                        {
                            debug!("Permissive hold!");
                            // Check first current releasing key is in the buffer, AND after the current key
                            let _ = decisions.push((held_key.event.source, HeldKeyDecision::PermissiveHold));
                            decision_for_current_key = KeyBehaviorDecision::CleanBuffer;
                        }
                    }
                }
            }
        }
        (decision_for_current_key, decisions)
    }

    async fn process_key_action_inner(
        &mut self,
        original_key_action: &KeyAction,
        event: KeyboardEvent,
        event_time: Instant,
    ) {
        // Start forks
        let (key_action, dropped_modifiers) = self.try_start_forks(original_key_action, event);

        // Send the modifier release in its own report. macOS may apply the key first if both share a report.
        if dropped_modifiers {
            self.send_keyboard_report_with_resolved_modifiers(true).await;
        }

        #[cfg(feature = "_ble")]
        LAST_KEY_TIMESTAMP.signal(Instant::now().as_secs() as u32);

        if key_action.is_morse() {
            self.process_key_action_morse(&key_action, event, event_time).await;
        } else {
            self.process_non_morse_key(key_action, event, event_time).await;
        }
        // Restore the modifiers the fork suppressed
        if self.try_finish_forks(event) {
            self.send_keyboard_report_with_resolved_modifiers(false).await;
        }
    }

    async fn process_non_morse_key(&mut self, action: KeyAction, event: KeyboardEvent, at: Instant) {
        match action {
            KeyAction::Single(action) => self.process_action(action, event).await,
            KeyAction::Tap(action) => self.process_key_action_tap(action, event).await,
            KeyAction::Sticky(action, profile) => self.process_sticky_key(action, profile, event, at).await,
            _ => (),
        }
    }

    /// Replaces the incoming key_action if a fork is configured for that key.
    /// The replacement decision is made at key_press time, and the decision
    /// is kept until the key is released.
    /// Also returns whether the fork suppresses held modifiers.
    fn try_start_forks(&mut self, key_action: &KeyAction, event: KeyboardEvent) -> (KeyAction, bool) {
        if self.keymap.forks_is_empty() {
            return (*key_action, false);
        }

        if !event.pressed {
            // Release the replacement; the fork is deactivated after the release report is sent
            if let Some(active) = self
                .fork_states
                .iter()
                .flatten()
                .find(|active| active.source == event.source)
            {
                debug!("replace input with fork action {:?}", active);
                return (active.replacement, false);
            }
            return (*key_action, false);
        }

        let mut decision_state = StateBits {
            modifiers: self.held_modifiers(),
            leds: LedIndicator::from_bits(LOCK_LED_STATES.load(core::sync::atomic::Ordering::Relaxed)),
            mouse: MouseButtons::from_bits(self.mouse.report.buttons),
        };
        let layer_bit = 1u32.checked_shl(self.keymap.pressed_layer(event.source).into());

        let fork_states = &self.fork_states;
        let fork_keep_mask = &mut self.fork_keep_mask;
        let (replacement, chain_starter, combined_suppress) = self.keymap.with_forks(|forks| {
            let mut triggered_forks = [false; FORK_MAX_NUM]; // used to avoid loops
            let mut chain_starter: Option<usize> = None;
            let mut combined_suppress = ModifierCombination::default();
            let mut replacement = *key_action;

            'bind: loop {
                for (i, fork) in forks.iter().enumerate() {
                    if !triggered_forks[i]
                        && fork_states[i].is_none()
                        && fork.trigger == replacement
                        && (fork.layers == u32::MAX || layer_bit.is_some_and(|bit| fork.layers & bit != 0))
                    {
                        let decision = (fork.match_any & decision_state) != StateBits::default()
                            && (fork.match_none & decision_state) == StateBits::default();

                        if !decision && fork.negative_output == KeyAction::Transparent {
                            continue;
                        }

                        replacement = if decision {
                            fork.positive_output
                        } else {
                            fork.negative_output
                        };

                        let suppress = fork.match_any.modifiers & !fork.kept_modifiers;

                        combined_suppress |= suppress;

                        // Reduce the previously aggregated keeps with the match_any mask
                        // (since this is the expected behavior in most cases)
                        *fork_keep_mask &= !fork.match_any.modifiers;

                        // Then add the user defined keeps (if any)
                        *fork_keep_mask |= fork.kept_modifiers;

                        if chain_starter.is_none() {
                            chain_starter = Some(i);
                        }

                        if fork.bindable {
                            // If this fork is bindable look for other not yet activated forks,
                            // which can be triggered by that the current replacement key
                            triggered_forks[i] = true; // Avoid triggering the same fork again -> no infinite loops either

                            // For the next fork evaluations, update the decision state
                            // with the suppressed modifiers
                            decision_state.modifiers &= !suppress;
                            continue 'bind;
                        }

                        //return final decision is ready
                        break 'bind;
                    }
                }

                // No (more) forks were triggered, so we are done
                break 'bind;
            }

            (replacement, chain_starter, combined_suppress)
        });

        if let Some(initial) = chain_starter {
            // After the initial fork triggered, we have switched to "bind mode".
            // The later triggered forks will not really activate, only update
            // the replacement decision and modifier suppressions of the initially
            // triggered fork, which is here marked as active:
            self.fork_states[initial] = Some(ActiveFork {
                replacement,
                suppress: combined_suppress,
                source: event.source,
            });
        }

        // Modifiers the replacement adds back are not dropped
        let readded = match replacement {
            KeyAction::Single(Action::KeyWithModifier(_, modifiers)) => modifiers,
            _ => ModifierCombination::default(),
        };
        let held = self.held_modifiers();
        let dropped = held & combined_suppress & !self.fork_keep_mask & !readded;

        // No (or no more) forks were triggered, so we are done
        (replacement, dropped.into_bits() != 0)
    }

    // Release of forked key must deactivate the fork
    // (explicit modifier suppressing effect will be stopped only AFTER the release hid report is sent)
    // Returns whether the resolved modifiers changed
    fn try_finish_forks(&mut self, event: KeyboardEvent) -> bool {
        let modifiers_before = self.resolve_modifiers(false);
        if !event.pressed {
            for fork_state in self.fork_states.iter_mut() {
                if fork_state.is_some_and(|active| active.source == event.source) {
                    *fork_state = None;
                }
            }
        }
        self.resolve_modifiers(false) != modifiers_before
    }

    /// Trigger a combo that is delayed(if exists).
    ///
    /// A combo is delayed when it's **triggered** but it's a "subset" of another combo, like combo "asd" and combo "asdf".
    /// When "asd" is pressed, combo "asd" is delayed due to there's "asdf" combo. The delayed combo will be triggered when:
    /// - Timeout
    /// - Any of the key in the delayed combo is released
    /// - Current delayed combos are interrupted
    ///
    /// When multiple combos are delayed, this function will only trigger the longest one, for example,
    /// combo "as", "sd" and "asd" are delayed, this function will only trigger "asd", and clear the combo state of "as"/"sd"
    ///
    /// If the full combo("asdf") is triggered, the delayed combo will be cleared without triggering it.
    ///
    /// Parameters:
    /// - `key_action`: The action of the key that triggered this function
    /// - `event`: The keyboard event. When pressing (interrupting), trigger any delayed combo.
    ///   When releasing, only trigger combos that contain the key_action.
    async fn trigger_delayed_combo(&mut self, key_action: &KeyAction, event: KeyboardEvent) {
        let delayed = self.keymap.with_combos(|combos| {
            combos
                .iter()
                .enumerate()
                .filter_map(|(i, c)| {
                    let c = c.as_ref()?;
                    (c.is_pending() && (event.pressed || c.config.contains(key_action))).then_some((c.size(), i))
                })
                .max_by_key(|&(size, _)| size)
                .map(|(_, i)| i)
        });
        if let Some(idx) = delayed {
            self.fire_combo(idx, Instant::now()).await;
        }
    }

    /// Fire the combo `idx`:
    /// - drop the chord keys from the held buffer
    /// - reset the combos it shadows
    /// - dispatch its output
    async fn fire_combo(&mut self, idx: usize, at: Instant) {
        let Some((output, chord)) = self
            .keymap
            .with_combos_mut(|combos| combos[idx].as_mut().map(|c| (c.trigger(), c.config.actions.clone())))
        else {
            return;
        };
        debug!("[Combo] {:?} triggered", output);
        self.held_buffer
            .keys
            .retain(|item| item.state != KeyState::WaitingCombo || !chord.contains(&item.action));
        self.reset_shadowed_combos(&chord);
        self.process_key_action(&output, KeyboardEvent::combo(idx as u8, true), at)
            .await;
    }

    // Reset combos shadowed by a just-triggered combo: any *other* combo that is
    // fully pressed but not yet triggered and shares at least one key with the
    // triggered combo.
    fn reset_shadowed_combos(&mut self, triggered_actions: &[KeyAction]) {
        self.keymap.with_combos_mut(|combos| {
            combos.iter_mut().filter_map(|c| c.as_mut()).for_each(|c| {
                if c.is_pending() && c.config.actions.iter().any(|a| triggered_actions.contains(a)) {
                    info!("Resetting shadowed combo: {:?}", c,);
                    c.reset();
                }
            });
        });
    }

    /// Handle combos before the key is processed, returns `true` if the key
    /// should still be processed as a normal key.
    async fn process_combo(&mut self, key_action: &KeyAction, event: KeyboardEvent, event_time: Instant) -> bool {
        let current_layer = self.keymap.get_activated_layer();

        // First, when releasing a key, check whether there's untriggered combo, if so, triggerer it first
        if !event.pressed {
            self.trigger_delayed_combo(key_action, event).await;
        }

        // If this is a re-press of a key belonging to an already-triggered combo
        // (the user released one chord key and pressed it again while the other
        // is still down), reassert its bit in the combo state and swallow the
        // press: its release is consumed by the combo below, so dispatching the
        // press would leave the key stuck on the host.
        if event.pressed {
            let reasserted = self.keymap.with_combos_mut(|combos| {
                let mut any = false;
                for combo in combos.iter_mut().filter_map(|c| c.as_mut()) {
                    if combo.reassert_if_triggered(key_action) {
                        any = true;
                    }
                }
                any
            });
            if reasserted {
                debug!("[Combo] re-press of triggered-combo key swallowed: {:?}", key_action);
                return false;
            }
        }
        // Combo idle cooldown: skip combo recording if within idle window
        // Equivalent to ZMK's require-prior-idle-ms. Key still dispatches normally.
        let skip_combo = event.pressed
            && self
                .keymap
                .combo_prior_idle_time()
                .is_some_and(|idle_time| self.last_press_time.elapsed() < idle_time);

        let max_size_of_updated_combo = if skip_combo {
            None
        } else {
            self.keymap.with_combos_mut(|combos| {
                combos
                    .iter_mut()
                    .filter_map(|c| c.as_mut())
                    .map(|c| {
                        if c.update(key_action, event, current_layer) {
                            info!("Updated combo: {:?}", c);
                            c.size()
                        } else {
                            0
                        }
                    })
                    .max()
            })
        };

        if event.pressed
            && let Some(max_size) = max_size_of_updated_combo
            && max_size > 0
        {
            // If the max_size > 0, there's at least one combo is updated
            self.held_buffer.push(HeldKey::new(
                event,
                *key_action,
                KeyState::WaitingCombo,
                event_time,
                event_time + self.keymap.combo_timeout(),
            ));

            // Only one combo is updated, and triggered
            let triggered = self.keymap.with_combos(|combos| {
                combos
                    .iter()
                    .position(|c| c.as_ref().is_some_and(|c| c.is_pending() && c.size() == max_size))
            });
            if let Some(idx) = triggered {
                self.fire_combo(idx, event_time).await;
            }
            false
        } else {
            // No combo is updated, dispatch combos
            if !event.pressed {
                info!("Releasing keys in combo: {:?} {:?}", event, key_action);

                // Overlapping triggered combos can each fully release on the same key
                // (e.g. `M+,` and `,+.` both sharing Comma), so collect every combo
                // output that unwinds — not just the first — otherwise the others
                // stay stuck on the host.
                let mut combo_outputs: Vec<(u8, KeyAction), COMBO_MAX_NUM> = Vec::new();
                let mut releasing_triggered_combo = false;

                self.keymap.with_combos_mut(|combos| {
                    for (i, combo) in combos.iter_mut().enumerate() {
                        let Some(combo) = combo else { continue };
                        if combo.config.contains(key_action) {
                            // Releasing a combo key in triggered combo
                            releasing_triggered_combo |= combo.is_triggered();
                            info!("[Combo] releasing: {:?}", combo);

                            // Release the combo key, check whether the combo is fully released
                            if combo.update_released(key_action) {
                                debug!("[Combo] {:?} is released", combo.config.output);
                                let _ = combo_outputs.push((i as u8, combo.config.output));
                            }
                        }
                    }
                });

                // Releasing a triggered combo: release every combo output whose combo
                // fully unwound, in iteration order. A partial release (combo output
                // still held) consumes the event without sending anything.
                if releasing_triggered_combo {
                    for (idx, output) in &combo_outputs {
                        self.process_key_action(output, KeyboardEvent::combo(*idx, false), event_time)
                            .await;
                    }
                    return false;
                }
            }

            // When no key is updated(the combo is interruptted), or a key is released,
            self.dispatch_combos(key_action, event).await;
            true
        }
    }

    // Dispatch combo keys buffered in the held buffer when the combo isn't being triggered.
    async fn dispatch_combos(&mut self, key_action: &KeyAction, event: KeyboardEvent) {
        self.trigger_delayed_combo(key_action, event).await;

        // Removing and reinserting keys preserves time order.
        while let Some(key) = self.held_buffer.remove_if(|key| key.state == KeyState::WaitingCombo) {
            debug!("[Combo] Dispatching combo: {:?}", key);
            self.process_key_action(&key.action, key.event, key.press_time()).await;
        }

        // Keep triggered combos intact so their releases can still be matched.
        self.keymap.with_combos_mut(|combos| {
            combos
                .iter_mut()
                .filter_map(|combo| combo.as_mut())
                .filter(|combo| !combo.is_triggered())
                .for_each(Combo::reset);
        });
    }

    async fn process_action(&mut self, action: Action, event: KeyboardEvent) {
        // Resolve Repeat/GraveEscape before checking consumption.
        let sticky_release = !event.pressed && matches!(event.source, KeyboardEventSource::Sticky(_));
        let input = ActionEvent {
            action: self.resolve_action(action),
            keyboard_event: event,
        };
        #[cfg(feature = "passkey_entry")]
        let observe_input = sticky_release
            || !(self.passkey_entry_state.is_active()
                && matches!(input.action,
                    Action::Key(KeyCode::Hid(key)) | Action::KeyWithModifier(key, _) if key.is_keyboard_key()));
        #[cfg(not(feature = "passkey_entry"))]
        let observe_input = true;
        if observe_input {
            self.before_sticky_input(input);
        }
        self.release_stickies(None).await;
        if self.execute_action(input).await {
            let modifiers = self.resolve_modifiers(event.pressed);
            self.send_keyboard_report(modifiers, sticky_release).await;
        }
        if observe_input {
            self.after_sticky_input(input);
        }
        self.release_stickies(None).await;
    }

    fn resolve_action(&self, action: Action) -> Action {
        match action {
            Action::Key(KeyCode::Hid(HidKeyCode::Again)) | Action::Special(SpecialKey::Repeat) => {
                Action::Key(KeyCode::Hid(self.last_key_code))
            }
            Action::KeyWithModifier(HidKeyCode::Again, mods) => Action::KeyWithModifier(self.last_key_code, mods),
            Action::Special(SpecialKey::GraveEscape) => {
                Action::Key(KeyCode::Hid(if self.held_modifiers().into_bits() == 0 {
                    HidKeyCode::Escape
                } else {
                    HidKeyCode::Grave
                }))
            }
            _ => action,
        }
    }

    /// Return whether a keyboard report needs sending.
    async fn execute_action(&mut self, input: ActionEvent) -> bool {
        publish_event_async(input).await;
        let (action, event) = (input.action, input.keyboard_event);
        match action {
            Action::Key(key) => return self.process_action_key(key, ModifierCombination::new(), event).await,
            Action::LayerOn(layer_num) => self.keymap.hold_layer(layer_num, event),
            Action::LayerOff(layer_num) => {
                // Turn off a layer temporarily when the key is pressed
                // Reactivate the layer after the key is released
                if event.pressed {
                    self.keymap.deactivate_layer(layer_num);
                }
            }
            Action::LayerToggle(layer_num) => {
                // Toggle a layer when the key is release
                if !event.pressed {
                    self.keymap.toggle_layer(layer_num);
                }
            }
            Action::LayerToggleOnly(layer_num) => {
                // Activate a layer and deactivate all other layers(except default layer)
                if event.pressed {
                    // Disable all layers except the default layer
                    let default_layer = self.keymap.get_default_layer();
                    let (_, _, num_layer) = self.keymap.get_keymap_config();
                    for i in 0..num_layer as u8 {
                        if i != default_layer {
                            self.keymap.deactivate_layer(i);
                        }
                    }
                    // Activate the target layer
                    self.keymap.activate_layer(layer_num);
                }
            }
            Action::DefaultLayer(layer_num) => {
                // Set the default layer
                self.keymap.set_default_layer(layer_num);
            }
            Action::PersistentDefaultLayer(layer_num) => {
                // Set the default layer and persist it so it survives a reboot
                self.keymap.set_default_layer(layer_num);
                // Persist only if the layer was valid (set_default_layer rejects out-of-range)
                #[cfg(feature = "storage")]
                if event.pressed && self.keymap.get_default_layer() == layer_num {
                    crate::storage::store_unchecked(crate::storage::StorageItem::DefaultLayer(layer_num)).await;
                }
            }
            Action::Modifier(modifiers) | Action::LayerOnWithModifier(_, modifiers) => {
                self.update_registered_key(KeyCode::Hid(HidKeyCode::No), modifiers, event);
                if let Action::LayerOnWithModifier(layer, _) = action {
                    self.keymap.hold_layer(layer, event);
                }
                return true;
            }
            Action::TriggerMacro(idx) => {
                if !self.keymap.macros(|m| m.queue(idx, event.pressed)) {
                    warn!("Macro queue full, dropped macro {}", idx);
                }
            }
            Action::KeyWithModifier(key_code, modifiers) => {
                return self.process_action_key(KeyCode::Hid(key_code), modifiers, event).await;
            }
            Action::Light(_light_action) => warn!("Light control is not supported"),
            Action::KeyboardControl(c) => self.process_action_keyboard_control(c, event).await,
            Action::User(id) => self.process_user(id, event).await,
            Action::TriLayerLower => {
                // Tri-layer lower, turn layer 1 on and update layer state
                self.keymap.hold_layer(1, event);
                self.keymap.update_fn_layer_state();
            }
            Action::TriLayerUpper => {
                // Tri-layer upper, turn layer 2 on and update layer state
                self.keymap.hold_layer(2, event);
                self.keymap.update_fn_layer_state();
            }
            #[cfg(feature = "steno")]
            Action::Steno(key) => {
                if let Some(report) = self.steno.on_event(key, event.pressed) {
                    crate::channel::try_send_hid_report(report);
                }
            }
            Action::No => {}
            _ => warn!("Action variant not supported: {:?}", action),
        }
        false
    }

    /// Tap action, send a key when the key is pressed, then release the key.
    async fn process_key_action_tap(&mut self, action: Action, mut event: KeyboardEvent) {
        debug!("TAP action: {:?}, {:?}", action, event);

        if event.pressed {
            self.process_action(action, event).await;

            // Wait 10ms, then send release
            Timer::after_millis(10).await;

            event.pressed = false;
            self.process_action(action, event).await;
        }
    }

    pub fn print_buffer(&self) {
        self.held_buffer
            .keys
            .iter()
            .enumerate()
            .for_each(|(i, k)| info!("\n✅Held buffer {}: {:?}, state: {:?}", i, k.event, k.state));
    }

    /// Calculates the combined effect of all modifiers:
    /// - registered (held) modifier keys, sticky ones included
    /// - `KeyWithModifier` modifiers, until the next press
    /// - possible fork related modifier suppressions
    pub fn resolve_modifiers(&self, pressed: bool) -> ModifierCombination {
        let mut result = self.held_modifiers();

        // The triggered forks suppress the 'match_any' modifiers automatically
        // unless they are configured as the 'kept_modifiers'
        let mut fork_suppress = ModifierCombination::default();
        for fork_state in self.fork_states.iter().flatten() {
            fork_suppress |= fork_state.suppress;
        }

        // Some of these suppressions could have been canceled after the fork activation
        // by "explicit" modifier key presses - fork_keep_mask collects these:
        fork_suppress &= !self.fork_keep_mask;

        // Execute the remaining suppressions
        result &= !fork_suppress;

        // Apply the modifiers from registered [`Action::KeyWithModifiers`],
        // the suppression effect of forks should not apply on these
        for k in self.registered.iter().filter(|k| !k.holds_modifiers()) {
            result |= k.mods;
        }

        // Apply Caps Word shift
        if self.caps_word.is_active() && pressed && self.caps_word.is_shift_current() {
            result |= ModifierCombination::new().with_left_shift(true);
        }

        result
    }

    /// Process a basic key event and return whether to send a keyboard report.
    fn process_hid_keycode(&mut self, key: HidKeyCode, mods: ModifierCombination, event: KeyboardEvent) -> bool {
        #[cfg(feature = "passkey_entry")]
        if self.passkey_entry_state.is_active() {
            use crate::ble::passkey::{PASSKEY_RESPONSE, PasskeyAction};

            // In passkey mode: capture on release only (prevents Enter release leaking)
            if !event.pressed {
                self.update_registered_key(KeyCode::Hid(key), mods, event);
                match self.passkey_entry_state.handle_key(key) {
                    PasskeyAction::Submitted(passkey) => {
                        info!("[passkey] Submitting passkey");
                        PASSKEY_RESPONSE.signal(Some(passkey));
                    }
                    PasskeyAction::Cancelled => {
                        info!("[passkey] Cancelled");
                        PASSKEY_RESPONSE.signal(None);
                    }
                    _ => {
                        // Ignore other states
                    }
                }
            }
            // Captured passkey input does not produce host keyboard reports.
            return false;
        }

        self.update_registered_key(KeyCode::Hid(key), mods, event);
        true
    }

    async fn process_action_keyboard_control(&mut self, keyboard_control: KeyboardAction, event: KeyboardEvent) {
        match keyboard_control {
            KeyboardAction::CapsWordToggle => {
                // Handle Caps Word
                if event.pressed {
                    self.caps_word.toggle();
                };
            }
            KeyboardAction::ComboOn => self.combo_on = true,
            KeyboardAction::ComboOff => self.combo_on = false,
            KeyboardAction::ComboToggle => {
                if event.pressed {
                    self.combo_on = !self.combo_on;
                }
            }
            KeyboardAction::Bootloader => {
                // When releasing the key, process the boot action
                if !event.pressed {
                    boot::jump_to_bootloader();
                }
            }
            KeyboardAction::Reboot => {
                // When releasing the key, process the boot action
                if !event.pressed {
                    boot::reboot_keyboard();
                }
            }
            #[cfg(feature = "storage")]
            KeyboardAction::ClearEeprom => {
                // When releasing the key, reset the storage — the same operation
                // `ViaCommand::EepromReset` performs from the host side
                if !event.pressed {
                    crate::storage::reset().await;
                }
            }

            _ => warn!("KeyboardAction: {:?} is not supported yet", keyboard_control),
        }
    }

    /// Register each output's source before building its HID report.
    async fn process_action_key(&mut self, key: KeyCode, mods: ModifierCombination, event: KeyboardEvent) -> bool {
        if let KeyCode::Hid(hid) = key {
            if event.pressed {
                if hid.is_simple_key() {
                    self.last_press_time = Instant::now();
                }
                self.last_key_code = hid;
                self.caps_word.check(hid);
            }
            if key.is_basic_keyboard_key() {
                return self.process_hid_keycode(hid, mods, event);
            }
        }

        let Some((key, _)) = action_key(Action::Key(key)) else {
            return false;
        };
        self.update_registered_key(key, mods, event);
        // Non-keyboard WM modifiers surround the output's own report.
        if mods.into_bits() != 0 && event.pressed {
            self.send_keyboard_report_with_resolved_modifiers(true).await;
        }
        let report = match key {
            KeyCode::Consumer(_) => Some(Report::MediaKeyboardReport(MediaKeyboardReport {
                usage_id: self
                    .registered
                    .iter()
                    .rev()
                    .find_map(|k| match k.keycode {
                        KeyCode::Consumer(key) => Some(key.into()),
                        _ => None,
                    })
                    .unwrap_or(0),
            })),
            KeyCode::SystemControl(_) => Some(Report::SystemControlReport(SystemControlReport {
                usage_id: self
                    .registered
                    .iter()
                    .rev()
                    .find_map(|k| match k.keycode {
                        KeyCode::SystemControl(key) => Some(key as u8),
                        _ => None,
                    })
                    .unwrap_or(0),
            })),
            KeyCode::Hid(hid) if hid.is_mouse_key() => {
                let held = self.registered.iter().any(|k| k.keycode == key);
                let action = self.mouse.process(hid, held, &self.keymap.mouse_key_config());
                self.keymap.set_mouse_buttons(self.mouse.report.buttons);
                matches!(action, MouseAction::SendReport).then(|| Report::MouseReport(self.mouse.get_report()))
            }
            _ => None,
        };
        if let Some(report) = report {
            self.send_report(report).await;
            yield_now().await;
        }
        if mods.into_bits() != 0 && !event.pressed {
            self.send_keyboard_report_with_resolved_modifiers(false).await;
        }
        false
    }

    /// Fire pending mouse repeats: recalculate movement with acceleration,
    /// send the report, and schedule the next repeat.
    async fn fire_mouse_repeat(&mut self) {
        let report = {
            let config = self.keymap.mouse_key_config();
            self.mouse.fire_repeats(&config)
        };

        if let Some(report) = report {
            self.keymap.set_mouse_buttons(self.mouse.report.buttons);
            self.send_report(Report::MouseReport(report)).await;
            yield_now().await;
        }
    }

    async fn process_user(&mut self, id: u8, event: KeyboardEvent) {
        debug!("Processing user key id: {:?}, event: {:?}", id, event);

        #[cfg(feature = "_ble")]
        {
            use crate::NUM_BLE_PROFILE;
            use crate::ble::profile::BleProfileAction;
            use crate::channel::BLE_PROFILE_CHANNEL;
            if event.pressed {
                // Start the 5s hold gesture for any user key. `fire_user_hold` decides
                // which ids actually do something, so the id list lives in one place.
                self.user_hold = Some((Instant::now() + Duration::from_secs(5), id));
            } else {
                // A tap sends press and release back to back, so cancel what the press started.
                self.user_hold = None;
                // Other user keys are processed when released.
                if id < NUM_BLE_PROFILE as u8 {
                    info!("Switch to profile: {}", id);
                    BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(id)).await;
                } else if id == NUM_BLE_PROFILE as u8 {
                    // Next profile
                    BLE_PROFILE_CHANNEL.send(BleProfileAction::Next).await;
                } else if id == NUM_BLE_PROFILE as u8 + 1 {
                    // Previous profile
                    BLE_PROFILE_CHANNEL.send(BleProfileAction::Previous).await;
                } else if id == NUM_BLE_PROFILE as u8 + 2 {
                    // Clear bond on current profile
                    BLE_PROFILE_CHANNEL.send(BleProfileAction::ClearBond).await;
                } else if id == NUM_BLE_PROFILE as u8 + 3 {
                    // Toggle preferred transport (USB <-> BLE);
                    // only meaningful when both transports exist in this build.
                    #[cfg(not(feature = "_no_usb"))]
                    crate::state::toggle_preferred().await;
                }
                // Switch to the dongle slot.
                #[cfg(feature = "dongle")]
                if id == NUM_BLE_PROFILE as u8 + 5 {
                    info!("Switch to dongle profile");
                    BLE_PROFILE_CHANNEL
                        .send(BleProfileAction::Switch(crate::ble::profile::DONGLE_PROFILE))
                        .await;
                }
            }
        }
    }

    /// Run the gesture of a User key held for the full 5s; ids without one do nothing.
    /// Getting here means no key event arrived meanwhile, because any event cancels
    /// the hold.
    #[cfg(feature = "_ble")]
    async fn fire_user_hold(&mut self) {
        use crate::NUM_BLE_PROFILE;
        use crate::ble::profile::BleProfileAction;
        use crate::channel::BLE_PROFILE_CHANNEL;

        let Some((_, id)) = self.user_hold.take_if(|(at, _)| *at <= Instant::now()) else {
            return;
        };

        // Tapping a bond slot switches to it; holding it forgets the bond, switches, then re-pairs.
        if id < NUM_BLE_PROFILE as u8 {
            info!("Profile key held: clearing bond on profile {}", id);
            BLE_PROFILE_CHANNEL.send(BleProfileAction::ClearSlot(id)).await;
            BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(id)).await;
        }
        #[cfg(feature = "split")]
        if id == NUM_BLE_PROFILE as u8 + 4 {
            info!("Clear peer");
            publish_event(ClearPeerEvent);
        }
        #[cfg(feature = "dongle")]
        if id == NUM_BLE_PROFILE as u8 + 5 {
            use crate::ble::profile::DONGLE_PROFILE;
            info!("Dongle key held: clearing dongle bond, seeking a dongle");
            BLE_PROFILE_CHANNEL
                .send(BleProfileAction::ClearSlot(DONGLE_PROFILE))
                .await;
            BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(DONGLE_PROFILE)).await;
        }
    }

    /// Run the macro op that is due. One op per call, so `run()` handles a queued
    /// key event before the next op: the macro and the user's keys interleave.
    async fn fire_macro(&mut self) {
        if self.macro_due > Instant::now() {
            return;
        }
        let Some(op) = self.keymap.macros(|m| m.next_op()) else {
            return;
        };
        // Every op registers under the macro identity, never the trigger key's.
        let press = KeyboardEvent {
            source: KeyboardEventSource::Macro,
            pressed: true,
        };
        let release = KeyboardEvent {
            pressed: false,
            ..press
        };
        match op {
            // Held for the same 10ms as a tap-hold's tap.
            MacroOp::Tap(action) => self.process_key_action_tap(action, press).await,
            MacroOp::Press(action) => self.process_action(action, press).await,
            MacroOp::Release(action) => self.process_action(action, release).await,
            // Two reports whose modifiers are the character's own shift and nothing
            // else, so held modifiers never change what a text macro types.
            MacroOp::Char(c) => {
                let (key, shift) = from_ascii(c);
                let modifiers = ModifierCombination::new().with_left_shift(shift);
                for event in [press, release] {
                    let input = ActionEvent {
                        action: Action::KeyWithModifier(key, modifiers),
                        keyboard_event: event,
                    };
                    if event.pressed {
                        self.before_sticky_input(input);
                        self.release_stickies(None).await;
                    }
                    self.update_registered_key(KeyCode::Hid(key), ModifierCombination::new(), event);
                    self.send_keyboard_report(modifiers, false).await;
                    self.after_sticky_input(input);
                    self.release_stickies(Some(modifiers)).await;
                    if event.pressed {
                        Timer::after_millis(10).await;
                    }
                }
                // The text ends here
                if !matches!(self.keymap.macros(|m| m.peek_op()), Some(MacroOp::Char(_))) {
                    Timer::after_millis(10).await;
                    self.send_keyboard_report_with_resolved_modifiers(false).await;
                }
                if shift {
                    self.macro_due = Instant::now() + Duration::from_millis(10);
                }
            }
            MacroOp::Delay(ms) => self.macro_due = Instant::now() + Duration::from_millis(ms as u64),
            // The halves split at the first one; a later one does nothing.
            MacroOp::PauseForRelease => {}
        }
    }

    /// Send the keyboard report with resolved modifiers to the host.
    pub(crate) async fn send_keyboard_report_with_resolved_modifiers(&mut self, pressed: bool) {
        let modifiers = self.resolve_modifiers(pressed);
        self.send_keyboard_report(modifiers, false).await;
    }

    /// Send held keycodes with exactly `modifiers`.
    /// Sticky releases suppress unchanged output; ordinary input reports are always sent.
    async fn send_keyboard_report(&mut self, modifiers: ModifierCombination, only_if_changed: bool) {
        let keycodes = self.held_keycodes().map(|key| key as u8);
        info!(
            "Sending keyboard report, modifiers: {:?}, keycodes: {:?}",
            modifiers, keycodes
        );
        let report = KeyboardReport {
            modifier: modifiers.into_bits(),
            reserved: 0,
            leds: LOCK_LED_STATES.load(core::sync::atomic::Ordering::Relaxed),
            keycodes,
        };
        if !only_if_changed || self.last_keyboard_report != (report.modifier, report.keycodes) {
            self.send_report(Report::KeyboardReport(report)).await;
            yield_now().await;
        }
    }

    fn update_registered_key(&mut self, key: KeyCode, mods: ModifierCombination, event: KeyboardEvent) {
        let Some((keycode, own_mods)) = action_key(Action::Key(key)) else {
            return;
        };
        let mods = mods | own_mods;
        if event.pressed && keycode == KeyCode::Hid(HidKeyCode::No) && mods.into_bits() == 0 {
            return;
        }
        let before = self.held_modifiers();
        let key = RegisteredKey {
            source: event.source,
            keycode,
            mods,
        };
        // Removing first also makes a repeated press replace its earlier record.
        self.registered.retain(|k| !k.matches(&key));
        if event.pressed {
            let keys = self.held_keycodes();
            let new_usage = key.keyboard_key().is_some_and(|key| !keys.contains(&key));
            if (new_usage && !keys.contains(&HidKeyCode::No)) || self.registered.is_full() {
                warn!("Keyboard report full, dropped {:?}", key.keycode);
            } else {
                // `KeyWithModifier` modifiers apply only until the next press.
                if key.keycode.is_basic_keyboard_key() || mods.into_bits() != 0 {
                    for k in self.registered.iter_mut().filter(|k| !k.holds_modifiers()) {
                        k.mods = ModifierCombination::new();
                    }
                }
                let _ = self.registered.push(key);
                if key.holds_modifiers() {
                    // An explicit modifier press cancels the corresponding fork suppression.
                    self.fork_keep_mask |= key.mods;
                }
            }
        }
        let held = self.held_modifiers();
        if held != before {
            publish_event(ModifierEvent { modifier: held });
        }
    }

    /// Modifiers held by modifier actions.
    fn held_modifiers(&self) -> ModifierCombination {
        self.registered
            .iter()
            .filter(|k| k.holds_modifiers())
            .fold(ModifierCombination::new(), |acc, k| acc | k.mods)
    }

    /// Distinct report usages in press order; multiple holders share one slot.
    fn held_keycodes(&self) -> [HidKeyCode; 6] {
        let mut keys = [HidKeyCode::No; 6];
        let mut n = 0;
        for key in self.registered.iter().filter_map(RegisteredKey::keyboard_key) {
            if !keys[..n].contains(&key) {
                keys[n] = key;
                n += 1;
            }
        }
        keys
    }
}

/// A real key identifies itself; modifier-only inputs use their complete modifier mask.
fn same_key(left: (KeyCode, ModifierCombination), right: (KeyCode, ModifierCombination)) -> bool {
    left.0 == right.0 && (left.0 != KeyCode::Hid(HidKeyCode::No) || left.1 == right.1)
}

#[derive(Clone, Copy, Debug)]
struct RegisteredKey {
    /// The source of the registered key.
    source: KeyboardEventSource,
    /// Pressed keycode, `No` for a modifier action.
    keycode: KeyCode,
    /// A modifier action's modifiers, or a `KeyWithModifier` key's.
    mods: ModifierCombination,
}

impl RegisteredKey {
    /// Ordinary WM modifiers expire at the next press; sticky ones last until release.
    fn holds_modifiers(&self) -> bool {
        self.keycode == KeyCode::Hid(HidKeyCode::No)
            || !self.keycode.is_basic_keyboard_key()
            || matches!(self.source, KeyboardEventSource::Sticky(_))
    }

    fn keyboard_key(&self) -> Option<HidKeyCode> {
        match self.keycode {
            KeyCode::Hid(key) if key != HidKeyCode::No && key.is_keyboard_key() => Some(key),
            _ => None,
        }
    }

    /// Keyboard macros retain their physical-release compatibility; other outputs keep their source.
    fn matches(&self, event: &RegisteredKey) -> bool {
        match self.source {
            KeyboardEventSource::Macro if self.keycode.is_basic_keyboard_key() || event.source == self.source => {
                same_key((self.keycode, self.mods), (event.keycode, event.mods))
            }
            _ => self.source == event.source,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum KeyBehaviorDecision {
    // Clean holding buffer due to permissive hold is triggered
    CleanBuffer,
    // Skip key action processing and buffer key event
    Buffer,
    // Continue processing as normal key event
    Ignore,
    // Release current key
    Release,
    // Flow tap of current key is triggered
    FlowTap,
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub enum HeldKeyDecision {
    // Ignore it
    Ignore,
    // Unilateral tap triggered
    UnilateralTap,
    // Flow tap triggered, all held morse keys should be triggered as tapping
    FlowTap,
    // Permissive hold triggered
    PermissiveHold,
    // Hold on other key press triggered
    HoldOnOtherKeyPress,
    // Used for the buffered key which is releasing now
    Release,
    // Releasing a key that is pressed before any keys in the buffer
    NotInBuffer,
    // The held key is a normal key,
    // It will always be added to the decision list, and the decision will be made later
    Normal,
}

#[cfg(test)]
mod test {

    use embassy_time::Duration;
    use rmk_types::action::{KeyAction, TouchAction, TouchGesture};
    use rmk_types::fork::Fork;
    use rmk_types::modifier::ModifierCombination;
    use rmk_types::morse::{MorseMode, MorseProfile};

    use super::*;
    use crate::config::{BehaviorConfig, ForksConfig, PositionalConfig};
    use crate::event::{KeyPos, KeyboardEvent, KeyboardEventSource, TouchPos};
    use crate::test_support::test_block_on as block_on;
    use crate::{a, k, layer, mo, th, thp};

    #[rustfmt::skip]
    pub const fn get_keymap() -> [[[KeyAction; 14]; 5]; 2] {
        [
            layer!([
                [k!(Grave), k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(Kc5), k!(Kc6), k!(Kc7), k!(Kc8), k!(Kc9), k!(Kc0), k!(Minus), k!(Equal), k!(Backspace)],
                [k!(Tab), k!(Q), k!(W), k!(E), k!(R), k!(T), k!(Y), k!(U), k!(I), k!(O), k!(P), k!(LeftBracket), k!(RightBracket), k!(Backslash)],
                [k!(Escape), thp!(A, LShift, 0), th!(S, LGui), k!(D), k!(F), k!(G), k!(H), k!(J), k!(K), k!(L), k!(Semicolon), k!(Quote), a!(No), k!(Enter)],
                [k!(LShift), k!(Z), k!(X), k!(C), k!(V), k!(B), k!(N), k!(M), k!(Comma), k!(Dot), k!(Slash), a!(No), a!(No), k!(RShift)],
                [k!(LCtrl), k!(LGui), k!(LAlt), a!(No), a!(No), k!(Space), a!(No), a!(No), a!(No), mo!(1), k!(RAlt), a!(No), k!(RGui), k!(RCtrl)]
            ]),
            layer!([
                [k!(Grave), k!(F1), k!(F2), k!(F3), k!(F4), k!(F5), k!(F6), k!(F7), k!(F8), k!(F9), k!(F10), k!(F11), k!(F12), k!(Delete)],
                [a!(No), a!(Transparent), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
                [k!(CapsLock), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No)],
                [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), k!(Up)],
                [a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), a!(No), k!(Left), a!(No), k!(Down), k!(Right)]
            ]),
        ]
    }

    fn create_test_keyboard_with_config(mut config: BehaviorConfig) -> Keyboard<'static> {
        // `get_keymap`'s tap-hold at (row 2, col 1) references profile index 0.
        // Populate it unless the caller supplied its own table.
        if config.morse.profiles.is_empty() {
            let _ = config.morse.profiles.push(MorseProfile::new(
                Some(true),
                Some(MorseMode::PermissiveHold),
                None,
                None,
            ));
        }
        // Box::leak is acceptable in tests: nextest runs each #[test] in its own process,
        // so the leaked memory is reclaimed when the process exits.
        let behavior_config: &'static mut BehaviorConfig = Box::leak(Box::new(config));
        let per_key_config: &'static PositionalConfig<5, 14> = Box::leak(Box::new(PositionalConfig::default()));
        let data = Box::leak(Box::new(crate::keymap::KeymapData::new(get_keymap())));
        let keymap = block_on(KeyMap::new(data, behavior_config, per_key_config));
        let keymap_ref = Box::leak(Box::new(keymap));

        Keyboard::new(keymap_ref)
    }

    fn create_test_keyboard() -> Keyboard<'static> {
        create_test_keyboard_with_config(BehaviorConfig::default())
    }

    async fn force_timeout_first_hold(keyboard: &mut Keyboard<'static>) {
        let key = keyboard.next_buffered_key().unwrap();
        embassy_time::Timer::at(key.timeout_time).await;
        keyboard.fire_buffered_key_timeout(key).await;
    }

    fn create_test_keyboard_with_forks(fork1: Fork, fork2: Fork) -> Keyboard<'static> {
        let mut cfg = ForksConfig::default();
        let _ = cfg.forks.push(fork1);
        let _ = cfg.forks.push(fork2);
        create_test_keyboard_with_config(BehaviorConfig {
            fork: cfg,
            ..BehaviorConfig::default()
        })
    }

    /// A test keyboard with one touchpad, whose actions per layer come from `touch_map`.
    fn create_test_keyboard_with_touch(touch_map: [[TouchAction; 1]; 2]) -> Keyboard<'static> {
        let mut config = BehaviorConfig::default();
        // `get_keymap`'s tap-hold at (row 2, col 1) references profile index 0.
        let _ = config.morse.profiles.push(MorseProfile::new(
            Some(true),
            Some(MorseMode::PermissiveHold),
            None,
            None,
        ));
        let behavior_config: &'static mut BehaviorConfig = Box::leak(Box::new(config));
        let per_key_config: &'static PositionalConfig<5, 14> = Box::leak(Box::new(PositionalConfig::default()));
        let data = Box::leak(Box::new(
            crate::keymap::KeymapData::new(get_keymap()).with_touch(touch_map),
        ));
        let keymap = Box::leak(Box::new(block_on(KeyMap::new(data, behavior_config, per_key_config))));
        Keyboard::new(keymap)
    }

    /// A press or release of touchpad 0's tap.
    fn tap(pressed: bool) -> KeyboardEvent {
        KeyboardEvent {
            pressed,
            source: KeyboardEventSource::Touch(TouchPos {
                id: 0,
                gesture: TouchGesture::Tap,
            }),
        }
    }

    #[test]
    fn touch_gesture_runs_its_action_on_the_active_layer() {
        let main = async {
            let mut keyboard = create_test_keyboard_with_touch([
                [TouchAction::new().with(TouchGesture::Tap, k!(B))],
                [TouchAction::new().with(TouchGesture::Tap, k!(C))],
            ]);

            keyboard.process_inner(tap(true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::B);
            keyboard.process_inner(tap(false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);

            // A tap started on layer 0 releases its layer 0 action after layer 1 turns on.
            keyboard.process_inner(tap(true)).await;
            keyboard.process_inner(KeyboardEvent::key(4, 9, true)).await;
            keyboard.process_inner(tap(false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);

            // With layer 1 on, the tap runs layer 1's action.
            keyboard.process_inner(tap(true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::C);
            keyboard.process_inner(tap(false)).await;
            keyboard.process_inner(KeyboardEvent::key(4, 9, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
        };
        block_on(main);
    }

    #[test]
    fn a_transparent_touch_gesture_takes_the_action_below() {
        let main = async {
            let mut keyboard = create_test_keyboard_with_touch([
                [TouchAction::new().with(TouchGesture::Tap, k!(B))],
                [TouchAction::transparent()],
            ]);

            keyboard.process_inner(KeyboardEvent::key(4, 9, true)).await;
            keyboard.process_inner(tap(true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::B);
            keyboard.process_inner(tap(false)).await;
            keyboard.process_inner(KeyboardEvent::key(4, 9, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
        };
        block_on(main);
    }

    #[test]
    fn direct_keycodes_bypass_keymap_combos_and_forks() {
        block_on(async {
            let mut behavior = BehaviorConfig::default();
            behavior.combo.combos[0] = Some(Combo::new(rmk_types::combo::Combo::new(
                [k!(Right), k!(Up)],
                k!(Escape),
                None,
            )));
            behavior
                .fork
                .forks
                .push(Fork {
                    trigger: k!(Right),
                    positive_output: k!(B),
                    negative_output: k!(C),
                    ..Fork::default()
                })
                .unwrap();
            let mut keyboard = create_test_keyboard_with_config(behavior);
            keyboard.keymap.activate_layer(1);
            keyboard
                .process_inner(KeyboardEvent::virtual_key(HidKeyCode::Right as u8, true))
                .await;
            keyboard
                .process_inner(KeyboardEvent::virtual_key(HidKeyCode::Right as u8, false))
                .await;
            assert!(keyboard.registered.is_empty());
            keyboard
                .process_inner(KeyboardEvent::keycode(HidKeyCode::Right, true))
                .await;
            keyboard
                .process_inner(KeyboardEvent::keycode(HidKeyCode::Up, true))
                .await;
            assert_eq!(&keyboard.held_keycodes()[..2], &[HidKeyCode::Right, HidKeyCode::Up]);
            assert!(keyboard.held_buffer.is_empty());
            assert!(keyboard.fork_states.iter().all(Option::is_none));
            keyboard
                .process_inner(KeyboardEvent::keycode(HidKeyCode::Right, false))
                .await;
            keyboard
                .process_inner(KeyboardEvent::keycode(HidKeyCode::Up, false))
                .await;
            assert!(keyboard.registered.is_empty());
        });
    }

    #[test]
    fn repeated_direct_presses_do_not_release_physical_keys() {
        block_on(async {
            let mut keyboard = create_test_keyboard();
            for keycode in [HidKeyCode::Right, HidKeyCode::LShift] {
                keyboard.keymap.set_action_at(
                    KeyboardEventSource::key_pos(0, 0),
                    0,
                    KeyAction::Single(Action::Key(KeyCode::Hid(keycode))),
                );
                keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
                keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
                keyboard.process_inner(KeyboardEvent::keycode(keycode, true)).await;
                keyboard.process_inner(KeyboardEvent::keycode(keycode, true)).await;
                assert_eq!(keyboard.registered.len(), 2);
                keyboard.process_inner(KeyboardEvent::keycode(keycode, false)).await;
                assert_eq!(keyboard.registered.len(), 1);
                assert_eq!(keyboard.registered[0].source, KeyboardEventSource::key_pos(0, 0));
                keyboard.process_inner(KeyboardEvent::keycode(keycode, false)).await;
                assert_eq!(keyboard.registered.len(), 1);
                keyboard.process_inner(KeyboardEvent::key(0, 0, false)).await;
                assert!(keyboard.registered.is_empty());
            }
        });
    }

    #[cfg(not(feature = "_no_usb"))]
    #[test]
    fn direct_keycodes_use_normal_sticky_and_consumer_execution() {
        use rmk_types::connection::UsbState;
        use rmk_types::keycode::ConsumerKey;

        use crate::channel::USB_REPORT_CHANNEL;

        block_on(async {
            let mut keyboard = create_test_keyboard();
            crate::state::set_usb_state(UsbState::Configured);
            keyboard.keymap.set_action_at(
                KeyboardEventSource::key_pos(0, 0),
                0,
                KeyAction::Sticky(Action::Modifier(ModifierCombination::new().with_left_shift(true)), 0),
            );
            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            keyboard.process_inner(KeyboardEvent::key(0, 0, false)).await;
            USB_REPORT_CHANNEL.clear();
            keyboard
                .process_inner(KeyboardEvent::keycode(HidKeyCode::Right, true))
                .await;
            let Report::KeyboardReport(report) = USB_REPORT_CHANNEL.try_receive().unwrap() else {
                panic!()
            };
            assert_eq!(report.modifier, 2);
            assert_eq!(report.keycodes[0], HidKeyCode::Right as u8);
            keyboard
                .process_inner(KeyboardEvent::keycode(HidKeyCode::Right, false))
                .await;
            let Report::KeyboardReport(report) = USB_REPORT_CHANNEL.try_receive().unwrap() else {
                panic!()
            };
            assert_eq!(report.modifier, 2);
            assert_eq!(report.keycodes, [0; 6]);
            let Report::KeyboardReport(report) = USB_REPORT_CHANNEL.try_receive().unwrap() else {
                panic!()
            };
            assert_eq!(report.modifier, 0);
            assert_eq!(report.keycodes, [0; 6]);
            assert!(keyboard.registered.is_empty());

            keyboard
                .process_inner(KeyboardEvent::keycode(HidKeyCode::AudioVolUp, true))
                .await;
            let Report::MediaKeyboardReport(report) = USB_REPORT_CHANNEL.try_receive().unwrap() else {
                panic!()
            };
            assert_eq!({ report.usage_id }, u16::from(ConsumerKey::VolumeIncrement));
            keyboard
                .process_inner(KeyboardEvent::keycode(HidKeyCode::AudioVolUp, false))
                .await;
            let Report::MediaKeyboardReport(report) = USB_REPORT_CHANNEL.try_receive().unwrap() else {
                panic!()
            };
            assert_eq!({ report.usage_id }, 0);
        });
    }

    #[test]
    fn test_register_key() {
        let main = async {
            let mut keyboard = create_test_keyboard();
            keyboard.update_registered_key(
                KeyCode::Hid(HidKeyCode::A),
                ModifierCombination::new(),
                KeyboardEvent::key(2, 1, true),
            );
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::A);
        };
        block_on(main);
    }

    #[test]
    fn test_basic_key_press_release() {
        let main = async {
            let mut keyboard = create_test_keyboard();

            // Press A key
            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::Grave); // A key's HID code is 0x04

            // Release A key
            keyboard.process_inner(KeyboardEvent::key(0, 0, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
        };
        block_on(main);
    }

    #[test]
    fn test_modifier_key() {
        let main = async {
            let mut keyboard = create_test_keyboard();

            // Press Shift key
            keyboard.update_registered_key(
                KeyCode::Hid(HidKeyCode::LShift),
                ModifierCombination::new(),
                KeyboardEvent::key(3, 0, true),
            );
            assert_eq!(
                keyboard.held_modifiers(),
                ModifierCombination::new().with_left_shift(true)
            ); // Left Shift's modifier bit is 0x02

            // Release Shift key
            keyboard.update_registered_key(
                KeyCode::Hid(HidKeyCode::LShift),
                ModifierCombination::new(),
                KeyboardEvent::key(3, 0, false),
            );
            assert_eq!(keyboard.held_modifiers(), ModifierCombination::new());
        };
        block_on(main);
    }

    #[test]
    fn test_multiple_keys() {
        let main = async {
            let mut keyboard = create_test_keyboard();

            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            assert!(keyboard.held_keycodes().contains(&HidKeyCode::Grave));

            keyboard.process_inner(KeyboardEvent::key(1, 0, true)).await;
            assert!(
                keyboard.held_keycodes().contains(&HidKeyCode::Grave)
                    && keyboard.held_keycodes().contains(&HidKeyCode::Tab)
            );

            keyboard.process_inner(KeyboardEvent::key(1, 0, false)).await;
            assert!(
                keyboard.held_keycodes().contains(&HidKeyCode::Grave)
                    && !keyboard.held_keycodes().contains(&HidKeyCode::Tab)
            );

            keyboard.process_inner(KeyboardEvent::key(0, 0, false)).await;
            assert!(!keyboard.held_keycodes().contains(&HidKeyCode::Grave));
            assert!(keyboard.held_keycodes().iter().all(|&k| k == HidKeyCode::No));
        };

        block_on(main);
    }

    #[test]
    fn test_repeat_key_single() {
        let main = async {
            let mut keyboard = create_test_keyboard();
            keyboard.keymap.set_action_at(
                KeyboardEventSource::Key(KeyPos { row: 0, col: 0 }),
                0,
                KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Again))),
            );

            // first press ever of the Again issues KeyCode:No
            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No); // A key's HID code is 0x04

            // Press A key
            keyboard.process_inner(KeyboardEvent::key(2, 0, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::Escape); // A key's HID code is 0x04

            // Release A key
            keyboard.process_inner(KeyboardEvent::key(2, 0, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);

            // after another key is pressed, that key is repeated
            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::Escape); // A key's HID code is 0x04

            // releasing the repeat key
            keyboard.process_inner(KeyboardEvent::key(0, 0, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No); // A key's HID code is 0x04

            // Press S key
            keyboard.process_inner(KeyboardEvent::key(1, 2, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::W); // A key's HID code is 0x04

            // after another key is pressed, that key is repeated
            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::W); // A key's HID code is 0x04
        };
        block_on(main);
    }

    #[test]
    fn test_repeat_key_th() {
        let main = async {
            let mut keyboard = create_test_keyboard();
            keyboard.keymap.set_action_at(
                KeyboardEventSource::Key(KeyPos { row: 0, col: 0 }),
                0,
                KeyAction::TapHold(
                    Action::Key(KeyCode::Hid(HidKeyCode::F)),
                    Action::Key(KeyCode::Hid(HidKeyCode::Again)),
                    Default::default(),
                ),
            );
            keyboard.keymap.set_action_at(
                KeyboardEventSource::Key(KeyPos { row: 2, col: 1 }),
                0,
                KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A))),
            );
            keyboard.keymap.set_action_at(
                KeyboardEventSource::Key(KeyPos { row: 2, col: 2 }),
                0,
                KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::S))),
            );

            // Press down F
            // first press ever of the Again issues KeyCode:No
            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            keyboard.send_keyboard_report_with_resolved_modifiers(true).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
            // Release F
            keyboard.process_inner(KeyboardEvent::key(0, 0, false)).await;

            // Press A key
            keyboard.process_inner(KeyboardEvent::key(2, 1, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::A);

            // Release A key
            keyboard.process_inner(KeyboardEvent::key(2, 1, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);

            // Release F
            keyboard.process_inner(KeyboardEvent::key(0, 0, false)).await;

            // Here release event should make again into hold

            // Skip ahead 200ms of virtual time.
            embassy_time::MockDriver::get().advance(Duration::from_millis(200));
            // after another key is pressed, that key is repeated
            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            force_timeout_first_hold(&mut keyboard).await;

            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::A);

            // releasing the repeat key
            keyboard.process_inner(KeyboardEvent::key(0, 0, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);

            // Press S key
            keyboard.process_inner(KeyboardEvent::key(2, 2, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::S);

            // after another key is pressed, that key is repeated
            keyboard.process_inner(KeyboardEvent::key(0, 0, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::S);
        };
        block_on(main);
    }

    #[test]
    fn test_key_action_transparent() {
        let main = async {
            let mut keyboard = create_test_keyboard();

            // Activate layer 1
            keyboard.keymap.hold_layer(1, KeyboardEvent::key(0, 0, true));

            // Press Transparent key (Q on lower layer)
            keyboard.process_inner(KeyboardEvent::key(1, 1, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::Q); // Q key's HID code is 0x14

            // Release Transparent key
            keyboard.process_inner(KeyboardEvent::key(1, 1, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
        };
        block_on(main);
    }

    #[test]
    fn test_key_action_no() {
        let main = async {
            let mut keyboard = create_test_keyboard();

            // Press No key
            keyboard.process_inner(KeyboardEvent::key(4, 3, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);

            // Release No key
            keyboard.process_inner(KeyboardEvent::key(4, 3, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
        };
        block_on(main);
    }

    #[test]
    fn test_fork_with_held_modifier() {
        let main = async {
            //{ trigger = "Dot", negative_output = "Dot", positive_output = "WM(Semicolon, LShift)", match_any = "LShift|RShift" },
            let fork1 = Fork {
                trigger: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Dot))),
                negative_output: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Dot))),
                positive_output: KeyAction::Single(Action::KeyWithModifier(
                    HidKeyCode::Semicolon,
                    ModifierCombination::default().with_left_shift(true),
                )),
                match_any: StateBits {
                    modifiers: ModifierCombination::default()
                        .with_left_shift(true)
                        .with_right_shift(true),
                    leds: LedIndicator::default(),
                    mouse: MouseButtons::default(),
                },
                match_none: StateBits::default(),
                kept_modifiers: ModifierCombination::default(),
                bindable: false,
                layers: u32::MAX,
            };

            //{ trigger = "Comma", negative_output = "Comma", positive_output = "Semicolon", match_any = "LShift|RShift" },
            let fork2 = Fork {
                trigger: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Comma))),
                negative_output: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Comma))),
                positive_output: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Semicolon))),
                match_any: StateBits {
                    modifiers: ModifierCombination::default()
                        .with_left_shift(true)
                        .with_right_shift(true),
                    leds: LedIndicator::default(),
                    mouse: MouseButtons::default(),
                },
                match_none: StateBits::default(),
                kept_modifiers: ModifierCombination::default(),
                bindable: false,
                layers: u32::MAX,
            };

            let mut keyboard = create_test_keyboard_with_forks(fork1, fork2);

            // Press Dot key, by itself it should emit '.'
            keyboard.process_inner(KeyboardEvent::key(3, 9, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::Dot);

            // Release Dot key
            keyboard.process_inner(KeyboardEvent::key(3, 9, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);

            // Press LShift key
            keyboard.process_inner(KeyboardEvent::key(3, 0, true)).await;

            // Press Dot key, with shift it should emit ':'
            keyboard.process_inner(KeyboardEvent::key(3, 9, true)).await;
            assert_eq!(
                keyboard.resolve_modifiers(true),
                ModifierCombination::new().with_left_shift(true)
            );
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::Semicolon);

            //Release Dot key
            keyboard.process_inner(KeyboardEvent::key(3, 9, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
            assert_eq!(
                keyboard.resolve_modifiers(false),
                ModifierCombination::new().with_left_shift(true)
            );

            // Release LShift key
            keyboard.process_inner(KeyboardEvent::key(3, 0, false)).await;
            assert_eq!(keyboard.held_modifiers(), ModifierCombination::new());
            assert_eq!(keyboard.resolve_modifiers(false), ModifierCombination::new());

            // Press Comma key, by itself it should emit ','
            keyboard.process_inner(KeyboardEvent::key(3, 8, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::Comma);

            // Release Dot key
            keyboard.process_inner(KeyboardEvent::key(3, 8, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);

            // Press LShift key
            keyboard.process_inner(KeyboardEvent::key(3, 0, true)).await;

            // Press Comma key, with shift it should emit ';' (shift is suppressed)
            keyboard.process_inner(KeyboardEvent::key(3, 8, true)).await;
            assert_eq!(keyboard.resolve_modifiers(true), ModifierCombination::new());
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::Semicolon);

            // Release Comma key, shift is still held
            keyboard.process_inner(KeyboardEvent::key(3, 8, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
            assert_eq!(
                keyboard.resolve_modifiers(false),
                ModifierCombination::new().with_left_shift(true)
            );

            // Release LShift key
            keyboard.process_inner(KeyboardEvent::key(3, 0, false)).await;
            assert_eq!(keyboard.held_modifiers(), ModifierCombination::new());
            assert_eq!(keyboard.resolve_modifiers(false), ModifierCombination::new());
        };

        block_on(main);
    }
    #[test]
    fn test_fork_with_held_mouse_button() {
        let main = async {
            //{ trigger = "Z", negative_output = "MouseBtn5", positive_output = "C", match_any = "LCtrl|RCtrl|LShift|RShift", kept_modifiers="LShift|RShift" },
            let fork1 = Fork {
                trigger: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Z))),
                negative_output: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::MouseBtn5))),
                positive_output: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::C))),
                match_any: StateBits {
                    modifiers: ModifierCombination::default()
                        .with_left_ctrl(true)
                        .with_right_ctrl(true)
                        .with_left_shift(true)
                        .with_right_shift(true),
                    leds: LedIndicator::default(),
                    mouse: MouseButtons::default(),
                },
                match_none: StateBits::default(),
                kept_modifiers: ModifierCombination::default()
                    .with_left_shift(true)
                    .with_right_shift(true),
                bindable: false,
                layers: u32::MAX,
            };

            //{ trigger = "A", negative_output = "S", positive_output = "D", match_any = "MouseBtn5" },
            let fork2 = Fork {
                trigger: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A))),
                negative_output: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::S))),
                positive_output: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::D))),
                match_any: StateBits {
                    modifiers: ModifierCombination::default(),
                    leds: LedIndicator::default(),
                    mouse: MouseButtons::default().with_button5(true),
                },
                match_none: StateBits::default(),
                kept_modifiers: ModifierCombination::default(),
                bindable: false,
                layers: u32::MAX,
            };

            let mut keyboard = create_test_keyboard_with_forks(fork1, fork2);

            // disable th on a
            keyboard.keymap.set_action_at(
                KeyboardEventSource::Key(KeyPos { row: 2, col: 1 }),
                0,
                KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A))),
            );

            // Press Z key, by itself it should emit 'MouseBtn5'
            keyboard.process_inner(KeyboardEvent::key(3, 1, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
            assert_eq!(keyboard.mouse.report.buttons, 1u8 << 4); // MouseBtn5

            // Release Z key
            keyboard.process_inner(KeyboardEvent::key(3, 1, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
            assert_eq!(keyboard.mouse.report.buttons, 0);

            // Press LCtrl key
            keyboard.process_inner(KeyboardEvent::key(4, 0, true)).await;
            // Press LShift key
            keyboard.process_inner(KeyboardEvent::key(3, 0, true)).await;
            assert_eq!(
                keyboard.resolve_modifiers(true),
                ModifierCombination::new().with_left_ctrl(true).with_left_shift(true)
            );

            // Press 'Z' key, with Ctrl it should emit 'C', with suppressed ctrl, but kept shift
            keyboard.process_inner(KeyboardEvent::key(3, 1, true)).await;
            assert_eq!(
                keyboard.resolve_modifiers(true),
                ModifierCombination::new().with_left_shift(true)
            );
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::C);
            assert_eq!(keyboard.mouse.report.buttons, 0);

            // Release 'Z' key, suppression of ctrl is removed
            keyboard.process_inner(KeyboardEvent::key(3, 1, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
            assert_eq!(
                keyboard.resolve_modifiers(false),
                ModifierCombination::new().with_left_ctrl(true).with_left_shift(true)
            );

            // Release LCtrl key
            keyboard.process_inner(KeyboardEvent::key(4, 0, false)).await;
            assert_eq!(
                keyboard.resolve_modifiers(false),
                ModifierCombination::new().with_left_shift(true)
            );

            // Release LShift key
            keyboard.process_inner(KeyboardEvent::key(3, 0, false)).await;
            assert_eq!(keyboard.resolve_modifiers(false), ModifierCombination::new());

            // Press 'A' key, by itself it should emit 'S'
            keyboard.process_inner(KeyboardEvent::key(2, 1, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::S);

            // Release 'A' key
            keyboard.process_inner(KeyboardEvent::key(2, 1, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
            assert_eq!(keyboard.resolve_modifiers(false), ModifierCombination::new());
            assert_eq!(keyboard.mouse.report.buttons, 0);

            // Skip ahead 200ms of virtual time.
            embassy_time::MockDriver::get().advance(Duration::from_millis(200));

            // Press Z key, by itself it should emit 'MouseBtn5'
            keyboard.process_inner(KeyboardEvent::key(3, 1, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
            assert_eq!(keyboard.mouse.report.buttons, 1u8 << 4); // MouseBtn5 //this fails, but ok in debug - why?

            // Press 'A' key, with 'MouseBtn5' it should emit 'D'
            keyboard.process_inner(KeyboardEvent::key(2, 1, true)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::D);

            // Release Z (MouseBtn1) key, 'D' is still held
            keyboard.process_inner(KeyboardEvent::key(3, 1, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::D);

            // Release 'A' key -> releases 'D'
            keyboard.process_inner(KeyboardEvent::key(2, 1, false)).await;
            assert_eq!(keyboard.held_keycodes()[0], HidKeyCode::No);
        };

        block_on(main);
    }
}
