use embassy_futures::select::{Either, select};
use embassy_time::Timer;
use rmk_types::modifier::ModifierCombination;

use crate::event::KeyboardEvent;
use crate::keyboard::Keyboard;

/// State machine for one shot keys
#[derive(Default)]
pub enum OneShotState<T> {
    /// First one shot key press
    Initial(T),
    /// One shot key was released before any other key, normal one shot behavior
    Single(T),
    /// Another key was pressed before one shot key was released, treat as a normal modifier/layer
    Held(T),
    /// One shot inactive
    #[default]
    None,
}

impl<T> OneShotState<T> {
    /// Get the current one shot value if any
    pub fn value(&self) -> Option<&T> {
        match self {
            OneShotState::Initial(v) | OneShotState::Single(v) | OneShotState::Held(v) => Some(v),
            OneShotState::None => None,
        }
    }
}

impl<'a> Keyboard<'a> {
    pub(crate) async fn process_action_osm(&mut self, new_modifiers: ModifierCombination, event: KeyboardEvent) {
        let activate_on_keypress = self.keymap.one_shot_modifiers_config().activate_on_keypress;

        // Update one shot state
        if event.pressed {
            let mut was_active = false;
            // Add new modifier combination to existing one shot or init if none
            self.osm_state = match self.osm_state {
                OneShotState::None => OneShotState::Initial(new_modifiers),
                OneShotState::Initial(cur_modifiers) => OneShotState::Initial(cur_modifiers | new_modifiers),
                OneShotState::Single(cur_modifiers) => {
                    was_active = cur_modifiers & new_modifiers == new_modifiers;

                    if was_active {
                        let result = cur_modifiers & !new_modifiers;
                        // Remove the matching event from unprocessed_events queue
                        self.unprocessed_events.retain(|e| e.pos != event.pos);
                        // Send report for current osm_state modifiers
                        self.send_keyboard_report_with_resolved_modifiers(true).await;

                        if result.into_bits() == 0 {
                            OneShotState::None
                        } else {
                            OneShotState::Single(result)
                        }
                    } else {
                        OneShotState::Single(cur_modifiers | new_modifiers)
                    }
                }
                OneShotState::Held(cur_modifiers) => OneShotState::Held(cur_modifiers | new_modifiers),
            };

            self.update_osl(event);

            // Send report for updated osm_state modifiers
            if was_active || activate_on_keypress {
                self.send_keyboard_report_with_resolved_modifiers(true).await;
            }
        } else {
            match self.osm_state {
                OneShotState::Initial(cur_modifiers) | OneShotState::Single(cur_modifiers) => {
                    self.osm_state = OneShotState::Single(cur_modifiers);
                    let timeout = Timer::after(self.keymap.one_shot_timeout());
                    match select(timeout, self.keyboard_event_subscriber.next_message_pure()).await {
                        Either::First(_) => {
                            // Timeout, release modifiers
                            self.update_osl(event);
                            self.osm_state = OneShotState::None;

                            // Send release report because modifiers were held
                            if activate_on_keypress {
                                self.send_keyboard_report_with_resolved_modifiers(false).await;
                            }
                        }
                        Either::Second(e) => {
                            // New event, send it to queue
                            if self.unprocessed_events.push(e).is_err() {
                                warn!("Unprocessed event queue is full, dropping event");
                            }
                        }
                    }
                }
                OneShotState::Held(cur_modifiers) => {
                    let was_active = cur_modifiers & new_modifiers == new_modifiers;

                    if !was_active {
                        return;
                    }

                    // Release modifier
                    self.update_osl(event);
                    self.osm_state = OneShotState::None;

                    // This sends a separate hid report with the
                    // currently registered modifiers except the
                    // one shot modifiers -> this way "releasing" them.
                    self.send_keyboard_report_with_resolved_modifiers(false).await;
                }
                _ => (),
            };
        }
    }

    pub(crate) async fn process_action_osl(&mut self, layer_num: u8, event: KeyboardEvent) {
        // Update one shot state
        if event.pressed {
            // Deactivate old layer if any
            if let Some(&l) = self.osl_state.value() {
                self.keymap.deactivate_layer(l);
            }

            // Update layer of one shot
            self.osl_state = match self.osl_state {
                OneShotState::None => OneShotState::Initial(layer_num),
                OneShotState::Initial(_) => OneShotState::Initial(layer_num),
                OneShotState::Single(_) => OneShotState::Single(layer_num),
                OneShotState::Held(_) => OneShotState::Held(layer_num),
            };

            // Activate new layer
            self.keymap.activate_layer(layer_num);
        } else {
            match self.osl_state {
                OneShotState::Initial(l) | OneShotState::Single(l) => {
                    self.osl_state = OneShotState::Single(l);

                    let timeout = embassy_time::Timer::after(self.keymap.one_shot_timeout());
                    match select(timeout, self.keyboard_event_subscriber.next_message_pure()).await {
                        Either::First(_) => {
                            // Timeout, deactivate layer
                            self.keymap.deactivate_layer(layer_num);
                            self.osl_state = OneShotState::None;
                        }
                        Either::Second(e) => {
                            // New event, send it to queue
                            if self.unprocessed_events.push(e).is_err() {
                                warn!("Unprocessed event queue is full, dropping event");
                            }
                        }
                    }
                }
                OneShotState::Held(layer_num) => {
                    self.osl_state = OneShotState::None;
                    self.keymap.deactivate_layer(layer_num);
                }
                _ => (),
            };
        }
    }

    /// Update OSM state based on the keyboard event.
    /// Returns `true` if the OSM was consumed (transitioned from Single to None).
    pub(crate) fn update_osm(&mut self, event: KeyboardEvent) -> bool {
        match self.osm_state {
            OneShotState::Initial(m) => {
                self.osm_state = OneShotState::Held(m);
                false
            }
            // Consume on press so a key rolled over before this one releases doesn't
            // also see the modifier (resolve_explicit_modifiers only applies `Single`
            // on the pressed report anyway, so the release report is unaffected).
            OneShotState::Single(_) if event.pressed => {
                self.osm_state = OneShotState::None;
                true
            }
            _ => false,
        }
    }

    pub(crate) fn update_osl(&mut self, event: KeyboardEvent) {
        match self.osl_state {
            OneShotState::Initial(l) => self.osl_state = OneShotState::Held(l),
            OneShotState::Single(layer_num) if !event.pressed => {
                self.keymap.deactivate_layer(layer_num);
                self.osl_state = OneShotState::None;
            }
            _ => (),
        }
    }
}
