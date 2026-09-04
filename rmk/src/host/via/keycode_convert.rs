use rmk_types::action::{Action, KeyAction, KeyboardAction};
use rmk_types::keycode::{HidKeyCode, KeyCode, SpecialKey};
use rmk_types::modifier::ModifierCombination;

pub(crate) fn to_via_keycode(key_action: KeyAction) -> u16 {
    match key_action {
        KeyAction::No => 0x0000,
        KeyAction::Transparent => 0x0001,
        KeyAction::Single(a) => match a {
            Action::Key(k) => match k {
                KeyCode::Hid(hid_keycode) => hid_keycode as u16,
                // Consumer and SystemControl keys are automatically converted to HID keycodes
                // which are natively supported in VIA protocol
                KeyCode::Consumer(consumer_key) => {
                    if let Some(hid_keycode) = consumer_key.to_hid_keycode() {
                        hid_keycode as u16
                    } else {
                        warn!(
                            "Consumer key {:?} has no corresponding HID keycode for VIA",
                            consumer_key
                        );
                        0
                    }
                }
                KeyCode::SystemControl(system_key) => {
                    if let Some(hid_keycode) = system_key.to_hid_keycode() {
                        hid_keycode as u16
                    } else {
                        warn!(
                            "SystemControl key {:?} has no corresponding HID keycode for VIA",
                            system_key
                        );
                        0
                    }
                }
                _ => {
                    warn!("KeyCode variant {:?} not supported by via", k);
                    0
                }
            },
            Action::KeyWithModifier(k, m) => ((m.into_packed_bits() as u16) << 8) | k as u16,
            Action::LayerToggleOnly(l) => 0x5200 | l as u16,
            Action::LayerOn(l) => 0x5220 | l as u16,
            Action::DefaultLayer(l) => 0x5240 | l as u16,
            Action::PersistentDefaultLayer(l) => 0x52E0 | l as u16,
            Action::LayerToggle(l) => 0x5260 | l as u16,
            Action::TriLayerLower => 0x7c77,
            Action::TriLayerUpper => 0x7c78,
            Action::TriggerMacro(idx) => {
                // if idx < 32 {
                0x7700 + (idx as u16)
                // } else {
                // 0x0
                // }
            }
            Action::LayerOnWithModifier(l, m) => {
                if l < 16 {
                    0x5000 | ((l as u16) << 5) | ((m.into_packed_bits() & 0b11111) as u16)
                } else {
                    0
                }
            }
            Action::KeyboardControl(c) => match c {
                KeyboardAction::Bootloader => 0x7c00,
                KeyboardAction::Reboot => 0x7c01,
                KeyboardAction::ClearEeprom => 0x7c03,
                KeyboardAction::ComboOn => 0x7c50,
                KeyboardAction::ComboOff => 0x7c51,
                KeyboardAction::ComboToggle => 0x7c52,
                KeyboardAction::CapsWordToggle => 0x7c73,
                _ => {
                    warn!("KeyboardAction: {:?} vial is not supported yet", c);
                    0
                }
            },
            Action::Special(special_key) => match special_key {
                SpecialKey::GraveEscape => 0x7c16,
                SpecialKey::Repeat => 0x7c79,
                _ => {
                    warn!("SpecialKey variant {:?} not supported by via", special_key);
                    0
                }
            },
            Action::User(id) => (id as u16 & 0x1F) | 0x7E00,
            _ => {
                warn!("Action: {:?} in vial is not supported yet", a);
                0
            }
        },
        KeyAction::Sticky(action, profile) => {
            if profile != u8::MAX {
                warn!("Profiled Sticky Key actions are not representable in VIA");
                return 0;
            }
            match action {
                Action::LayerOn(layer) if layer < 16 => 0x5280 | layer as u16,
                Action::Modifier(modifiers) => 0x52A0 | modifiers.into_packed_bits() as u16,
                _ => {
                    warn!("Sticky Key action {:?} is not representable in VIA", action);
                    0
                }
            }
        }
        KeyAction::Tap(_) => {
            warn!("Tap action is not supported by via");
            0
        }
        KeyAction::TapHold(tap, hold, _) => match hold {
            Action::LayerOn(l) => {
                if l > 16 {
                    0
                } else {
                    let keycode = match tap {
                        Action::Key(KeyCode::Hid(k)) => k as u16,
                        _ => 0,
                    };
                    0x4000 | ((l as u16) << 8) | keycode
                }
            }
            Action::Modifier(m) => {
                match tap {
                    Action::KeyWithModifier(k, sm) if sm == ModifierCombination::LSHIFT => match k {
                        // Space cadet keys
                        HidKeyCode::Kc9 if m == ModifierCombination::LCTRL => 0x7C18,
                        HidKeyCode::Kc0 if m == ModifierCombination::RCTRL => 0x7C19,
                        HidKeyCode::Kc9 if m == ModifierCombination::LSHIFT => 0x7C1A,
                        HidKeyCode::Kc0 if m == ModifierCombination::RSHIFT => 0x7C1B,
                        HidKeyCode::Kc9 if m == ModifierCombination::LALT => 0x7C1C,
                        HidKeyCode::Kc0 if m == ModifierCombination::RALT => 0x7C1D,
                        _ => 0,
                    },
                    Action::Key(KeyCode::Hid(k)) => 0x2000 | ((m.into_packed_bits() as u16) << 8) | (k as u16),
                    _ => 0,
                }
            }
            _ => 0x0000,
        },
        KeyAction::Morse(index) => {
            // Tap dance keycodes: 0x5700..=0x57FF
            0x5700 | (index as u16)
        }
        _ => {
            warn!("KeyAction variant {:?} not supported by via", key_action);
            0
        }
    }
}

/// Convert via keycode to KeyAction.
pub(crate) fn from_via_keycode(via_keycode: u16) -> KeyAction {
    match via_keycode {
        0x0000 => KeyAction::No,
        0x0001 => KeyAction::Transparent,
        0x0002..=0x00FF => KeyAction::Single(Action::Key(KeyCode::Hid((via_keycode as u8).into()))),
        0x0100..=0x1FFF => {
            // WithModifier. Modifiers only apply to HID keyboard keys, so the key narrows to HidKeyCode.
            let modifier = ModifierCombination::from_packed_bits((via_keycode >> 8) as u8);
            KeyAction::Single(Action::KeyWithModifier((via_keycode as u8).into(), modifier))
        }
        0x2000..=0x3FFF => {
            // Modifier tap-hold.
            let keycode = KeyCode::Hid((via_keycode as u8).into());
            let modifier = ModifierCombination::from_packed_bits(((via_keycode >> 8) & 0b11111) as u8);
            KeyAction::TapHold(Action::Key(keycode), Action::Modifier(modifier), u8::MAX)
        }
        0x4000..=0x4FFF => {
            // Layer tap-hold.
            let layer = (via_keycode >> 8) & 0xF;
            let keycode = KeyCode::Hid((via_keycode as u8).into());
            KeyAction::TapHold(Action::Key(keycode), Action::LayerOn(layer as u8), u8::MAX)
        }
        0x5000..=0x51FF => {
            let layer = (via_keycode >> 5) & 0xF;
            let modifier = ModifierCombination::from_packed_bits((via_keycode & 0b11111) as u8);
            KeyAction::Single(Action::LayerOnWithModifier(layer as u8, modifier))
        }
        0x5200..=0x521F => {
            // Activate layer X and deactivate other layers(except default layer)
            let layer = via_keycode as u8 & 0x0F;
            KeyAction::Single(Action::LayerToggleOnly(layer))
        }
        0x5220..=0x523F => {
            // Layer activate
            let layer = via_keycode as u8 & 0x0F;
            KeyAction::Single(Action::LayerOn(layer))
        }
        0x5240..=0x525F => {
            // Set default layer
            let layer = via_keycode as u8 & 0x0F;
            KeyAction::Single(Action::DefaultLayer(layer))
        }
        0x5260..=0x527F => {
            // Layer toggle
            let layer = via_keycode as u8 & 0x0F;
            KeyAction::Single(Action::LayerToggle(layer))
        }
        0x5280..=0x529F => {
            // One-shot layer
            let layer = via_keycode as u8 & 0xF;
            KeyAction::Sticky(Action::LayerOn(layer), u8::MAX)
        }
        0x52A0..=0x52BF => {
            // One-shot modifier
            let m = ModifierCombination::from_packed_bits((via_keycode & 0x1F) as u8);
            KeyAction::Sticky(Action::Modifier(m), u8::MAX)
        }
        0x52C0..=0x52DF => {
            // TODO: Layer tap toggle
            warn!("Layer tap toggle {:#X} not supported", via_keycode);
            KeyAction::No
        }
        0x52E0..=0x52FF => {
            // Persistent default layer (PDF)
            let layer = via_keycode as u8 & 0x0F;
            KeyAction::Single(Action::PersistentDefaultLayer(layer))
        }
        0x5700..=0x57FF => {
            // Tap dance
            let index = (via_keycode & 0xFF) as u8;
            KeyAction::Morse(index)
        }
        0x7000..=0x701F => {
            // TODO: QMK functions, such as swap ctrl/caps, gui on, haptic, music, clicky, combo, RGB, etc
            warn!("QMK functions {:#X} not supported", via_keycode);
            KeyAction::No
        }
        0x7700..=0x771F => {
            // Macro
            let id = via_keycode as u8 & 0x1F;
            KeyAction::Single(Action::TriggerMacro(id))
        }
        0x7800..=0x783F => {
            // TODO: backlight and rgb configuration
            warn!("Backlight and RGB configuration key not supported");
            KeyAction::No
        }
        0x7C00 => KeyAction::Single(Action::KeyboardControl(KeyboardAction::Bootloader)),
        0x7C01 => KeyAction::Single(Action::KeyboardControl(KeyboardAction::Reboot)),
        0x7C03 => KeyAction::Single(Action::KeyboardControl(KeyboardAction::ClearEeprom)),
        0x7C50 => KeyAction::Single(Action::KeyboardControl(KeyboardAction::ComboOn)),
        0x7C51 => KeyAction::Single(Action::KeyboardControl(KeyboardAction::ComboOff)),
        0x7C52 => KeyAction::Single(Action::KeyboardControl(KeyboardAction::ComboToggle)),
        0x7C16 => KeyAction::Single(Action::Special(SpecialKey::GraveEscape)),
        0x7C73 => KeyAction::Single(Action::KeyboardControl(KeyboardAction::CapsWordToggle)),
        0x7C77 => KeyAction::Single(Action::TriLayerLower),
        0x7C78 => KeyAction::Single(Action::TriLayerUpper),
        0x7C79 => KeyAction::Single(Action::Special(SpecialKey::Repeat)),
        0x7C18 => KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::LCTRL),
            u8::MAX,
        ),
        0x7C19 => KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::RCTRL),
            u8::MAX,
        ),
        0x7C1A => KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::LSHIFT),
            u8::MAX,
        ),
        0x7C1B => KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::RSHIFT),
            u8::MAX,
        ),
        0x7C1C => KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::LALT),
            u8::MAX,
        ),
        0x7C1D => KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::RALT),
            u8::MAX,
        ),
        // RS Enter (SC_SENT): decode only. Re-encodes as the equivalent RShift mod-tap (0x3228).
        0x7C1E => KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::Enter)),
            Action::Modifier(ModifierCombination::RSHIFT),
            u8::MAX,
        ),
        0x7C02..=0x7C5F => {
            // TODO: Reset/Haptic/Auto shift(AS)/Dynamic macro
            warn!(
                "Reset/Haptic/Auto shift(AS)/Dynamic macro not supported: {:#X}",
                via_keycode
            );
            KeyAction::No
        }
        0x7E00..=0x7E1F => {
            // QK_KB_N, aka UserN
            KeyAction::Single(Action::User(via_keycode as u8 & 0x1F))
        }
        _ => {
            warn!("Via keycode {:#X} is not processed", via_keycode);
            KeyAction::No
        }
    }
}

#[cfg(test)]
mod test {
    use rmk_types::keycode::HidKeyCode;

    use super::*;

    #[test]
    fn test_convert_via_keycode_to_key_action() {
        // A
        let via_keycode = 0x04;
        assert_eq!(
            KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A))),
            from_via_keycode(via_keycode)
        );

        // Right shift
        let via_keycode = 0xE5;
        assert_eq!(
            KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::RShift))),
            from_via_keycode(via_keycode)
        );

        // User0 (QK_KB_0)
        let via_keycode = 0x7E00;
        assert_eq!(KeyAction::Single(Action::User(0)), from_via_keycode(via_keycode));

        // User16 (QK_KB_16) — must not alias to User0
        let via_keycode = 0x7E10;
        assert_eq!(KeyAction::Single(Action::User(16)), from_via_keycode(via_keycode));

        // User31 (QK_KB_31)
        let via_keycode = 0x7E1F;
        assert_eq!(KeyAction::Single(Action::User(31)), from_via_keycode(via_keycode));

        // ClearEeprom (QK_CLEAR_EEPROM)
        let via_keycode = 0x7C03;
        assert_eq!(
            KeyAction::Single(Action::KeyboardControl(KeyboardAction::ClearEeprom)),
            from_via_keycode(via_keycode)
        );

        // Mo(3)
        let via_keycode = 0x5223;
        assert_eq!(KeyAction::Single(Action::LayerOn(3)), from_via_keycode(via_keycode));

        // OSL(3)
        let via_keycode = 0x5283;
        assert_eq!(
            KeyAction::Sticky(Action::LayerOn(3), u8::MAX),
            from_via_keycode(via_keycode)
        );

        // OSM RCtrl
        let via_keycode = 0x52B1;
        assert_eq!(
            KeyAction::Sticky(
                Action::Modifier(ModifierCombination::new_from(true, false, false, false, true)),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // DF(3)
        let via_keycode = 0x5243;
        assert_eq!(
            KeyAction::Single(Action::DefaultLayer(3)),
            from_via_keycode(via_keycode)
        );

        // PDF(0)
        let via_keycode = 0x52E0;
        assert_eq!(
            KeyAction::Single(Action::PersistentDefaultLayer(0)),
            from_via_keycode(via_keycode)
        );

        // PDF(3)
        let via_keycode = 0x52E3;
        assert_eq!(
            KeyAction::Single(Action::PersistentDefaultLayer(3)),
            from_via_keycode(via_keycode)
        );

        // PDF(15)
        let via_keycode = 0x52EF;
        assert_eq!(
            KeyAction::Single(Action::PersistentDefaultLayer(15)),
            from_via_keycode(via_keycode)
        );

        // LCtrl(A) -> WithModifier(A)
        let via_keycode = 0x104;
        assert_eq!(
            KeyAction::Single(Action::KeyWithModifier(
                HidKeyCode::A,
                ModifierCombination::new_from(false, false, false, false, true)
            )),
            from_via_keycode(via_keycode)
        );

        // RCtrl(A) -> WithModifier(A)
        let via_keycode = 0x1104;
        assert_eq!(
            KeyAction::Single(Action::KeyWithModifier(
                HidKeyCode::A,
                ModifierCombination::new_from(true, false, false, false, true)
            )),
            from_via_keycode(via_keycode)
        );

        // Meh(A) -> WithModifier(A)
        let via_keycode = 0x704;
        assert_eq!(
            KeyAction::Single(Action::KeyWithModifier(
                HidKeyCode::A,
                ModifierCombination::new_from(false, false, true, true, true)
            )),
            from_via_keycode(via_keycode)
        );

        // Hypr(A) -> WithModifier(A)
        let via_keycode = 0xF04;
        assert_eq!(
            KeyAction::Single(Action::KeyWithModifier(
                HidKeyCode::A,
                ModifierCombination::new_from(false, true, true, true, true)
            )),
            from_via_keycode(via_keycode)
        );

        // LT0(A) -> LayerTapHold(A, 0)
        let via_keycode = 0x4004;
        assert_eq!(
            KeyAction::TapHold(Action::Key(KeyCode::Hid(HidKeyCode::A)), Action::LayerOn(0), u8::MAX),
            from_via_keycode(via_keycode)
        );

        // LT3(A) -> LayerTapHold(A, 3)
        let via_keycode = 0x4304;
        assert_eq!(
            KeyAction::TapHold(Action::Key(KeyCode::Hid(HidKeyCode::A)), Action::LayerOn(3), u8::MAX),
            from_via_keycode(via_keycode)
        );

        // LSA_T(A) ->
        let via_keycode = 0x2604;
        assert_eq!(
            KeyAction::TapHold(
                Action::Key(KeyCode::Hid(HidKeyCode::A)),
                Action::Modifier(ModifierCombination::new_from(false, false, true, true, false)),
                u8::MAX,
            ), //hrm
            from_via_keycode(via_keycode)
        );

        // RCAG_T(B) ->
        let via_keycode = 0x3D05;
        assert_eq!(
            KeyAction::TapHold(
                Action::Key(KeyCode::Hid(HidKeyCode::B)),
                Action::Modifier(ModifierCombination::new_from(true, true, true, false, true)),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // ALL_T(A) ->
        let via_keycode: u16 = 0x2F04;
        assert_eq!(
            KeyAction::TapHold(
                Action::Key(KeyCode::Hid(HidKeyCode::A)),
                Action::Modifier(ModifierCombination::new_from(false, true, true, true, true)),
                u8::MAX,
            ), //hrm
            from_via_keycode(via_keycode)
        );

        // Meh_T(B) ->
        let via_keycode = 0x2705;
        assert_eq!(
            KeyAction::TapHold(
                Action::Key(KeyCode::Hid(HidKeyCode::B)),
                Action::Modifier(ModifierCombination::new_from(false, false, true, true, true)),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // LM(1, LSHIFT)
        let via_keycode = 0x5022;
        assert_eq!(
            KeyAction::Single(Action::LayerOnWithModifier(1, ModifierCombination::LSHIFT)),
            from_via_keycode(via_keycode)
        );

        // LM(15, RGUI | RCTRL)
        let via_keycode = 0x5039;
        assert_eq!(
            KeyAction::Single(Action::LayerOnWithModifier(
                1,
                ModifierCombination::new().with_right_gui(true).with_right_ctrl(true)
            )),
            from_via_keycode(via_keycode)
        );

        // ComboOff
        let via_keycode = 0x7C51;
        assert_eq!(
            KeyAction::Single(Action::KeyboardControl(KeyboardAction::ComboOff)),
            from_via_keycode(via_keycode)
        );

        // GraveEscape
        let via_keycode = 0x7C16;
        assert_eq!(
            KeyAction::Single(Action::Special(SpecialKey::GraveEscape)),
            from_via_keycode(via_keycode)
        );

        // RepeatKey
        let via_keycode = 0x7C79;
        assert_eq!(
            KeyAction::Single(Action::Special(SpecialKey::Repeat)),
            from_via_keycode(via_keycode)
        );

        // Space Cadet LC( / KC_LCPO
        let via_keycode = 0x7C18;
        assert_eq!(
            KeyAction::TapHold(
                Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
                Action::Modifier(ModifierCombination::LCTRL),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // Space Cadet RC) / KC_RCPC
        let via_keycode = 0x7C19;
        assert_eq!(
            KeyAction::TapHold(
                Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
                Action::Modifier(ModifierCombination::RCTRL),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // Space Cadet LS( / KC_LSPO
        let via_keycode = 0x7C1A;
        assert_eq!(
            KeyAction::TapHold(
                Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
                Action::Modifier(ModifierCombination::LSHIFT),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // Space Cadet RS) / KC_RSPC
        let via_keycode = 0x7C1B;
        assert_eq!(
            KeyAction::TapHold(
                Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
                Action::Modifier(ModifierCombination::RSHIFT),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // Space Cadet LA( / KC_LAPO
        let via_keycode = 0x7C1C;
        assert_eq!(
            KeyAction::TapHold(
                Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
                Action::Modifier(ModifierCombination::LALT),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // Space Cadet RA) / KC_RAPC
        let via_keycode = 0x7C1D;
        assert_eq!(
            KeyAction::TapHold(
                Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
                Action::Modifier(ModifierCombination::RALT),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // Space Cadet RS Enter / KC_SENT (decode only)
        let via_keycode = 0x7C1E;
        assert_eq!(
            KeyAction::TapHold(
                Action::Key(KeyCode::Hid(HidKeyCode::Enter)),
                Action::Modifier(ModifierCombination::RSHIFT),
                u8::MAX,
            ),
            from_via_keycode(via_keycode)
        );

        // Morse(0)
        let via_keycode = 0x5700;
        assert_eq!(KeyAction::Morse(0), from_via_keycode(via_keycode));

        // Morse(5)
        let via_keycode = 0x5705;
        assert_eq!(KeyAction::Morse(5), from_via_keycode(via_keycode));

        // Morse(255)
        let via_keycode = 0x57FF;
        assert_eq!(KeyAction::Morse(255), from_via_keycode(via_keycode));
    }

    #[test]
    fn test_convert_key_action_to_via_keycode() {
        use rmk_types::keycode::HidKeyCode;

        // A
        let a = KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::A)));
        assert_eq!(0x04, to_via_keycode(a));

        // Right shift
        let a = KeyAction::Single(Action::Key(KeyCode::Hid(HidKeyCode::RShift)));
        assert_eq!(0xE5, to_via_keycode(a));

        // Mo(3)
        let a = KeyAction::Single(Action::LayerOn(3));
        assert_eq!(0x5223, to_via_keycode(a));

        // User0 (QK_KB_0)
        let a = KeyAction::Single(Action::User(0));
        assert_eq!(0x7E00, to_via_keycode(a));

        // User16 (QK_KB_16) — must not alias to User0's 0x7E00
        let a = KeyAction::Single(Action::User(16));
        assert_eq!(0x7E10, to_via_keycode(a));

        // User31 (QK_KB_31)
        let a = KeyAction::Single(Action::User(31));
        assert_eq!(0x7E1F, to_via_keycode(a));

        // ClearEeprom (QK_CLEAR_EEPROM)
        let a = KeyAction::Single(Action::KeyboardControl(KeyboardAction::ClearEeprom));
        assert_eq!(0x7C03, to_via_keycode(a));

        // OSL(3)
        let a = KeyAction::Sticky(Action::LayerOn(3), u8::MAX);
        assert_eq!(0x5283, to_via_keycode(a));

        // OSM RCtrl
        let a = KeyAction::Sticky(
            Action::Modifier(ModifierCombination::new_from(true, false, false, false, true)),
            u8::MAX,
        );
        assert_eq!(0x52B1, to_via_keycode(a));

        // VIA has no representation for named Sticky Key profiles.
        let a = KeyAction::Sticky(Action::Modifier(ModifierCombination::LCTRL), 0);
        assert_eq!(0, to_via_keycode(a));

        // Tap-key Sticky Keys cannot be down-converted to OSM without losing
        // their tap key, so they remain deliberately unrepresentable.
        let a = KeyAction::Sticky(
            Action::KeyWithModifier(HidKeyCode::Tab, ModifierCombination::LALT),
            u8::MAX,
        );
        assert_eq!(0, to_via_keycode(a));

        // DF(3)
        let a = KeyAction::Single(Action::DefaultLayer(3));
        assert_eq!(0x5243, to_via_keycode(a));

        // PDF(3)
        let a = KeyAction::Single(Action::PersistentDefaultLayer(3));
        assert_eq!(0x52E3, to_via_keycode(a));

        // PDF(15)
        let a = KeyAction::Single(Action::PersistentDefaultLayer(15));
        assert_eq!(0x52EF, to_via_keycode(a));

        // LCtrl(A) -> WithModifier(A)
        let a = KeyAction::Single(Action::KeyWithModifier(
            HidKeyCode::A,
            ModifierCombination::new_from(false, false, false, false, true),
        ));
        assert_eq!(0x104, to_via_keycode(a));

        // RCtrl(A) -> WithModifier(A)
        let a = KeyAction::Single(Action::KeyWithModifier(
            HidKeyCode::A,
            ModifierCombination::new_from(true, false, false, false, true),
        ));
        assert_eq!(0x1104, to_via_keycode(a));

        // Meh(A) -> WithModifier(A)
        let a = KeyAction::Single(Action::KeyWithModifier(
            HidKeyCode::A,
            ModifierCombination::new_from(false, false, true, true, true),
        ));
        assert_eq!(0x704, to_via_keycode(a));

        // Hypr(A) -> WithModifier(A)
        let a = KeyAction::Single(Action::KeyWithModifier(
            HidKeyCode::A,
            ModifierCombination::new_from(false, true, true, true, true),
        ));
        assert_eq!(0xF04, to_via_keycode(a));

        // LT0(A) -> LayerTapHold(A, 0)
        let a = KeyAction::TapHold(Action::Key(KeyCode::Hid(HidKeyCode::A)), Action::LayerOn(0), u8::MAX);
        assert_eq!(0x4004, to_via_keycode(a));

        // LT3(A) -> LayerTapHold(A, 3)
        let a = KeyAction::TapHold(Action::Key(KeyCode::Hid(HidKeyCode::A)), Action::LayerOn(3), u8::MAX);
        assert_eq!(0x4304, to_via_keycode(a));

        // LSA_T(A) ->
        let a = KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::A)),
            Action::Modifier(ModifierCombination::new_from(false, false, true, true, false)),
            u8::MAX,
        );
        assert_eq!(0x2604, to_via_keycode(a));

        // RCAG_T(A) ->
        let a = KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::A)),
            Action::Modifier(ModifierCombination::new_from(true, true, true, false, true)),
            u8::MAX,
        );
        assert_eq!(0x3D04, to_via_keycode(a));

        // ALL_T(A) ->
        let a = KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::A)),
            Action::Modifier(ModifierCombination::new_from(false, true, true, true, true)),
            u8::MAX,
        );
        assert_eq!(0x2F04, to_via_keycode(a));

        // Meh_T(A) ->
        let a = KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::A)),
            Action::Modifier(ModifierCombination::new_from(false, false, true, true, true)),
            u8::MAX,
        );
        assert_eq!(0x2704, to_via_keycode(a));

        // LM(1, LSHIFT)
        let a = KeyAction::Single(Action::LayerOnWithModifier(1, ModifierCombination::LSHIFT));
        assert_eq!(0x5022, to_via_keycode(a));

        // LM(15, RGUI | RCTRL)
        let a = KeyAction::Single(Action::LayerOnWithModifier(
            1,
            ModifierCombination::new().with_right_gui(true).with_right_ctrl(true),
        ));
        assert_eq!(0x5039, to_via_keycode(a));

        // ComboOff
        let a = KeyAction::Single(Action::KeyboardControl(KeyboardAction::ComboOff));
        assert_eq!(0x7C51, to_via_keycode(a));

        // GraveEscape
        let a = KeyAction::Single(Action::Special(SpecialKey::GraveEscape));
        assert_eq!(0x7C16, to_via_keycode(a));

        // RepeatKey
        let a = KeyAction::Single(Action::Special(SpecialKey::Repeat));
        assert_eq!(0x7C79, to_via_keycode(a));

        // Space Cadet LC( / KC_LCPO
        let a = KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::LCTRL),
            u8::MAX,
        );
        assert_eq!(0x7C18, to_via_keycode(a));

        // Space Cadet RC) / KC_RCPC
        let a = KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::RCTRL),
            u8::MAX,
        );
        assert_eq!(0x7C19, to_via_keycode(a));

        // Space Cadet LS( / KC_LSPO
        let a = KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::LSHIFT),
            u8::MAX,
        );
        assert_eq!(0x7C1A, to_via_keycode(a));

        // Space Cadet RS) / KC_RSPC
        let a = KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::RSHIFT),
            u8::MAX,
        );
        assert_eq!(0x7C1B, to_via_keycode(a));

        // Space Cadet LA( / KC_LAPO
        let a = KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc9, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::LALT),
            u8::MAX,
        );
        assert_eq!(0x7C1C, to_via_keycode(a));

        // Space Cadet RA) / KC_RAPC
        let a = KeyAction::TapHold(
            Action::KeyWithModifier(HidKeyCode::Kc0, ModifierCombination::LSHIFT),
            Action::Modifier(ModifierCombination::RALT),
            u8::MAX,
        );
        assert_eq!(0x7C1D, to_via_keycode(a));

        // RS Enter (SC_SENT) shares its internal action with RSFT_T(Enter), so it re-encodes to 0x3228.
        let a = KeyAction::TapHold(
            Action::Key(KeyCode::Hid(HidKeyCode::Enter)),
            Action::Modifier(ModifierCombination::RSHIFT),
            u8::MAX,
        );
        assert_eq!(0x3228, to_via_keycode(a));

        // Morse
        let a = KeyAction::Morse(0);
        assert_eq!(0x5700, to_via_keycode(a));

        let a = KeyAction::Morse(5);
        assert_eq!(0x5705, to_via_keycode(a));

        let a = KeyAction::Morse(255);
        assert_eq!(0x57FF, to_via_keycode(a));
    }

    #[test]
    fn test_convert_consumer_system_keys_to_via() {
        use rmk_types::keycode::{ConsumerKey, SystemControlKey};

        // Test Consumer keys conversion
        // AudioMute (ConsumerKey::Mute) -> HidKeyCode::AudioMute (0xA8)
        let a = KeyAction::Single(Action::Key(KeyCode::Consumer(ConsumerKey::Mute)));
        assert_eq!(0xA8, to_via_keycode(a));

        // PlayPause (ConsumerKey::PlayPause) -> HidKeyCode::MediaPlayPause (0xAE)
        let a = KeyAction::Single(Action::Key(KeyCode::Consumer(ConsumerKey::PlayPause)));
        assert_eq!(0xAE, to_via_keycode(a));

        // VolumeIncrement (ConsumerKey::VolumeIncrement) -> HidKeyCode::AudioVolUp (0xA9)
        let a = KeyAction::Single(Action::Key(KeyCode::Consumer(ConsumerKey::VolumeIncrement)));
        assert_eq!(0xA9, to_via_keycode(a));

        // Test SystemControl keys conversion
        // PowerDown (SystemControlKey::PowerDown) -> HidKeyCode::SystemPower (0xA5)
        let a = KeyAction::Single(Action::Key(KeyCode::SystemControl(SystemControlKey::PowerDown)));
        assert_eq!(0xA5, to_via_keycode(a));

        // Sleep (SystemControlKey::Sleep) -> HidKeyCode::SystemSleep (0xA6)
        let a = KeyAction::Single(Action::Key(KeyCode::SystemControl(SystemControlKey::Sleep)));
        assert_eq!(0xA6, to_via_keycode(a));

        // WakeUp (SystemControlKey::WakeUp) -> HidKeyCode::SystemWake (0xA7)
        let a = KeyAction::Single(Action::Key(KeyCode::SystemControl(SystemControlKey::WakeUp)));
        assert_eq!(0xA7, to_via_keycode(a));
    }

    #[test]
    fn test_convert_from_to_ascii_a() {
        use rmk_types::keycode::{HidKeyCode, from_ascii, to_ascii};

        let keycode = HidKeyCode::A;
        let shifted = false;
        let ascii = b'a';

        assert_eq!(to_ascii(keycode, shifted), ascii);
        assert_eq!(from_ascii(ascii), (keycode, shifted));
    }
    #[test]
    fn test_convert_from_to_ascii_K() {
        use rmk_types::keycode::{HidKeyCode, from_ascii, to_ascii};

        let keycode = HidKeyCode::K;
        let shifted = true;
        let ascii = b'K';

        assert_eq!(to_ascii(keycode, shifted), ascii);
        assert_eq!(from_ascii(ascii), (keycode, shifted));
    }
}
