//! Timelines needing capacities or independent terms rather than another board.

use rmk::config::BehaviorConfig;
use rmk::test_support::test_block_on;
use rmk::types::action::{Action, KeyAction};
use rmk::types::keycode::{HidKeyCode, KeyCode};
use rmk::types::modifier::ModifierCombination;
use rmk::types::morse::{MorseMode, MorseProfile};

use crate::simulator::SimKeyboard;

fn key(code: HidKeyCode) -> Action {
    Action::Key(KeyCode::Hid(code))
}

fn behavior(term: u16) -> BehaviorConfig {
    let mut behavior = BehaviorConfig::default();
    behavior.morse.default_profile = MorseProfile::new(Some(false), Some(MorseMode::Normal), Some(term), Some(250))
        .with_buffered_tap_hold(Some(true));
    behavior
}

#[test]
fn nested_short_tap_uses_release_time_not_late_execution_time() {
    test_block_on(async {
        let mut config = behavior(500);
        config
            .morse
            .profiles
            .push(MorseProfile::const_default().with_hold_timeout_ms(Some(200)))
            .unwrap();
        let outer = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let inner = KeyAction::TapHold(key(HidKeyCode::A), Action::Modifier(ModifierCombination::LSHIFT), 0);
        let mut keyboard = SimKeyboard::builder([[[outer, inner]], [[KeyAction::Transparent; 2]]])
            .behavior_config(config)
            .build()
            .await;
        keyboard
            .press(0, 0)
            .delay(50)
            .press(0, 1)
            .delay(50)
            .release(0, 1)
            .delay(450)
            .release(0, 0)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn full_pending_store_recovers_without_losing_tap_releases() {
    test_block_on(async {
        let owner = KeyAction::TapHold(key(HidKeyCode::Tab), Action::LayerOn(1), u8::MAX);
        let mut keyboard = SimKeyboard::builder([
            [[owner, KeyAction::Single(key(HidKeyCode::Q))]],
            [[KeyAction::Transparent; 2]],
        ])
        .behavior_config(behavior(5000))
        .build()
        .await;
        keyboard.press(0, 0).delay(10);
        for _ in 0..15 {
            keyboard.tap(0, 1, 10).delay(10);
        }
        // Consume recovery reports before feeding more input: the HID channel
        // is bounded too, but this case is about held-record capacity.
        keyboard.press(0, 1).expect_keys([HidKeyCode::Tab]);
        for _ in 0..15 {
            keyboard
                .expect_keys([HidKeyCode::Tab, HidKeyCode::Q])
                .expect_keys([HidKeyCode::Tab]);
        }
        keyboard
            .expect_keys([HidKeyCode::Tab, HidKeyCode::Q])
            .release(0, 1)
            .expect_keys([HidKeyCode::Tab]);
        for _ in 0..8 {
            keyboard
                .tap(0, 1, 10)
                .expect_keys([HidKeyCode::Tab, HidKeyCode::Q])
                .expect_keys([HidKeyCode::Tab]);
        }
        keyboard
            .release(0, 0)
            .expect_keys([])
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::Q])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn a_full_live_store_rejects_new_actions_but_releases_every_layer_owner() {
    test_block_on(async {
        let mut base = [[KeyAction::Single(Action::LayerOn(1)); 17]];
        base[0][16] = KeyAction::Single(key(HidKeyCode::A));
        let mut upper = [[KeyAction::Transparent; 17]];
        upper[0][16] = KeyAction::Single(key(HidKeyCode::B));
        let mut keyboard = SimKeyboard::builder([base, upper])
            .behavior_config(behavior(200))
            .build()
            .await;
        for col in 0..16 {
            keyboard.press(0, col).delay(10);
        }
        keyboard.tap(0, 16, 10);
        for col in 0..16 {
            keyboard.release(0, col).delay(10);
        }
        keyboard
            .tap(0, 16, 10)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn a_legacy_key_created_during_replay_observes_later_presses() {
    test_block_on(async {
        let mut config = behavior(500);
        config
            .morse
            .profiles
            .push(
                MorseProfile::new(Some(false), Some(MorseMode::HoldOnOtherPress), Some(200), None)
                    .with_buffered_tap_hold(Some(false)),
            )
            .unwrap();
        let outer = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let legacy = KeyAction::TapHold(key(HidKeyCode::A), Action::Modifier(ModifierCombination::LSHIFT), 0);
        let ordinary = KeyAction::Single(key(HidKeyCode::B));
        let mut keyboard = SimKeyboard::builder([[[outer, legacy, ordinary]], [[KeyAction::Transparent; 3]]])
            .behavior_config(config)
            .build()
            .await;
        keyboard
            .press(0, 0)
            .delay(50)
            .press(0, 1)
            .delay(30)
            .press(0, 2)
            .delay(20)
            .release(0, 2)
            .delay(50)
            .release(0, 1)
            .delay(30)
            .release(0, 0)
            .expect_keys_with_mods(2, [])
            .expect_keys_with_mods(2, [HidKeyCode::B])
            .expect_keys_with_mods(2, [])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn replay_created_legacy_layer_decides_before_the_next_ordered_binding() {
    test_block_on(async {
        let mut config = behavior(500);
        config
            .morse
            .profiles
            .push(
                MorseProfile::new(Some(false), Some(MorseMode::HoldOnOtherPress), Some(400), None)
                    .with_buffered_tap_hold(Some(false)),
            )
            .unwrap();
        let outer = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let legacy = KeyAction::TapHold(Action::No, Action::LayerOn(2), 0);
        let inner = KeyAction::TapHold(
            key(HidKeyCode::A),
            Action::Modifier(ModifierCombination::LSHIFT),
            u8::MAX,
        );
        let mut upper = [[KeyAction::Transparent; 3]];
        upper[0][2] = KeyAction::Single(key(HidKeyCode::C));
        let mut keyboard = SimKeyboard::builder([[[outer, legacy, inner]], [[KeyAction::Transparent; 3]], upper])
            .behavior_config(config)
            .build()
            .await;
        keyboard
            .press(0, 0)
            .delay(50)
            .press(0, 1)
            .delay(30)
            .press(0, 2)
            .delay(40)
            .release(0, 2)
            .delay(30)
            .release(0, 1)
            .delay(30)
            .release(0, 0)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn ordered_hold_term_includes_its_exact_boundary() {
    test_block_on(async {
        for (duration, hold) in [(199, false), (200, true), (201, true)] {
            let owner = KeyAction::TapHold(
                key(HidKeyCode::A),
                Action::Modifier(ModifierCombination::LSHIFT),
                u8::MAX,
            );
            let mut keyboard = SimKeyboard::builder([[[owner]]])
                .behavior_config(behavior(200))
                .build()
                .await;
            keyboard.tap(0, 0, duration);
            if hold {
                keyboard.expect_keys_with_mods(2, []);
            } else {
                keyboard.expect_keys([HidKeyCode::A]);
            }
            keyboard.expect_keys([]).run().await;
        }
    });
}

#[test]
fn ordered_quick_tap_includes_the_window_and_can_be_disabled() {
    test_block_on(async {
        for (window, gap, hold) in [(120, 119, false), (120, 120, false), (120, 121, true), (0, 10, true)] {
            let mut config = behavior(200);
            config.morse.default_profile = config.morse.default_profile.with_quick_tap_timeout_ms(Some(window));
            let owner = KeyAction::TapHold(
                key(HidKeyCode::A),
                Action::Modifier(ModifierCombination::LSHIFT),
                u8::MAX,
            );
            let mut keyboard = SimKeyboard::builder([[[owner]]]).behavior_config(config).build().await;
            keyboard
                .tap(0, 0, 20)
                .delay(gap)
                .press(0, 0)
                .delay(220)
                .release(0, 0)
                .expect_keys([HidKeyCode::A])
                .expect_keys([]);
            if hold {
                keyboard.expect_keys_with_mods(2, []);
            } else {
                keyboard.expect_keys([HidKeyCode::A]);
            }
            keyboard.expect_keys([]).delay(250).run().await;
        }
    });
}

#[test]
fn malformed_edges_cannot_release_another_generation_or_stick_an_action() {
    use rmk::event::KeyboardEvent;
    test_block_on(async {
        let owner = KeyAction::TapHold(key(HidKeyCode::Tab), Action::LayerOn(1), u8::MAX);
        let ordinary = KeyAction::Single(key(HidKeyCode::A));
        let mut keyboard = SimKeyboard::builder([[[owner, ordinary]], [[KeyAction::Transparent; 2]]])
            .behavior_config(behavior(200))
            .build()
            .await;
        keyboard
            .inject_event(KeyboardEvent::key(0, 1, false))
            .press(0, 0)
            .press(0, 1)
            .inject_event(KeyboardEvent::key(0, 1, true))
            .release(0, 1)
            .inject_event(KeyboardEvent::key(0, 1, false))
            .press(0, 1)
            .release(0, 1)
            .delay(220)
            .release(0, 0)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn direct_keycodes_bypass_an_owner_and_virtual_events_do_not_become_owners() {
    use rmk::event::KeyboardEvent;
    test_block_on(async {
        let owner = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let mut keyboard = SimKeyboard::builder([[[owner]], [[KeyAction::Transparent]]])
            .behavior_config(behavior(200))
            .build()
            .await;
        keyboard
            .press(0, 0)
            .event(KeyboardEvent::keycode(HidKeyCode::B, true))
            .expect_keys([HidKeyCode::B])
            .within(5)
            .event(KeyboardEvent::virtual_key(42, true))
            .event(KeyboardEvent::virtual_key(42, false))
            .event(KeyboardEvent::keycode(HidKeyCode::B, false))
            .expect_keys([])
            .within(5)
            .delay(220)
            .release(0, 0)
            .run()
            .await;
    });
}

#[test]
fn a_live_macro_deadline_is_not_delayed_until_an_owner_resolves() {
    use rmk::types::keyboard_macros::MacroOp;
    test_block_on(async {
        let mut config = behavior(500);
        config.keyboard_macros = &[&[
            MacroOp::Delay(50),
            MacroOp::Tap(Action::Key(KeyCode::Hid(HidKeyCode::B))),
        ]];
        let owner = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let trigger = KeyAction::Single(Action::TriggerMacro(0));
        let ordinary = KeyAction::Single(key(HidKeyCode::A));
        let mut keyboard = SimKeyboard::builder([[[trigger, owner, ordinary]], [[KeyAction::Transparent; 3]]])
            .behavior_config(config)
            .build()
            .await;
        keyboard
            .press(0, 0)
            .release(0, 0)
            .press(0, 1)
            .delay(10)
            .press(0, 2)
            .expect_keys([HidKeyCode::B])
            .within(45)
            .expect_keys([])
            .within(15)
            .release(0, 1)
            .expect_keys([HidKeyCode::A])
            .release(0, 2)
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn deferred_legacy_flow_tap_continuation_expires_in_original_time() {
    use rmk::types::morse::{HOLD, Morse, TAP};
    test_block_on(async {
        let mut config = behavior(500);
        let mut morse = Morse {
            profile: MorseProfile::new(Some(false), Some(MorseMode::Normal), Some(200), Some(250))
                .with_buffered_tap_hold(Some(false))
                .with_enable_flow_tap(Some(true)),
            ..Morse::default()
        };
        morse.put(TAP, key(HidKeyCode::Z)).unwrap();
        morse.put(HOLD, Action::Modifier(ModifierCombination::RCTRL)).unwrap();
        morse.put(TAP.followed_by_hold(), key(HidKeyCode::Z)).unwrap();
        config.morse.morses.push(morse).unwrap();
        let outer = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let mut keyboard = SimKeyboard::builder([
            [[KeyAction::Single(key(HidKeyCode::H)), outer, KeyAction::Morse(0)]],
            [[KeyAction::Transparent; 3]],
        ])
        .behavior_config(config)
        .build()
        .await;
        keyboard
            .tap(0, 0, 10)
            .delay(30)
            .press(0, 1)
            .delay(50)
            .press(0, 2)
            .delay(50)
            .release(0, 2)
            .delay(300)
            .press(0, 2)
            .delay(300)
            .release(0, 2)
            .delay(60)
            .release(0, 1)
            .expect_keys([HidKeyCode::H])
            .expect_keys([])
            .expect_keys([HidKeyCode::Z])
            .expect_keys([])
            .expect_keys_with_mods(16, [])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn replay_cannot_adopt_a_newer_same_source_waiting_combo_generation() {
    use rmk::keyboard::combo::{Combo, ComboConfig};
    test_block_on(async {
        let mut config = behavior(200);
        let a = KeyAction::Single(key(HidKeyCode::A));
        let b = KeyAction::Single(key(HidKeyCode::B));
        config.combo.combos[0] = Some(Combo::new(ComboConfig::new(
            [a, b],
            KeyAction::Single(key(HidKeyCode::Z)),
            None,
        )));
        config.combo.timeout = embassy_time::Duration::from_millis(50);
        let owner = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let mut keyboard = SimKeyboard::builder([
            [[owner, a, b, KeyAction::Single(key(HidKeyCode::X))]],
            [[KeyAction::Transparent; 4]],
        ])
        .behavior_config(config)
        .build()
        .await;
        keyboard
            .press(0, 0)
            .delay(20)
            .press(0, 1)
            .delay(10)
            .press(0, 3)
            .delay(10)
            .release(0, 3)
            .delay(10)
            .release(0, 1)
            .delay(140)
            .press(0, 1)
            .delay(20)
            .press(0, 2)
            .delay(10)
            .release(0, 1)
            .delay(10)
            .release(0, 2)
            .release(0, 0)
            .expect_keys([HidKeyCode::A])
            .expect_keys([HidKeyCode::A, HidKeyCode::X])
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .expect_keys([HidKeyCode::Z])
            .expect_keys([])
            .tap(0, 2, 10)
            .expect_keys([HidKeyCode::B])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn legacy_timeout_flush_uses_the_prepared_fork_replacement() {
    use rmk::types::fork::Fork;
    test_block_on(async {
        let mut config = behavior(200);
        config
            .morse
            .profiles
            .push(
                MorseProfile::new(Some(false), Some(MorseMode::PermissiveHold), Some(200), None)
                    .with_buffered_tap_hold(Some(false)),
            )
            .unwrap();
        let a = KeyAction::Single(key(HidKeyCode::A));
        config
            .fork
            .forks
            .push(Fork {
                trigger: a,
                negative_output: KeyAction::Single(Action::LayerOn(1)),
                positive_output: KeyAction::Single(Action::LayerOn(1)),
                ..Fork::default()
            })
            .unwrap();
        let legacy = KeyAction::TapHold(Action::No, Action::No, 0);
        let mut upper = [[KeyAction::Transparent; 3]];
        upper[0][2] = KeyAction::Single(key(HidKeyCode::C));
        let mut keyboard = SimKeyboard::builder([[[legacy, a, KeyAction::Single(key(HidKeyCode::B))]], upper])
            .behavior_config(config)
            .build()
            .await;
        keyboard
            .press(0, 0)
            .delay(10)
            .press(0, 1)
            .delay(220)
            .tap(0, 2, 10)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .release(0, 1)
            .release(0, 0)
            .tap(0, 2, 10)
            .expect_keys([HidKeyCode::B])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn original_deadline_precedes_a_later_unilateral_observation() {
    use rmk::config::Hand;
    test_block_on(async {
        let mut config = behavior(500);
        config
            .morse
            .profiles
            .push(
                MorseProfile::new(Some(true), Some(MorseMode::Normal), Some(100), None)
                    .with_buffered_tap_hold(Some(false)),
            )
            .unwrap();
        let owner = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let legacy = KeyAction::TapHold(key(HidKeyCode::A), Action::Modifier(ModifierCombination::LSHIFT), 0);
        let mut keyboard = SimKeyboard::builder([
            [[owner, legacy, KeyAction::Single(key(HidKeyCode::B))]],
            [[KeyAction::Transparent; 3]],
        ])
        .hands([[Hand::Left; 3]])
        .behavior_config(config)
        .build()
        .await;
        keyboard
            .press(0, 0)
            .delay(50)
            .press(0, 1)
            .delay(200)
            .press(0, 2)
            .delay(10)
            .release(0, 2)
            .delay(40)
            .release(0, 1)
            .delay(100)
            .release(0, 0)
            .expect_keys_with_mods(2, [])
            .expect_keys_with_mods(2, [HidKeyCode::B])
            .expect_keys_with_mods(2, [])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn full_store_dispatches_combo_members_without_unowned_output() {
    use rmk::keyboard::combo::{Combo, ComboConfig};
    test_block_on(async {
        let mut config = behavior(200);
        let a = KeyAction::Single(key(HidKeyCode::A));
        let b = KeyAction::Single(key(HidKeyCode::B));
        config.combo.combos[0] = Some(Combo::new(ComboConfig::new(
            [a, b],
            KeyAction::Single(key(HidKeyCode::Z)),
            None,
        )));
        let mut base = [[KeyAction::Single(Action::LayerOn(1)); 16]];
        base[0][14] = a;
        base[0][15] = b;
        let mut keyboard = SimKeyboard::builder([base, [[KeyAction::Transparent; 16]]])
            .behavior_config(config)
            .build()
            .await;
        for col in 0..14 {
            keyboard.press(0, col).delay(1);
        }
        keyboard
            .press(0, 14)
            .press(0, 15)
            .release(0, 14)
            .release(0, 15)
            .expect_keys([HidKeyCode::A])
            .expect_keys([HidKeyCode::A, HidKeyCode::B])
            .expect_keys([HidKeyCode::B])
            .expect_keys([]);
        for col in 0..14 {
            keyboard.release(0, col);
        }
        keyboard
            .tap(0, 14, 10)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn overlapping_combo_release_burst_keeps_modifier_ownership_balanced() {
    use rmk::keyboard::combo::{Combo, ComboConfig};
    test_block_on(async {
        let mut config = behavior(500);
        let a = KeyAction::Single(key(HidKeyCode::A));
        let b = KeyAction::Single(key(HidKeyCode::B));
        let common = KeyAction::Single(key(HidKeyCode::Z));
        let shift = KeyAction::Single(Action::Modifier(ModifierCombination::LSHIFT));
        config.combo.combos[0] = Some(Combo::new(ComboConfig::new([a, common], shift, None)));
        config.combo.combos[1] = Some(Combo::new(ComboConfig::new([b, common], shift, None)));
        let owner = KeyAction::TapHold(Action::No, Action::No, u8::MAX);
        let mut keyboard = SimKeyboard::builder([[[owner, a, b, common]]])
            .behavior_config(config)
            .build()
            .await;
        keyboard
            .press(0, 0)
            .press(0, 1)
            .press(0, 3)
            .release(0, 1)
            .press(0, 2)
            .release(0, 2)
            .release(0, 3)
            .release(0, 0)
            .expect_keys_with_mods(2, [])
            .expect_keys_with_mods(2, [])
            .expect_keys_with_mods(2, [])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn touch_gesture_uses_the_layer_resolved_before_its_execution() {
    use rmk::event::{KeyboardEvent, KeyboardEventSource, TouchPos};
    use rmk::types::action::{TouchAction, TouchGesture};
    test_block_on(async {
        let owner = KeyAction::TapHold(Action::No, Action::LayerOn(1), u8::MAX);
        let down = KeyboardEvent {
            pressed: true,
            source: KeyboardEventSource::Touch(TouchPos {
                id: 0,
                gesture: TouchGesture::Tap,
            }),
        };
        let up = KeyboardEvent { pressed: false, ..down };
        let mut keyboard = SimKeyboard::builder([[[owner]], [[KeyAction::Transparent]]])
            .touchpads([
                [TouchAction::new().with(TouchGesture::Tap, KeyAction::Single(key(HidKeyCode::A)))],
                [TouchAction::new().with(TouchGesture::Tap, KeyAction::Single(key(HidKeyCode::B)))],
            ])
            .behavior_config(behavior(200))
            .build()
            .await;
        keyboard
            .press(0, 0)
            .delay(50)
            .event(down)
            .delay(40)
            .event(up)
            .delay(150)
            .release(0, 0)
            .expect_keys([HidKeyCode::B])
            .expect_keys([])
            .run()
            .await;
    });
}
