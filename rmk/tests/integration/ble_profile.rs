//! Profile-key gestures, which a scenario cannot observe: the harness records
//! what the keyboard sent the BLE profile task.

use rmk::k;
use rmk::test_support::test_block_on;
use rmk::types::action::{Action, KeyAction};
use rmk::types::keycode::{HidKeyCode, KeyCode};

use crate::simulator::SimKeyboard;

const USER0: KeyAction = KeyAction::Single(Action::User(0));

#[test]
fn held_profile_key_clears_its_bond_after_5s() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[USER0]]]).build().await;
        keyboard.press(0, 0).delay(5100).release(0, 0).run().await;
        assert_eq!(
            keyboard.ble_profile_actions(),
            ["ClearSlot(0)", "Switch(0)", "Switch(0)"]
        );
    });
}

#[test]
fn intervening_key_cancels_the_hold() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[USER0, k!(A)]]]).build().await;
        keyboard
            .press(0, 0)
            .delay(100)
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .release(0, 0)
            .delay(5200)
            .run()
            .await;
        assert_eq!(keyboard.ble_profile_actions(), ["Switch(0)"]);
    });
}

// Regression: the tap side of a tap-hold armed the hold on its synthesized
// press, nothing disarmed it, and the bond was wiped after 5s of idle.
#[test]
fn tapped_profile_key_action_does_not_clear_its_bond() {
    test_block_on(async {
        let tap_hold = KeyAction::TapHold(Action::User(0), Action::Key(KeyCode::Hid(HidKeyCode::Z)), u8::MAX);
        let mut keyboard = SimKeyboard::builder([[[tap_hold]]]).build().await;
        keyboard.tap(0, 0, 20).delay(5200).run().await;
        assert_eq!(keyboard.ble_profile_actions(), ["Switch(0)"]);
    });
}
