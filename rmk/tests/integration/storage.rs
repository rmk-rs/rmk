use rmk::config::{AutoShiftConfig, BehaviorConfig};
use rmk::test_support::test_block_on;
use rmk::types::keycode::HidKeyCode;

use crate::simulator::{Flash, SimKeyboard};

#[test]
fn auto_shift_toggle_survives_restart() {
    let behavior = || BehaviorConfig {
        auto_shift: AutoShiftConfig {
            enabled: true,
            groups: AutoShiftConfig::ALPHA,
            ..Default::default()
        },
        ..Default::default()
    };
    let keymap = [[[rmk::k!(A), rmk::kbctrl!(AutoShiftToggle)]]];
    test_block_on(async {
        let flash = Flash::new();
        {
            let mut keyboard = SimKeyboard::builder(keymap)
                .behavior_config(behavior())
                .build_with_flash(flash.clone())
                .await;
            keyboard
                .tap(0, 0, 300)
                .expect_keys_with_mods(0x02, [HidKeyCode::A])
                .expect_keys([])
                .tap(0, 1, 10)
                .tap(0, 0, 300)
                .expect_keys([HidKeyCode::A])
                .expect_keys([])
                .run()
                .await;
        }
        let mut keyboard = SimKeyboard::builder(keymap)
            .behavior_config(behavior())
            .build_with_flash(flash)
            .await;
        keyboard
            .tap(0, 0, 300)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .run()
            .await;
    });
}
