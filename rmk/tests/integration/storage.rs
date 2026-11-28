//! What a keyboard built over flash restores from the one that wrote it.

use rmk::test_support::test_block_on;
use rmk::types::keycode::HidKeyCode;
use rmk::{k, pdf};

use crate::simulator::{Flash, SimKeyboard};

/// Tap `PDF(1)` on a two-layer keyboard and hand back the flash it saved to.
async fn persist_default_layer_1() -> Flash {
    let flash = Flash::new();
    let mut keyboard = SimKeyboard::builder([[[pdf!(1), k!(A)]], [[pdf!(1), k!(B)]]])
        .build_with_flash(flash.clone())
        .await;
    keyboard.tap(0, 0, 10).run().await;
    flash
}

/// Without a host protocol nothing restored the saved default layer, so `PDF`
/// lasted only until the next power cycle.
#[test]
fn persistent_default_layer_survives_restart() {
    test_block_on(async {
        let flash = persist_default_layer_1().await;
        let mut keyboard = SimKeyboard::builder([[[pdf!(1), k!(A)]], [[pdf!(1), k!(B)]]])
            .build_with_flash(flash)
            .await;
        keyboard
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::B])
            .expect_keys([])
            .run()
            .await;
    });
}

/// A saved default layer past the end of a smaller keymap used to be restored
/// anyway, leaving every key dead until storage was cleared.
#[test]
fn saved_default_layer_beyond_the_keymap_is_ignored() {
    test_block_on(async {
        let flash = persist_default_layer_1().await;
        let mut keyboard = SimKeyboard::builder([[[pdf!(1), k!(A)]]]).build_with_flash(flash).await;
        keyboard
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .run()
            .await;
    });
}
