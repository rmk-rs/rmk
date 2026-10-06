//! Via/Vial host exchanges and their live-keyboard integration tests.

use rmk::config::BehaviorConfig;
use rmk::test_support::{test_block_on, to_via_keycode};
use rmk::types::action::{Action, EncoderAction, KeyAction};
use rmk::types::constants::{MACRO_MAX_NUM, MACRO_SPACE_SIZE};
use rmk::types::keyboard_macros::MacroOp;
use rmk::types::keycode::{HidKeyCode, KeyCode};
use rmk::types::modifier::ModifierCombination;
use rmk::{k, macros, osl, osm, sk, text};
use rmk_types::protocol::vial::{
    SettingKey, VIA_PROTOCOL_VERSION, VIAL_EP_SIZE as REPORT, ViaCommand, VialCommand, VialDynamic,
};

use crate::simulator::SimKeyboard;

fn via(cmd: ViaCommand) -> [u8; REPORT] {
    let mut data = [0; REPORT];
    data[0] = cmd as u8;
    data
}

fn vial(cmd: VialCommand) -> [u8; REPORT] {
    let mut data = via(ViaCommand::Vial);
    data[1] = cmd as u8;
    data
}

fn dynamic(op: VialDynamic, index: u8) -> [u8; REPORT] {
    let mut data = vial(VialCommand::DynamicEntryOp);
    data[2..4].copy_from_slice(&[op as u8, index]);
    data
}

impl SimKeyboard {
    fn echo(&mut self, request: [u8; REPORT]) {
        self.host_exchange(request, request);
    }

    fn echo_with_status(&mut self, request: [u8; REPORT]) {
        let mut expected = request;
        expected[0] = 0;
        self.host_exchange(request, expected);
    }

    fn set_behavior(&mut self, setting: SettingKey, value: u16) {
        let mut request = vial(VialCommand::SetBehaviorSetting);
        request[2..4].copy_from_slice(&(setting as u16).to_le_bytes());
        request[4..6].copy_from_slice(&value.to_le_bytes());
        self.echo(request)
    }

    fn expect_behavior(&mut self, setting: SettingKey, value: u16) {
        let mut request = vial(VialCommand::GetBehaviorSetting);
        request[2..4].copy_from_slice(&(setting as u16).to_le_bytes());
        let mut reply = [0xFF; REPORT];
        reply[0] = 0;
        reply[1..3].copy_from_slice(&value.to_le_bytes());
        self.host_exchange(request, reply);
    }

    fn set_combo<const N: usize>(&mut self, index: u8, actions: [KeyAction; N], output: KeyAction) {
        let mut request = dynamic(VialDynamic::DynamicVialComboSet, index);
        const MAX: usize = rmk::test_support::COMBO_MAX_LENGTH;
        assert!(N <= MAX);
        for (idx, action) in actions.into_iter().enumerate() {
            let start = 4 + idx * 2;
            request[start..start + 2].copy_from_slice(&to_via_keycode(action).to_le_bytes());
        }
        request[4 + MAX * 2..6 + MAX * 2].copy_from_slice(&to_via_keycode(output).to_le_bytes());
        self.echo_with_status(request)
    }
}

#[test]
fn protocol_version_round_trips() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build().await;
        let request = via(ViaCommand::GetProtocolVersion);
        let mut reply = request;
        reply[1..3].copy_from_slice(&VIA_PROTOCOL_VERSION.to_be_bytes());
        keyboard.host_exchange(request, reply);
        keyboard.run().await;
    });
}

#[test]
fn keymap_write_changes_the_key() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build().await;
        let mut request = via(ViaCommand::DynamicKeymapSetKeyCode);
        request[1..4].copy_from_slice(&[0, 0, 0]);
        request[4..6].copy_from_slice(&to_via_keycode(k!(B)).to_be_bytes());
        keyboard.echo(request);
        keyboard
            .tap(0, 0, 10)
            .expect_keys([HidKeyCode::B])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn one_shot_timeout_reads_and_updates_default_sticky_profile() {
    test_block_on(async {
        for (sticky, modifiers, consumed) in [
            (osm!(ModifierCombination::LCTRL), 1, HidKeyCode::A),
            (sk!(Action::Key(KeyCode::Hid(HidKeyCode::LCtrl))), 1, HidKeyCode::A),
            (osl!(1), 0, HidKeyCode::B),
        ] {
            let mut behavior = BehaviorConfig::default();
            behavior.sticky_key.default_profile.wait_timeout_ms = 230;
            let mut keyboard = SimKeyboard::builder([[[sticky, k!(A)]], [[KeyAction::Transparent, k!(B)]]])
                .behavior_config(behavior)
                .build()
                .await;
            keyboard.expect_behavior(SettingKey::OneShotTimeout, 230);
            keyboard.set_behavior(SettingKey::OneShotTimeout, 80);
            keyboard.expect_behavior(SettingKey::OneShotTimeout, 80);
            keyboard.tap(0, 0, 10);
            if modifiers != 0 {
                keyboard.expect_keys_with_mods(modifiers, []);
            }
            keyboard.delay(120);
            if modifiers != 0 {
                keyboard.expect_keys([]);
            }
            keyboard.tap(0, 1, 10).expect_keys([HidKeyCode::A]).expect_keys([]);
            keyboard.set_behavior(SettingKey::OneShotTimeout, 0);
            keyboard.expect_behavior(SettingKey::OneShotTimeout, 0);
            keyboard.tap(0, 0, 10);
            if modifiers != 0 {
                keyboard.expect_keys_with_mods(modifiers, []);
            }
            keyboard
                .delay(1500)
                .tap(0, 1, 10)
                .expect_keys_with_mods(modifiers, [consumed]);
            if modifiers != 0 {
                keyboard.expect_keys_with_mods(modifiers, []);
            }
            keyboard.expect_keys([]).run().await;
        }
    });
}

#[test]
fn encoder_write_changes_the_knob() {
    test_block_on(async {
        let action = EncoderAction::new(k!(C), k!(D));
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]])
            .encoders([[EncoderAction::new(k!(A), k!(B))]])
            .build()
            .await;
        for (direction, key) in [(1, action.clockwise), (0, action.counter_clockwise)] {
            let mut request = vial(VialCommand::SetEncoder);
            request[2..5].copy_from_slice(&[0, 0, direction]);
            request[5..7].copy_from_slice(&to_via_keycode(key).to_be_bytes());
            keyboard.echo(request);
        }
        keyboard
            .rotary_cw(0)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .rotary_ccw(0)
            .expect_keys([HidKeyCode::D])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn rejects_out_of_range_and_unknown_requests() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]])
            .encoders([[EncoderAction::new(k!(A), k!(B))]])
            .build()
            .await;
        let mut request = vial(VialCommand::GetEncoder);
        request[2..4].copy_from_slice(&[0, 99]);
        keyboard.host_exchange(request, [0; REPORT]);
        keyboard.host_exchange(dynamic(VialDynamic::Unhandled, 0), [0; REPORT]);
        keyboard.run().await;
    });
}

#[test]
fn combo_and_behavior_writes_change_the_chord() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[k!(A), k!(B)]]]).build().await;
        keyboard.set_combo(0, [k!(A), k!(B)], k!(C));
        keyboard.set_behavior(SettingKey::ComboTimeout, 80);
        keyboard
            .press(0, 0)
            .expect_no_report(60)
            .expect_keys([HidKeyCode::A])
            .release(0, 0)
            .expect_keys([])
            .delay(20)
            .press(0, 0)
            .delay(10)
            .press(0, 1)
            .expect_keys([HidKeyCode::C])
            .release(0, 0)
            .release(0, 1)
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn tap_capslock_interval_reads_back_its_own_value() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build().await;
        keyboard.set_behavior(SettingKey::TapInterval, 180);
        keyboard.set_behavior(SettingKey::TapCapslockInterval, 240);
        let mut request = vial(VialCommand::GetBehaviorSetting);
        request[2..4].copy_from_slice(&(SettingKey::TapCapslockInterval as u16).to_le_bytes());
        let mut reply = [0xFF; REPORT];
        reply[0] = 0;
        reply[1..3].copy_from_slice(&240u16.to_le_bytes());
        keyboard.host_exchange(request, reply);
        keyboard.run().await;
    });
}

#[test]
fn morse_write_changes_the_tap() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[rmk::td!(0)]]]).build().await;
        let mut request = dynamic(VialDynamic::DynamicVialMorseSet, 0);
        for (idx, action) in [k!(A), k!(B), k!(C), k!(D)].into_iter().enumerate() {
            let start = 4 + idx * 2;
            request[start..start + 2].copy_from_slice(&to_via_keycode(action).to_le_bytes());
        }
        request[12..14].copy_from_slice(&80u16.to_le_bytes());
        keyboard.echo_with_status(request);
        keyboard
            .delay(150)
            .tap(0, 0, 20)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .run()
            .await;
    });
}

/// Macro 0 is `tap A`, a pause Vial cannot spell, then `tap B`; macro 1 types `hi`.
const MACROS: &[&[MacroOp]] = &[
    &[
        MacroOp::Tap(Action::Key(KeyCode::Hid(HidKeyCode::A))),
        MacroOp::PauseForRelease,
        MacroOp::Tap(Action::Key(KeyCode::Hid(HidKeyCode::B))),
    ],
    &text!("hi"),
];

/// A two-key keyboard, `MACRO(0)` and `MACRO(1)`, with [`MACROS`] compiled in.
fn macro_keyboard() -> crate::simulator::SimKeyboardBuilder<1, 2, 1, 0> {
    SimKeyboard::builder([[[macros!(0), macros!(1)]]]).behavior_config(BehaviorConfig {
        keyboard_macros: MACROS,
        ..Default::default()
    })
}

/// A `DynamicKeymapMacroGetBuffer`/`SetBuffer` report for `size` bytes at `offset`.
fn macro_buffer(cmd: ViaCommand, offset: u16, payload: &[u8]) -> [u8; REPORT] {
    let mut request = via(cmd);
    request[1..3].copy_from_slice(&offset.to_be_bytes());
    request[3] = payload.len() as u8;
    request[4..4 + payload.len()].copy_from_slice(payload);
    request
}

/// vial-gui asks the buffer size, then reads the buffer in Vial's own encoding:
/// every macro rendered and `0x00`-terminated, an op Vial cannot spell left
/// out, the rest zero.
#[test]
fn macro_buffer_renders_the_macros() {
    test_block_on(async {
        let mut keyboard = macro_keyboard().build().await;
        let mut size = via(ViaCommand::DynamicKeymapMacroGetBufferSize);
        size[1..3].copy_from_slice(&(MACRO_SPACE_SIZE as u16).to_be_bytes());
        keyboard.host_exchange(via(ViaCommand::DynamicKeymapMacroGetBufferSize), size);
        let request = macro_buffer(ViaCommand::DynamicKeymapMacroGetBuffer, 0, &[0; 28]);
        let reply = macro_buffer(
            ViaCommand::DynamicKeymapMacroGetBuffer,
            0,
            &[
                0x01, 0x01, 0x04, 0x01, 0x01, 0x05, 0x00, // tap A, tap B: the pause left out
                b'h', b'i', 0x00, // hi
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        );
        keyboard.host_exchange(request, reply);
        keyboard.run().await;
    });
}

/// Save `data` from offset 0 in 28-byte packets, the way vial-gui does.
fn save_macro_buffer(keyboard: &mut SimKeyboard, data: &[u8]) {
    for (i, chunk) in data.chunks(28).enumerate() {
        keyboard.echo(macro_buffer(
            ViaCommand::DynamicKeymapMacroSetBuffer,
            i as u16 * 28,
            chunk,
        ));
    }
}

/// A slot Vial changed plays what Vial saved, storage or not: slot 0 becomes
/// `tap C` without a pause, so the release does nothing.
#[test]
fn a_changed_slot_plays_what_vial_saved() {
    test_block_on(async {
        let mut keyboard = macro_keyboard().build().await;
        let mut data = vec![0x01, 0x01, 0x06, 0x00, b'h', b'i', 0x00];
        data.resize(data.len() + MACRO_MAX_NUM - 2, 0);
        save_macro_buffer(&mut keyboard, &data);
        keyboard
            .press(0, 0)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .release(0, 0)
            .expect_no_report(50)
            .run()
            .await;
    });
}

/// A macro index past the slots plays nothing, even when a shorter save left
/// the longer one's tail behind in the buffer.
#[test]
fn a_macro_past_the_slots_plays_nothing() {
    test_block_on(async {
        let past = KeyAction::Single(Action::TriggerMacro(MACRO_MAX_NUM as u8));
        let mut keyboard = SimKeyboard::builder([[[k!(A), past]]]).build().await;
        let mut long = vec![b'a'; 64];
        long.resize(long.len() + MACRO_MAX_NUM, 0);
        save_macro_buffer(&mut keyboard, &long);
        let mut short = vec![b'b'];
        short.resize(short.len() + MACRO_MAX_NUM, 0);
        save_macro_buffer(&mut keyboard, &short);
        keyboard.tap(0, 1, 10).expect_no_report(200).run().await;
    });
}

/// Save [`MACROS`] the way vial-gui does, with slot 1 changed to `yo`: every
/// macro from offset 0, each `0x00`-terminated, without padding, the last
/// packet sent twice when `resend_last`. The packets are 8 bytes rather than
/// vial-gui's 28 so that the resent one lands past offset 0.
#[cfg(feature = "storage")]
async fn save_from_vial_gui(resend_last: bool) -> (SimKeyboard, crate::simulator::Flash) {
    let flash = crate::simulator::Flash::new();
    let mut keyboard = macro_keyboard().build_with_flash(flash.clone()).await;
    // Slot 0 is sent back as read, slot 1 becomes `yo`, the rest are empty.
    let mut data = vec![0x01, 0x01, 0x04, 0x01, 0x01, 0x05, 0x00, b'y', b'o', 0x00];
    data.resize(data.len() + MACRO_MAX_NUM - 2, 0);
    let packets: Vec<_> = data
        .chunks(8)
        .enumerate()
        .map(|(i, chunk)| macro_buffer(ViaCommand::DynamicKeymapMacroSetBuffer, i as u16 * 8, chunk))
        .collect();
    for packet in &packets {
        keyboard.echo(*packet);
    }
    if resend_last {
        keyboard.echo(packets[1]);
    }
    keyboard.run().await;
    (keyboard, flash)
}

/// A resent packet writes nothing. The slot Vial sent back as read keeps its
/// default, pause included, and the saved macro survives a restart.
#[cfg(feature = "storage")]
#[test]
fn macro_save_from_vial_gui_writes_only_what_changed() {
    test_block_on(async {
        let (_, resent) = save_from_vial_gui(true).await;
        // Scoped so the first keyboard's event subscription ends before the restart.
        let flash = {
            let (mut keyboard, flash) = save_from_vial_gui(false).await;
            let first_save = flash.writes();
            assert_eq!(resent.writes(), first_save, "a resent packet writes nothing");
            // Slot 1 becomes `ya`.
            keyboard.echo(macro_buffer(ViaCommand::DynamicKeymapMacroSetBuffer, 8, b"a"));
            keyboard.run().await;
            keyboard
                .press(0, 0)
                .expect_keys([HidKeyCode::A])
                .expect_keys([])
                .release(0, 0)
                .expect_keys([HidKeyCode::B])
                .expect_keys([])
                .tap(0, 1, 10)
                .expect_keys([HidKeyCode::Y])
                .expect_keys([])
                .expect_keys([HidKeyCode::A])
                .expect_keys([])
                .expect_keys([])
                .run()
                .await;
            flash
        };
        let mut keyboard = macro_keyboard().build_with_flash(flash).await;
        keyboard
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::Y])
            .expect_keys([])
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .expect_keys([])
            .run()
            .await;
    });
}

/// The first save writes only the chunks with data, like any later change: a
/// chunk flash lacks boots as zeros, so the empty rest of the buffer needs no write.
#[cfg(feature = "storage")]
#[test]
fn a_save_writes_only_the_chunks_with_data() {
    test_block_on(async {
        let flash = crate::simulator::Flash::new();
        let mut keyboard = macro_keyboard().build_with_flash(flash.clone()).await;
        keyboard.run().await;
        let mut writes = Vec::new();
        // Slot 1's `hi` becomes `ho`, then `yo`: a byte of the first chunk each time.
        for (offset, byte) in [(8, b'o'), (7, b'y')] {
            let before = flash.writes();
            keyboard.echo(macro_buffer(ViaCommand::DynamicKeymapMacroSetBuffer, offset, &[byte]));
            keyboard.run().await;
            writes.push(flash.writes() - before);
        }
        assert!(writes[0] > 0);
        assert_eq!(writes[0], writes[1], "the first save writes one chunk, like the next");
    });
}

/// A reset empties every macro and writes the buffer, so a second reset has
/// nothing left to write.
#[cfg(feature = "storage")]
#[test]
fn macro_reset_clears_the_macros() {
    test_block_on(async {
        let flash = crate::simulator::Flash::new();
        let mut keyboard = macro_keyboard().build_with_flash(flash.clone()).await;
        keyboard.echo(via(ViaCommand::DynamicKeymapMacroReset));
        keyboard.tap(0, 0, 10).expect_no_report(50).run().await;
        let cleared = flash.writes();
        assert!(cleared > 0, "the two macros are cleared");
        keyboard.echo(via(ViaCommand::DynamicKeymapMacroReset));
        keyboard.run().await;
        assert_eq!(flash.writes(), cleared, "nothing left to clear");
    });
}

/// vial-gui lets a save spend the whole buffer but one byte a macro, and one
/// macro that long is kept whole.
#[cfg(feature = "storage")]
#[test]
fn the_longest_macro_vial_can_save_is_kept() {
    const TEXT: usize = MACRO_SPACE_SIZE - MACRO_MAX_NUM;
    test_block_on(async {
        let flash = crate::simulator::Flash::new();
        let mut keyboard = macro_keyboard().build_with_flash(flash).await;
        let mut data = vec![b'a'; TEXT];
        data.resize(MACRO_SPACE_SIZE, 0);
        save_macro_buffer(&mut keyboard, &data);
        keyboard.tap(0, 0, 10);
        for _ in 0..TEXT {
            keyboard.expect_keys([HidKeyCode::A]).expect_keys([]);
        }
        keyboard.expect_keys([]).run().await;
    });
}

#[cfg(feature = "storage")]
#[test]
fn behavior_write_survives_restart() {
    test_block_on(async {
        let flash = crate::simulator::Flash::new();
        {
            let mut keyboard = SimKeyboard::builder([[[k!(A), k!(B), osm!(ModifierCombination::LSHIFT)]]])
                .build_with_flash(flash.clone())
                .await;
            keyboard.set_behavior(SettingKey::ComboTimeout, 80);
            keyboard.set_behavior(SettingKey::OneShotTimeout, 80);
            keyboard.set_combo(0, [k!(A), k!(B)], k!(C));
            keyboard.run().await;
        }
        let mut keyboard = SimKeyboard::builder([[[k!(A), k!(B), osm!(ModifierCombination::LSHIFT)]]])
            .build_with_flash(flash)
            .await;
        keyboard.expect_behavior(SettingKey::OneShotTimeout, 80);
        keyboard
            .press(0, 0)
            .expect_no_report(60)
            .expect_keys([HidKeyCode::A])
            .release(0, 0)
            .expect_keys([])
            .tap(0, 2, 10)
            .expect_keys_with_mods(ModifierCombination::LSHIFT.into_bits(), [])
            .delay(120)
            .expect_keys([])
            .run()
            .await;
    });
}

/// QMK's `TT(1)`, as Vial sends it.
const TT1: u16 = 0x52C1;

/// `TT(1)` at (0,0) after Vial writes it, B at (0,1) with C on layer 1.
async fn tt_keyboard() -> SimKeyboard {
    let mut keyboard = SimKeyboard::builder([[[k!(A), k!(B)]], [[rmk::a!(Transparent), k!(C)]]])
        .build()
        .await;
    let mut request = via(ViaCommand::DynamicKeymapSetKeyCode);
    request[1..4].copy_from_slice(&[0, 0, 0]);
    request[4..6].copy_from_slice(&TT1.to_be_bytes());
    keyboard.echo(request);
    keyboard
}

/// The exchange `elimkeys-tt-repro.py` makes: write `TT(1)`, read it back.
#[test]
fn tt_write_reads_back_as_tt() {
    test_block_on(async {
        let mut keyboard = tt_keyboard().await;
        let mut request = via(ViaCommand::DynamicKeymapGetKeyCode);
        request[1..4].copy_from_slice(&[0, 0, 0]);
        let mut reply = request;
        reply[4..6].copy_from_slice(&TT1.to_be_bytes());
        keyboard.host_exchange(request, reply);
        keyboard.run().await;
    });
}

/// A tap toggles layer 1 on, a second tap toggles it off.
#[test]
fn tt_tap_toggles_the_layer() {
    test_block_on(async {
        let mut keyboard = tt_keyboard().await;
        keyboard
            .tap(0, 0, 10)
            .delay(300)
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .tap(0, 0, 10)
            .delay(300)
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::B])
            .expect_keys([])
            .run()
            .await;
    });
}

/// A hold turns layer 1 on only while held.
#[test]
fn tt_hold_is_momentary() {
    test_block_on(async {
        let mut keyboard = tt_keyboard().await;
        keyboard
            .press(0, 0)
            .delay(300)
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .release(0, 0)
            .delay(50)
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::B])
            .expect_keys([])
            .run()
            .await;
    });
}

/// `TT(1)` written over Vial is still `TT(1)` after a restart.
#[cfg(feature = "storage")]
#[test]
fn tt_write_survives_restart() {
    test_block_on(async {
        let keymap = [[[k!(A), k!(B)]], [[rmk::a!(Transparent), k!(C)]]];
        let flash = crate::simulator::Flash::new();
        {
            let mut keyboard = SimKeyboard::builder(keymap).build_with_flash(flash.clone()).await;
            let mut request = via(ViaCommand::DynamicKeymapSetKeyCode);
            request[1..4].copy_from_slice(&[0, 0, 0]);
            request[4..6].copy_from_slice(&TT1.to_be_bytes());
            keyboard.echo(request);
            keyboard.run().await;
        }
        let mut keyboard = SimKeyboard::builder(keymap).build_with_flash(flash).await;
        let mut request = via(ViaCommand::DynamicKeymapGetKeyCode);
        request[1..4].copy_from_slice(&[0, 0, 0]);
        let mut reply = request;
        reply[4..6].copy_from_slice(&TT1.to_be_bytes());
        keyboard.host_exchange(request, reply);
        keyboard
            .tap(0, 0, 10)
            .delay(300)
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .run()
            .await;
    });
}

#[test]
fn high_layer_momentary_write_activates_the_requested_layer() {
    test_block_on(async {
        let mut keymap = [[[rmk::a!(Transparent), k!(B)]]; 32];
        keymap[16][0][1] = k!(C);
        let mut keyboard = SimKeyboard::builder(keymap).build().await;
        let mut request = via(ViaCommand::DynamicKeymapSetKeyCode);
        request[4..6].copy_from_slice(&0x5230u16.to_be_bytes()); // MO(16)
        keyboard.echo(request);
        keyboard
            .press(0, 0)
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .release(0, 0)
            .tap(0, 1, 10)
            .expect_keys([HidKeyCode::B])
            .expect_keys([]);
        request[0] = ViaCommand::DynamicKeymapGetKeyCode as u8;
        keyboard.echo(request);
        keyboard.run().await;
    });
}
