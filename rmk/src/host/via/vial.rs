use byteorder::{BigEndian, ByteOrder, LittleEndian};
use rmk_types::action::KeyAction;
use rmk_types::constants::{COMBO_MAX_LENGTH, COMBO_MAX_NUM, MORSE_MAX_NUM};
use rmk_types::morse::{DOUBLE_TAP, HOLD, HOLD_AFTER_TAP, Morse, MorseMode, TAP};
use rmk_types::protocol::vial::{SettingKey, VIAL_EP_SIZE, VIAL_PROTOCOL_VERSION, VialCommand, VialDynamic};

use crate::config::VialConfig;
use crate::hid::ViaReport;
use crate::host::context::KeyboardContext;
use crate::host::via::keycode_convert::{from_via_keycode, to_via_keycode};

/// Note: vial uses little endian, while via uses big endian
pub(crate) async fn process_vial<'a>(
    report: &mut ViaReport,
    vial_config: &VialConfig<'a>,
    #[cfg(feature = "host_lock")] locker: &crate::host::lock::HostLock<'_>,
    ctx: &KeyboardContext<'_>,
) {
    // report.output_data[0] == 0xFE -> vial commands
    let vial_command = report.output_data[1].into();
    debug!("Received vial command: {:?}", vial_command);
    match vial_command {
        VialCommand::GetKeyboardId => {
            // Returns vial protocol version + vial keyboard id
            LittleEndian::write_u32(&mut report.input_data[0..4], VIAL_PROTOCOL_VERSION);
            report.input_data[4..12].clone_from_slice(vial_config.vial_keyboard_id);
            debug!("Vial return: {:?}", report.input_data);
        }
        VialCommand::GetSize => {
            LittleEndian::write_u32(&mut report.input_data[0..4], vial_config.vial_keyboard_def.len() as u32);
        }
        VialCommand::GetKeyboardDef => {
            let page = LittleEndian::read_u16(&report.output_data[2..4]) as usize;
            let start = page * VIAL_EP_SIZE;
            let mut end = start + VIAL_EP_SIZE;
            let vial_keyboard_def = &vial_config.vial_keyboard_def;
            if end < start || start >= vial_keyboard_def.len() {
                return;
            }
            if end > vial_keyboard_def.len() {
                end = vial_keyboard_def.len();
            }
            vial_keyboard_def[start..end].iter().enumerate().for_each(|(i, v)| {
                report.input_data[i] = *v;
            });
            debug!(
                "Vial return: page:{} start:{} end: {}, data: {:?}",
                page, start, end, report.input_data
            );
        }
        VialCommand::GetUnlockStatus => {
            // Reset all data to 0xFF(it's required!)
            report.input_data.fill(0xFF);
            #[cfg(feature = "host_lock")]
            {
                // Unlocked
                report.input_data[0] = locker.is_unlocked() as u8;
                // Unlock in progress
                report.input_data[1] = locker.is_unlocking() as u8;
                // Unlock keys
                for (idx, (row, col)) in vial_config.unlock_keys.iter().enumerate() {
                    report.input_data[2 + idx * 2] = *row;
                    report.input_data[3 + idx * 2] = *col;
                }
            }
            #[cfg(not(feature = "host_lock"))]
            {
                // Unlocked
                report.input_data[0] = 1;
                // Unlock in progress
                report.input_data[1] = 0;
                warn!("Vial lock feature is not enabled");
            }
        }
        VialCommand::UnlockStart => {
            #[cfg(feature = "host_lock")]
            locker.unlocking();
            #[cfg(not(feature = "host_lock"))]
            error!("Vial lock feature is not enabled");
        }
        VialCommand::UnlockPoll => {
            #[cfg(feature = "host_lock")]
            {
                locker.unlocking();
                report.input_data[0] = locker.is_unlocked() as u8;
                report.input_data[1] = locker.is_unlocking() as u8;
                report.input_data[2] = locker.check_unlock();
            }
            #[cfg(not(feature = "host_lock"))]
            error!("Vial lock feature is not enabled");
        }
        VialCommand::Lock => {
            #[cfg(feature = "host_lock")]
            locker.lock();
            #[cfg(not(feature = "host_lock"))]
            error!("Vial lock feature is not enabled");
        }
        VialCommand::BehaviorSettingQuery => {
            report.input_data.fill(0xFF);
            let value = u16::from_le_bytes([report.output_data[2], report.output_data[3]]);
            if value <= 8 {
                LittleEndian::write_u16(&mut report.input_data[0..2], 0x02);
                LittleEndian::write_u16(&mut report.input_data[2..4], 0x06);
                LittleEndian::write_u16(&mut report.input_data[4..6], 0x07);
                LittleEndian::write_u16(&mut report.input_data[6..8], 0x12);
                LittleEndian::write_u16(&mut report.input_data[8..10], 0x13);
                LittleEndian::write_u16(&mut report.input_data[10..12], 0x16);
                LittleEndian::write_u16(&mut report.input_data[12..14], 0x17);
                LittleEndian::write_u16(&mut report.input_data[14..16], 0x19);
                LittleEndian::write_u16(&mut report.input_data[16..18], 0x1A);
                LittleEndian::write_u16(&mut report.input_data[18..20], 0x1B);
            }
        }
        VialCommand::GetBehaviorSetting => {
            report.input_data.fill(0xFF);
            let value = u16::from_le_bytes([report.output_data[2], report.output_data[3]]);
            report.input_data[0] = 0;
            match value.into() {
                SettingKey::None => report.input_data[0] = 0xFF,
                SettingKey::ComboTimeout => {
                    let combo_timeout = ctx.combo_timeout().as_millis() as u16;
                    LittleEndian::write_u16(&mut report.input_data[1..3], combo_timeout);
                }
                SettingKey::MorseTimeout => {
                    let tapping_term = ctx.morse_default_profile().hold_timeout_ms().unwrap_or(250);
                    LittleEndian::write_u16(&mut report.input_data[1..3], tapping_term);
                }
                SettingKey::OneShotTimeout => {
                    let one_shot_timeout = ctx.one_shot_timeout().as_millis() as u16;
                    LittleEndian::write_u16(&mut report.input_data[1..3], one_shot_timeout);
                }
                SettingKey::TapInterval => {
                    let tap_interval = ctx.tap_interval();
                    LittleEndian::write_u16(&mut report.input_data[1..3], tap_interval);
                }
                SettingKey::TapCapslockInterval => {
                    let tap_capslock_interval = ctx.tap_capslock_interval();
                    LittleEndian::write_u16(&mut report.input_data[1..3], tap_capslock_interval);
                }
                SettingKey::PermissiveHold => {
                    if let Some(m) = ctx.morse_default_profile().mode()
                        && m == MorseMode::PermissiveHold
                    {
                        report.input_data[1] = 1
                    } else {
                        report.input_data[1] = 0
                    }
                }
                SettingKey::HoldOnOtherKeyPress => {
                    if let Some(m) = ctx.morse_default_profile().mode()
                        && m == MorseMode::HoldOnOtherPress
                    {
                        report.input_data[1] = 1
                    } else {
                        report.input_data[1] = 0
                    }
                }
                SettingKey::QuickTapTerm => {
                    let quick_tap_term = ctx.morse_default_profile().quick_tap_timeout_ms().unwrap_or(0);
                    LittleEndian::write_u16(&mut report.input_data[1..3], quick_tap_term);
                }
                SettingKey::UnilateralTap => {
                    let unilateral_tap = ctx.morse_default_profile().unilateral_tap().unwrap_or(false);
                    if unilateral_tap {
                        report.input_data[1] = 1;
                    } else {
                        report.input_data[1] = 0;
                    };
                }
                SettingKey::PriorIdleTime => {
                    let prior_idle_time = ctx.morse_prior_idle_time().as_millis() as u16;
                    LittleEndian::write_u16(&mut report.input_data[1..3], prior_idle_time);
                }
            }
        }
        VialCommand::SetBehaviorSetting => {
            let key = u16::from_le_bytes([report.output_data[2], report.output_data[3]]);
            match key.into() {
                SettingKey::None => (),
                SettingKey::ComboTimeout => {
                    let combo_timeout = u16::from_le_bytes([report.output_data[4], report.output_data[5]]);
                    ctx.set_combo_timeout(combo_timeout).await;
                }
                SettingKey::MorseTimeout => {
                    let timeout_time = u16::from_le_bytes([report.output_data[4], report.output_data[5]]);
                    let new_profile = ctx.morse_default_profile().with_hold_timeout_ms(Some(timeout_time));
                    ctx.set_morse_default_profile(new_profile).await;
                }
                SettingKey::OneShotTimeout => {
                    let timeout_time = u16::from_le_bytes([report.output_data[4], report.output_data[5]]);
                    ctx.set_one_shot_timeout(timeout_time).await;
                }
                SettingKey::TapInterval => {
                    let tap_interval = u16::from_le_bytes([report.output_data[4], report.output_data[5]]);
                    ctx.set_tap_interval(tap_interval).await;
                }
                SettingKey::TapCapslockInterval => {
                    let tap_capslock_interval = u16::from_le_bytes([report.output_data[4], report.output_data[5]]);
                    ctx.set_tap_capslock_interval(tap_capslock_interval).await;
                }

                SettingKey::PermissiveHold => {
                    let enabled = report.output_data[4] == 1;
                    let old = ctx.morse_default_profile();
                    let new_mode = if enabled {
                        // Hold On Other Key Press has higher priority
                        if old.mode() == Some(MorseMode::HoldOnOtherPress) {
                            old.mode()
                        } else {
                            // Enable: Set to Permissive Hold
                            Some(MorseMode::PermissiveHold)
                        }
                    } else {
                        // Disable: Only set to Normal if currently PermissiveHold
                        if old.mode() == Some(MorseMode::PermissiveHold) {
                            Some(MorseMode::Normal)
                        } else {
                            old.mode() // Keep current mode unchanged
                        }
                    };
                    ctx.set_morse_default_profile(old.with_mode(new_mode)).await;
                }
                SettingKey::HoldOnOtherKeyPress => {
                    let enabled = report.output_data[4] == 1;
                    let old = ctx.morse_default_profile();
                    let new_mode = if enabled {
                        // Enable: Set to HoldOnOtherPress (will override other modes)
                        Some(MorseMode::HoldOnOtherPress)
                    } else {
                        // Disable: Only set to Normal if currently HoldOnOtherPress
                        if old.mode() == Some(MorseMode::HoldOnOtherPress) {
                            Some(MorseMode::Normal)
                        } else {
                            old.mode() // Keep current mode unchanged
                        }
                    };
                    ctx.set_morse_default_profile(old.with_mode(new_mode)).await;
                }
                SettingKey::QuickTapTerm => {
                    let timeout_time = u16::from_le_bytes([report.output_data[4], report.output_data[5]]);
                    let new_profile = ctx
                        .morse_default_profile()
                        .with_quick_tap_timeout_ms(Some(timeout_time));
                    ctx.set_morse_default_profile(new_profile).await;
                }
                SettingKey::UnilateralTap => {
                    let new_profile = ctx
                        .morse_default_profile()
                        .with_unilateral_tap(Some(report.output_data[4] == 1));
                    ctx.set_morse_default_profile(new_profile).await;
                }
                SettingKey::PriorIdleTime => {
                    let prior_idle_time = u16::from_le_bytes([report.output_data[4], report.output_data[5]]);
                    ctx.set_morse_prior_idle_time(prior_idle_time).await;
                }
            }
        }
        VialCommand::DynamicEntryOp => {
            let vial_dynamic = report.output_data[2].into();
            match vial_dynamic {
                VialDynamic::DynamicVialGetNumberOfEntries => {
                    debug!("DynamicEntryOp - DynamicVialGetNumberOfEntries");
                    report.input_data[0] = core::cmp::min(MORSE_MAX_NUM, 255) as u8; // Tap dance entries
                    report.input_data[1] = core::cmp::min(COMBO_MAX_NUM, 255) as u8; // Combo entries
                    // TODO: Support dynamic key override
                    report.input_data[2] = 0; // Key override entries
                    report.input_data[31] = 1 // Enable caps word
                }
                VialDynamic::DynamicVialMorseGet => {
                    debug!("DynamicEntryOp - DynamicVialMorseGet");
                    report.input_data[0] = 0; // Index 0 is the return code, 0 means success

                    let morse_idx = report.output_data[3];
                    if let Some(morse) = ctx.get_morse(morse_idx) {
                        // Pack morse data into report
                        LittleEndian::write_u16(
                            &mut report.input_data[1..3],
                            to_via_keycode(morse.get(TAP).map_or(KeyAction::No, KeyAction::Single)),
                        );
                        LittleEndian::write_u16(
                            &mut report.input_data[3..5],
                            to_via_keycode(morse.get(HOLD).map_or(KeyAction::No, KeyAction::Single)),
                        );
                        LittleEndian::write_u16(
                            &mut report.input_data[5..7],
                            to_via_keycode(morse.get(DOUBLE_TAP).map_or(KeyAction::No, KeyAction::Single)),
                        );
                        LittleEndian::write_u16(
                            &mut report.input_data[7..9],
                            to_via_keycode(morse.get(HOLD_AFTER_TAP).map_or(KeyAction::No, KeyAction::Single)),
                        );
                        let timeout_ms = morse.profile.hold_timeout_ms().unwrap_or(250);
                        LittleEndian::write_u16(&mut report.input_data[9..11], timeout_ms);
                    } else {
                        report.input_data[1..11].fill(0);
                    }
                }
                VialDynamic::DynamicVialMorseSet => {
                    debug!("DynamicEntryOp - DynamicVialMorseSet");
                    report.input_data[0] = 0; // Index 0 is the return code, 0 means success

                    let morse_idx = report.output_data[3];

                    if (morse_idx as usize) < ctx.morses_len() {
                        // Extract morse (also known as "tap dance" in vial)
                        let tap = from_via_keycode(LittleEndian::read_u16(&report.output_data[4..6]));
                        let hold = from_via_keycode(LittleEndian::read_u16(&report.output_data[6..8]));
                        let double_tap = from_via_keycode(LittleEndian::read_u16(&report.output_data[8..10]));
                        let hold_after_tap = from_via_keycode(LittleEndian::read_u16(&report.output_data[10..12]));
                        let timeout_ms = LittleEndian::read_u16(&report.output_data[12..14]);

                        ctx.update_morse(morse_idx, |morse: &mut Morse| {
                            let _ = morse.put(TAP, tap.to_action());
                            let _ = morse.put(DOUBLE_TAP, double_tap.to_action());
                            let _ = morse.put(HOLD, hold.to_action());
                            let _ = morse.put(HOLD_AFTER_TAP, hold_after_tap.to_action());
                            morse.profile.set_hold_timeout_ms(timeout_ms);
                            morse.profile.set_gap_timeout_ms(timeout_ms);
                        })
                        .await;
                    }
                }
                VialDynamic::DynamicVialComboGet => {
                    debug!("DynamicEntryOp - DynamicVialComboGet");
                    report.input_data[0] = 0; // Index 0 is the return code, 0 means success

                    let combo_idx = report.output_data[3] as usize;
                    ctx.with_combos(|combos| {
                        if let Some(Some(combo)) = combos.get(combo_idx) {
                            // Combo components
                            for i in 0..COMBO_MAX_LENGTH {
                                let kc = combo.config.actions.get(i).copied().unwrap_or(KeyAction::No);
                                LittleEndian::write_u16(
                                    &mut report.input_data[1 + i * 2..3 + i * 2],
                                    to_via_keycode(kc),
                                );
                            }
                            // Combo output
                            LittleEndian::write_u16(
                                &mut report.input_data[1 + COMBO_MAX_LENGTH * 2..3 + COMBO_MAX_LENGTH * 2],
                                to_via_keycode(combo.config.output),
                            );
                        } else {
                            report.input_data[1..3 + COMBO_MAX_LENGTH * 2].fill(0);
                        }
                    });
                }
                VialDynamic::DynamicVialComboSet => {
                    debug!("DynamicEntryOp - DynamicVialComboSet");
                    report.input_data[0] = 0; // Index 0 is the return code, 0 means success

                    use rmk_types::combo::Combo as ComboConfig;

                    let combo_idx = report.output_data[3];

                    let mut actions = heapless::Vec::<KeyAction, COMBO_MAX_LENGTH>::new();
                    let mut overflow = false;
                    for i in 0..COMBO_MAX_LENGTH {
                        let action =
                            from_via_keycode(LittleEndian::read_u16(&report.output_data[4 + i * 2..6 + i * 2]));
                        if !action.is_empty() && actions.push(action).is_err() {
                            overflow = true;
                            break;
                        }
                    }
                    if overflow {
                        return;
                    }
                    let output = from_via_keycode(LittleEndian::read_u16(
                        &report.output_data[4 + COMBO_MAX_LENGTH * 2..6 + COMBO_MAX_LENGTH * 2],
                    ));
                    let config = ComboConfig {
                        actions,
                        output,
                        layer: None,
                    };
                    ctx.set_combo(combo_idx, config).await;
                }
                VialDynamic::DynamicVialKeyOverrideGet => {
                    warn!("DynamicEntryOp - DynamicVialKeyOverrideGet -- to be implemented");
                    report.input_data.fill(0x00);
                }
                VialDynamic::DynamicVialKeyOverrideSet => {
                    warn!("DynamicEntryOp - DynamicVialKeyOverrideSet -- to be implemented");
                    report.input_data.fill(0x00);
                }
                VialDynamic::Unhandled => {
                    warn!("DynamicEntryOp - Unhandled -- subcommand not recognized");
                    report.input_data.fill(0x00);
                }
            }
        }
        VialCommand::GetEncoder => {
            let layer = report.output_data[2];
            let index = report.output_data[3];
            debug!("Received Vial - GetEncoder, encoder idx: {} at layer: {}", index, layer);

            // Get encoder value
            if let Some(encoder_action) = ctx.get_encoder(layer, index) {
                let clockwise = to_via_keycode(encoder_action.clockwise);
                let counter_clockwise = to_via_keycode(encoder_action.counter_clockwise);
                BigEndian::write_u16(&mut report.input_data[0..2], counter_clockwise);
                BigEndian::write_u16(&mut report.input_data[2..4], clockwise);
                return;
            }

            // Clear returned value, aka `KeyAction::No`
            report.input_data.fill(0x0);
        }
        VialCommand::SetEncoder => {
            let layer = report.output_data[2];
            let index = report.output_data[3];
            let clockwise = report.output_data[4];
            debug!(
                "Received Vial - SetEncoder, encoder idx: {} clockwise: {} at layer: {}",
                index, clockwise, layer
            );
            let keycode = BigEndian::read_u16(&report.output_data[5..7]);
            let action = from_via_keycode(keycode);
            info!("Setting encoder action (clockwise: {}): {:?}", clockwise, action);
            ctx.set_encoder_direction(layer, index, clockwise == 1, action).await;
        }
        _ => (),
    }
}

#[cfg(test)]
#[cfg(feature = "storage")]
mod tests {
    use rmk_types::action::Action;
    use rmk_types::combo::Combo as ComboConfig;
    use rmk_types::keycode::{HidKeyCode, KeyCode};
    use sequential_storage::map::Value;

    use super::*;
    use crate::COMBO_MAX_LENGTH;
    use crate::storage::StorageData;
    #[test]
    fn test_combo_serialization_deserialization() {
        let mut actions = heapless::Vec::<KeyAction, COMBO_MAX_LENGTH>::new();
        actions
            .push(KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Kc1))))
            .unwrap();
        let combo_config = ComboConfig {
            actions,
            output: KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::Space))),
            layer: None,
        };
        let mut buffer = [0u8; 64];
        let storage_data = StorageData::Combo(combo_config.clone());
        let serialized_size = Value::serialize_into(&storage_data, &mut buffer).unwrap();
        // Deserialization
        let deserialized_data = StorageData::deserialize_from(&buffer[..serialized_size]).unwrap();
        // Validation
        match deserialized_data {
            (StorageData::Combo(deserialized_config), _) => {
                assert_eq!(deserialized_config, combo_config);
            }
            _ => panic!("Expected Combo"),
        }
    }
}
