//! Via/Vial host exchanges and their live-keyboard integration tests.

use rmk::k;
use rmk::test_support::{test_block_on, to_via_keycode};
use rmk::types::action::{EncoderAction, KeyAction};
use rmk::types::keycode::HidKeyCode;
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

#[cfg(feature = "storage")]
#[test]
fn behavior_write_survives_restart() {
    test_block_on(async {
        let flash = crate::simulator::flash::InMemoryFlash::new();
        {
            let mut keyboard = SimKeyboard::builder([[[k!(A), k!(B)]]])
                .build_with_flash(flash.clone())
                .await;
            keyboard.set_behavior(SettingKey::ComboTimeout, 80);
            keyboard.set_combo(0, [k!(A), k!(B)], k!(C));
            keyboard.wait_storage().run().await;
        }
        let mut keyboard = SimKeyboard::builder([[[k!(A), k!(B)]]]).build_with_flash(flash).await;
        keyboard
            .press(0, 0)
            .expect_no_report(60)
            .expect_keys([HidKeyCode::A])
            .release(0, 0)
            .expect_keys([])
            .run()
            .await;
    });
}
