use serde::Deserialize;

use crate::{DEFAULT_PASSKEY_ENTRY_TIMEOUT_SECS, MIN_PASSKEY_ENTRY_TIMEOUT_SECS, SplitSide};

const SUBSCRIBER_DEFAULT_CONFIG: &str = include_str!("../default_config/subscriber_default.toml");

/// Parsed representation of `subscriber_default.toml`.
#[derive(Deserialize)]
struct SubscriberConfig {
    subscriber: Vec<SubscriberEntry>,
}

/// A single entry: bump `subs` for each listed event when all `features` are enabled.
#[derive(Deserialize)]
struct SubscriberEntry {
    features: Vec<String>,
    events: Vec<SubscriberEventEntry>,
}

/// Per-event subscriber bump. `count` defaults to 1.
#[derive(Deserialize)]
struct SubscriberEventEntry {
    name: String,
    #[serde(default = "default_sub_count")]
    count: usize,
}

fn default_sub_count() -> usize {
    1
}

/// Compile-time constants emitted as `pub const` items by `rmk-types/build.rs`.
pub struct BuildConstants {
    pub max_custom_message_size: usize,
    pub max_combos: usize,
    pub max_combo_keys: usize,
    pub max_forks: usize,
    pub max_morses: usize,
    pub max_morse_profiles: usize,
    pub max_hold_trigger_key_positions: usize,
    pub max_patterns_per_key: usize,
    pub max_macros: usize,
    pub macro_space_size: usize,
    pub debounce_time: u16,
    pub matrix_scan_interval_us: u32,
    pub mouse_key_interval: u16,
    pub mouse_wheel_interval: u16,
    pub report_channel_size: usize,
    pub vial_channel_size: usize,
    pub flash_channel_size: usize,
    pub num_split_peripherals: usize,
    /// Side of the central when both halves expose standard left/right batteries.
    pub paired_battery_central_side: Option<SplitSide>,
    pub central_battery_user_description: String,
    pub split_battery_peripheral_ids: Vec<usize>,
    pub split_battery_peripheral_user_descriptions: Vec<String>,
    /// Host profiles, set by `[rmk] num_ble_profiles`.
    pub num_ble_profiles: usize,
    /// Host profiles plus the dongle profile.
    pub total_ble_profiles: usize,
    /// Simultaneous BLE host and dongle connections: 0 without BLE.
    pub num_host_links: usize,
    pub ble_use_2m_phy: bool,
    pub ble_profile_name_suffix: bool,
    pub split_central_sleep_timeout_seconds: u32,
    pub max_auto_mouse_layers: usize,
    /// Rynk RX/TX buffer size (bytes).
    pub rynk_buffer_size: usize,
    pub dongle_pairing_window_secs: u32,
    pub events: Vec<EventChannel>,
    pub passkey: Option<Passkey>,
}

pub struct EventChannel {
    pub name: String,
    pub channel_size: usize,
    pub pubs: usize,
    pub subs: usize,
}

pub struct Passkey {
    pub enabled: bool,
    pub timeout_secs: u32,
}

impl crate::KeyboardTomlConfig {
    /// PointingDevices on the busiest board: a binary carries one board's devices.
    fn pointing_device_count(&self) -> usize {
        fn count(input_device: Option<&crate::InputDeviceConfig>) -> usize {
            input_device
                .map(|d| d.pmw3610.as_ref().map_or(0, |v| v.len()) + d.pmw33xx.as_ref().map_or(0, |v| v.len()))
                .unwrap_or(0)
        }
        let boards = self.split.iter().flat_map(|split| {
            core::iter::once(split.central.input_device.as_ref())
                .chain(split.peripheral.iter().map(|p| p.input_device.as_ref()))
        });
        core::iter::once(self.input_device.as_ref())
            .chain(boards)
            .map(count)
            .max()
            .unwrap_or(0)
    }

    /// Build compile-time constants from the configuration.
    ///
    /// `active_features` contains feature names enabled on the
    /// **downstream crate** (e.g. `["split", "_ble"]`). These are matched
    /// against `subscriber_default.toml` to auto-bump event subscriber counts.
    pub fn build_constants(&self, active_features: &[&str]) -> Result<BuildConstants, String> {
        let rmk = &self.rmk;

        // Fix num_split_peripherals: when split feature is enabled, ensure at least 1
        let num_split_peripherals = if active_features.contains(&"split") && rmk.num_split_peripherals < 1 {
            1
        } else {
            rmk.num_split_peripherals
        };
        let central_battery = self.resolve_battery_config(None)?;
        let mut pair_side = None;
        let mut split_battery_peripheral_ids = Vec::new();
        let mut split_battery_peripheral_user_descriptions = Vec::new();
        if active_features.contains(&"split")
            && let Some(split) = &self.split
        {
            for id in 0..split.peripheral.len() {
                let battery = self.resolve_battery_config(Some(id))?;
                if battery.battery_adc_pin.is_some() {
                    split_battery_peripheral_ids.push(id);
                    split_battery_peripheral_user_descriptions.push(
                        battery
                            .battery_user_description
                            .unwrap_or_else(|| format!("Peripheral {id}")),
                    );
                }
            }
            pair_side = split.central_side.filter(|_| {
                split.peripheral.len() == 1
                    && !split_battery_peripheral_ids.is_empty()
                    && central_battery.battery_adc_pin.is_some()
            });
        }
        let total_ble_profiles = rmk.num_ble_profiles + usize::from(active_features.contains(&"dongle"));
        if total_ble_profiles == 0 && active_features.contains(&"_ble") {
            return Err("BLE requires at least one BLE profile; set num_ble_profiles > 0 or enable dongle".to_string());
        }
        let ble_profile_name_suffix = self.ble.as_ref().is_some_and(|ble| ble.profile_name_suffix);
        let num_host_links = if active_features.contains(&"_ble") {
            if ble_profile_name_suffix && !active_features.contains(&"_ext_adv") {
                return Err("keyboard.toml: [ble] profile_name_suffix requires the `ext_adv` feature".to_string());
            }
            let links = self.ble.as_ref().and_then(|ble| ble.max_connections).unwrap_or(1);
            if links == 0 || links > total_ble_profiles {
                return Err(format!(
                    "keyboard.toml: [ble.max_connections] must be between 1 and the {total_ble_profiles} BLE profiles, got {links}"
                ));
            }
            // Only extended advertising gives each profile an address, and so a link, of its own.
            if links > 1 && !active_features.contains(&"_ext_adv") {
                return Err("keyboard.toml: [ble.max_connections] above 1 requires the `ext_adv` feature".to_string());
            }
            links
        } else {
            0
        };

        let mut event_config = self.event.clone();
        if num_host_links > 0 {
            // One subscriber maintains the GATT values; each link has its own notification task.
            let battery_subscribers = 1 + num_host_links;
            event_config.battery_status.subs += battery_subscribers;
            if !split_battery_peripheral_ids.is_empty() {
                event_config.peripheral_battery.subs += battery_subscribers;
            }
            // Both the HID writer and the PHY task wait for connection status.
            event_config.connection_status_change.subs += 2 * num_host_links;
        }
        // host/rynk/topics.rs: each host link opens its own Rynk session.
        if active_features.contains(&"rynk") {
            for event in [
                &mut event_config.battery_status,
                &mut event_config.layer_change,
                &mut event_config.wpm_update,
                &mut event_config.connection_status_change,
                &mut event_config.sleep_state,
                &mut event_config.led_indicator,
            ] {
                event.subs += num_host_links;
            }
        }

        // Each PointingDevice subscribes to the sleep state. Devices built by
        // hand in Rust are declared under `[event.sleep_state]` instead.
        event_config.sleep_state.subs += self.pointing_device_count();

        // Every link subscribes to the outgoing queue, so a central needs one
        // slot per split peripheral on top of its link toward the dongle.
        if active_features.contains(&"custom_message") && active_features.contains(&"split") {
            event_config.custom_message_out.subs += num_split_peripherals;
        }

        // Dynamically size dfu_cmd subscribers: 1 (central) + N (peripherals).
        // The base count of 1 covers the central; each peripheral adds one.
        if active_features.contains(&"dfu_split") {
            event_config.dfu_cmd.subs += num_split_peripherals;
            event_config.dfu_cmd.pubs += 1; // Split-Loop as second publisher (USB-Proxy is first)
        }

        // Build event channels
        macro_rules! event_channels {
            ($($field:ident),* $(,)?) => {
                vec![$(
                    EventChannel {
                        name: stringify!($field).to_string(),
                        channel_size: event_config.$field.channel_size,
                        pubs: event_config.$field.pubs,
                        subs: event_config.$field.subs,
                    },
                )*]
            };
        }

        let mut events = event_channels!(
            connection_status_change,
            modifier,
            keyboard,
            layer_change,
            wpm_update,
            led_indicator,
            sleep_state,
            battery_status,
            battery_adc,
            charging_state,
            pointing,
            touch,
            peripheral_connected,
            central_connected,
            peripheral_battery,
            clear_peer,
            dongle_state,
            dfu_status,
            dfu_cmd,
            action,
            custom_message,
            custom_message_out,
        );

        // Auto-bump subscriber counts based on enabled feature flags.
        // Declarations live in subscriber_default.toml.
        apply_feature_subscriber_bumps(&mut events, active_features);

        // Only validate passkey settings when the build will emit passkey constants.
        let passkey = if active_features.contains(&"passkey_entry") {
            self.ble.as_ref().map(resolve_passkey_enabled).transpose()?
        } else {
            None
        };

        // Validate that config values do not exceed protocol ceilings.
        use crate::protocol_limits;
        if rmk.max_combo_keys > protocol_limits::MAX_COMBO_SIZE {
            return Err(format!(
                "max_combo_keys ({}) exceeds protocol ceiling MAX_COMBO_SIZE ({})",
                rmk.max_combo_keys,
                protocol_limits::MAX_COMBO_SIZE
            ));
        }
        if rmk.max_patterns_per_key > protocol_limits::MAX_MORSE_SIZE {
            return Err(format!(
                "max_patterns_per_key ({}) exceeds protocol ceiling MAX_MORSE_SIZE ({})",
                rmk.max_patterns_per_key,
                protocol_limits::MAX_MORSE_SIZE
            ));
        }
        if rmk.macro_space_size > protocol_limits::MAX_MACRO_SPACE_SIZE {
            return Err(format!(
                "macro_space_size ({}) exceeds protocol ceiling MAX_MACRO_SPACE_SIZE ({})",
                rmk.macro_space_size,
                protocol_limits::MAX_MACRO_SPACE_SIZE
            ));
        }
        let max_auto_mouse_layers = rmk
            .max_auto_mouse_layers
            .unwrap_or(crate::resolved::behavior::DEFAULT_MAX_AUTO_MOUSE_LAYERS);
        if let Some(entries) = self.behavior.as_ref().and_then(|b| b.auto_mouse_layer.as_ref()) {
            if entries.len() > max_auto_mouse_layers {
                return Err(format!(
                    "number of [[behavior.auto_mouse_layer]] entries ({}) exceeds max_auto_mouse_layers ({})",
                    entries.len(),
                    max_auto_mouse_layers
                ));
            }
            let uses_action_event = entries
                .iter()
                .any(|e| e.deactivate_on_key == Some(true) || e.reset_timeout_on_key == Some(true));
            if uses_action_event && events.iter().any(|e| e.name == "action" && e.subs == 0) {
                return Err(
                    "[[behavior.auto_mouse_layer]].deactivate_on_key / reset_timeout_on_key require [event.action] subs to be at least 1".to_string(),
                );
            }
        }

        // Host capability fields are u8/u16 on the wire; check the values no deserializer bound
        // covers (max_morses and num_split_peripherals can also be auto-raised past 255).
        validate_u8_capability("max_morses", rmk.max_morses)?;
        validate_u8_capability("num_split_peripherals", num_split_peripherals)?;
        validate_u8_capability("num_ble_profiles", rmk.num_ble_profiles)?;
        validate_u8_capability("max_connections", num_host_links)?;
        validate_u8_capability("max_macros", rmk.max_macros)?;
        validate_u16_capability("macro_space_size", rmk.macro_space_size)?;
        validate_u16_capability("rynk_buffer_size", rmk.rynk_buffer_size)?;
        Ok(BuildConstants {
            max_custom_message_size: rmk.max_custom_message_size,
            max_combos: rmk.max_combos,
            max_combo_keys: rmk.max_combo_keys,
            max_forks: rmk.max_forks,
            max_morses: rmk.max_morses,
            max_morse_profiles: rmk.max_morse_profiles,
            max_hold_trigger_key_positions: rmk.max_hold_trigger_key_positions,
            max_patterns_per_key: rmk.max_patterns_per_key,
            max_macros: rmk.max_macros,
            macro_space_size: rmk.macro_space_size,
            debounce_time: rmk.debounce_time,
            matrix_scan_interval_us: rmk.matrix_scan_interval_us,
            mouse_key_interval: rmk.mouse_key_interval,
            mouse_wheel_interval: rmk.mouse_wheel_interval,
            report_channel_size: rmk.report_channel_size,
            vial_channel_size: rmk.vial_channel_size,
            flash_channel_size: rmk.flash_channel_size,
            num_split_peripherals,
            paired_battery_central_side: pair_side,
            central_battery_user_description: central_battery
                .battery_user_description
                .unwrap_or_else(|| "Central".into()),
            split_battery_peripheral_ids,
            split_battery_peripheral_user_descriptions,
            num_ble_profiles: rmk.num_ble_profiles,
            total_ble_profiles,
            num_host_links,
            ble_use_2m_phy: self.ble.as_ref().and_then(|ble| ble.use_2m_phy).unwrap_or(true),
            ble_profile_name_suffix,
            split_central_sleep_timeout_seconds: rmk.split_central_sleep_timeout_seconds,
            max_auto_mouse_layers,
            rynk_buffer_size: rmk.rynk_buffer_size,
            dongle_pairing_window_secs: rmk.dongle_pairing_window_secs,
            events,
            passkey,
        })
    }
}

fn validate_u8_capability(name: &str, value: usize) -> Result<(), String> {
    if value > u8::MAX as usize {
        return Err(format!(
            "{name} ({value}) exceeds the u8 host capability field (max 255)"
        ));
    }
    Ok(())
}

fn validate_u16_capability(name: &str, value: usize) -> Result<(), String> {
    if value > u16::MAX as usize {
        return Err(format!(
            "{name} ({value}) exceeds the u16 host capability field (max 65535)"
        ));
    }
    Ok(())
}

/// Bump event subscriber counts based on feature flags declared in `subscriber_default.toml`.
///
/// `active_features` contains lowercase feature names (e.g. `"split"`, `"_ble"`).
fn apply_feature_subscriber_bumps(events: &mut [EventChannel], active_features: &[&str]) {
    let sub_config: SubscriberConfig =
        toml::from_str(SUBSCRIBER_DEFAULT_CONFIG).expect("Failed to parse subscriber_default.toml");

    for entry in &sub_config.subscriber {
        let all_enabled = entry.features.iter().all(|f| active_features.contains(&f.as_str()));
        if all_enabled {
            for sub_event in &entry.events {
                if let Some(event) = events.iter_mut().find(|e| e.name == sub_event.name) {
                    event.subs += sub_event.count;
                } else {
                    println!(
                        "cargo:warning=subscriber_default.toml: unknown event \"{}\"",
                        sub_event.name
                    );
                }
            }
        }
    }
}

fn resolve_passkey_enabled(ble: &crate::BleConfig) -> Result<Passkey, String> {
    let enabled = ble.passkey_entry.unwrap_or(false);
    let timeout_secs = ble.passkey_entry_timeout.unwrap_or(DEFAULT_PASSKEY_ENTRY_TIMEOUT_SECS);
    if timeout_secs < MIN_PASSKEY_ENTRY_TIMEOUT_SECS {
        return Err(format!(
            "keyboard.toml: [ble.passkey_entry_timeout] must be at least {} seconds, got {}",
            MIN_PASSKEY_ENTRY_TIMEOUT_SECS, timeout_secs
        ));
    }
    Ok(Passkey { enabled, timeout_secs })
}

#[cfg(test)]
mod tests {
    use super::{BuildConstants, resolve_passkey_enabled, validate_u8_capability, validate_u16_capability};
    use crate::battery::BatteryTomlConfig;
    use crate::{
        BleConfig, DEFAULT_PASSKEY_ENTRY_TIMEOUT_SECS, KeyboardTomlConfig, MIN_PASSKEY_ENTRY_TIMEOUT_SECS,
        SplitBoardConfig, SplitConfig, SplitSide,
    };

    #[test]
    fn reserves_battery_subscribers_without_requiring_ble() {
        // The LED processor, split forwarding, and each Rynk session can coexist.
        let config: KeyboardTomlConfig = toml::from_str("").unwrap();
        for (features, expected) in [
            (vec![], 1),
            (vec!["split"], 2),
            (vec!["rynk"], 2),
            (vec!["split", "rynk"], 3),
            (vec!["split", "rynk", "_ble"], 6),
        ] {
            let constants = config.build_constants(&features).unwrap();
            assert_eq!(subscriber_count(&constants, "battery_status"), expected, "{features:?}");
        }
    }

    #[test]
    fn reserves_led_subscribers_for_display_split_and_dual_rynk_sessions() {
        let config: KeyboardTomlConfig = toml::from_str("").unwrap();
        let constants = config.build_constants(&["display", "split", "rynk", "_ble"]).unwrap();
        // Three indicator processors, the display, two split peripherals, and USB/BLE Rynk sessions.
        assert_eq!(subscriber_count(&constants, "led_indicator"), 8);
    }

    #[test]
    fn dongle_display_reserves_the_dongle_state_subscriber() {
        let config: KeyboardTomlConfig = toml::from_str("").unwrap();
        let subs = |features: &[&str]| subscriber_count(&config.build_constants(features).unwrap(), "dongle_state");

        // Nobody listens on a screenless dongle, so publishing there is a no-op.
        assert_eq!(subs(&["dongle", "_ble", "storage"]), 0);
        assert_eq!(subs(&["dongle", "display", "_ble", "storage"]), 1);
    }

    #[test]
    fn each_configured_pointing_device_reserves_a_sleep_state_subscriber() {
        use crate::{InputDeviceConfig, Pmw3610Config};
        let subs = |config: &KeyboardTomlConfig| subscriber_count(&config.build_constants(&[]).unwrap(), "sleep_state");
        let sensors = |n: usize| {
            Some(InputDeviceConfig {
                pmw3610: Some(vec![Pmw3610Config::default(); n]),
                ..Default::default()
            })
        };
        let mut config: KeyboardTomlConfig = toml::from_str("").unwrap();
        let base = subs(&config);
        config.input_device = sensors(1);
        assert_eq!(subs(&config), base + 1);
        // A split binary carries one board's devices: one on the central and
        // two on a peripheral reserve two, not three.
        config.input_device = None;
        config.split = Some(SplitConfig {
            central: SplitBoardConfig {
                input_device: sensors(1),
                ..Default::default()
            },
            peripheral: vec![SplitBoardConfig {
                input_device: sensors(2),
                ..Default::default()
            }],
            ..Default::default()
        });
        assert_eq!(subs(&config), base + 2);
    }

    fn split_keyboard(central: SplitBoardConfig, peripheral: Vec<SplitBoardConfig>) -> KeyboardTomlConfig {
        let mut config = parse("");
        config.split = Some(SplitConfig {
            central,
            peripheral,
            ..Default::default()
        });
        config.auto_calculate_parameters();
        config
    }

    fn board_with_battery(adc_pin: Option<&str>, description: Option<&str>) -> SplitBoardConfig {
        SplitBoardConfig {
            battery: Some(BatteryTomlConfig {
                battery_adc_pin: adc_pin.map(str::to_owned),
                battery_user_description: description.map(str::to_owned),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn subscriber_count(constants: &BuildConstants, name: &str) -> usize {
        constants.events.iter().find(|event| event.name == name).unwrap().subs
    }

    #[test]
    fn unmeasured_peripherals_reserve_no_battery_ids_names_or_subscribers() {
        let unibody = parse("");
        let split = split_keyboard(Default::default(), vec![SplitBoardConfig::default()]);
        for (case, config, features) in [
            ("unibody", &unibody, &["_ble"][..]),
            ("split without a table", &unibody, &["split", "_ble"]),
            ("peripheral without ADC", &split, &["split", "_ble"]),
        ] {
            let base = config.build_constants(&[]).unwrap();
            let ble = config.build_constants(features).unwrap();
            assert!(ble.split_battery_peripheral_ids.is_empty(), "{case}");
            assert!(ble.split_battery_peripheral_user_descriptions.is_empty(), "{case}");
            assert_eq!(
                subscriber_count(&ble, "peripheral_battery"),
                subscriber_count(&base, "peripheral_battery"),
                "{case}"
            );
        }
    }

    #[test]
    fn ble_battery_subscribers_cover_the_server_and_each_host_link() {
        let mut config = split_keyboard(Default::default(), vec![board_with_battery(Some("P0_02"), None)]);
        config.rmk.num_ble_profiles = 5;
        config.event.battery_status.subs = 2;
        config.event.peripheral_battery.subs = 3;

        let cases: &[(&[&str], Option<usize>, usize, usize)] = &[
            (&[], None, 3, 3),
            (&["_ble"], None, 5, 3),
            (&["_ble", "_ext_adv"], Some(5), 9, 3),
            (&["split", "_ble"], None, 6, 5),
            (&["split", "_ble", "_ext_adv"], Some(5), 10, 9),
            (&["split", "_ble", "_ext_adv", "dongle"], Some(6), 12, 10),
        ];
        for &(features, max_connections, battery_status_subs, peripheral_battery_subs) in cases {
            config.ble = Some(BleConfig {
                max_connections,
                ..Default::default()
            });
            let constants = config.build_constants(features).unwrap();
            for (name, expected) in [
                ("battery_status", battery_status_subs),
                ("peripheral_battery", peripheral_battery_subs),
            ] {
                assert_eq!(subscriber_count(&constants, name), expected, "{features:?}: {name}");
            }
        }
    }

    #[test]
    fn each_host_link_reserves_both_of_its_connection_status_waits() {
        let mut config: KeyboardTomlConfig = toml::from_str("").unwrap();
        config.rmk.num_ble_profiles = 3;
        for (features, max_connections, expected) in [
            (&[][..], None, 1),
            (&["_ble"], None, 3),
            (&["_ble", "_ext_adv"], Some(3), 7),
        ] {
            config.ble = Some(BleConfig {
                max_connections,
                ..Default::default()
            });
            let constants = config.build_constants(features).unwrap();
            assert_eq!(
                subscriber_count(&constants, "connection_status_change"),
                expected,
                "{features:?}"
            );
        }
    }

    #[test]
    fn max_connections_fit_the_profiles_and_need_extended_advertising() {
        let mut config: KeyboardTomlConfig = toml::from_str("").unwrap();
        config.rmk.num_ble_profiles = 2;
        assert_eq!(config.build_constants(&["_ble", "_ext_adv"]).unwrap().num_host_links, 1);
        for (max_connections, features, links) in [
            (0, &["_ble", "_ext_adv"][..], None),
            (2, &["_ble"], None),
            (2, &["_ble", "_ext_adv"], Some(2)),
            (3, &["_ble", "_ext_adv"], None),
            (3, &["_ble", "_ext_adv", "dongle"], Some(3)),
        ] {
            config.ble = Some(BleConfig {
                max_connections: Some(max_connections),
                ..Default::default()
            });
            let resolved = config
                .build_constants(features)
                .ok()
                .map(|constants| constants.num_host_links);
            assert_eq!(resolved, links, "{max_connections} links with {features:?}");
        }
    }

    #[test]
    fn only_an_explicit_measured_pair_has_standard_left_right_batteries() {
        let board = |col_offset| SplitBoardConfig {
            rows: 1,
            cols: 1,
            col_offset,
            ..board_with_battery(Some("vddh"), None)
        };
        let left = Some(SplitSide::Left);
        let right = Some(SplitSide::Right);
        let sparse = vec![board(1), Default::default(), board(3)];
        for (central_side, central, peripheral, paired_side, ids) in [
            (None, board(0), vec![board(1)], None, &[0][..]),
            (left, board(0), vec![board(1)], left, &[0]),
            (right, board(0), vec![board(1)], right, &[0]),
            (left, board(0), vec![Default::default()], None, &[]),
            (left, Default::default(), vec![board(1)], None, &[0]),
            (left, board(0), vec![Default::default(), board(2)], None, &[1]),
            (left, board(0), sparse, None, &[0, 2]),
        ] {
            let mut config = split_keyboard(central, peripheral);
            config.split.as_mut().unwrap().central_side = central_side;
            let constants = config.build_constants(&["split", "_ble"]).unwrap();
            assert_eq!(constants.paired_battery_central_side, paired_side);
            assert_eq!(constants.split_battery_peripheral_ids, ids);
            assert_eq!(constants.central_battery_user_description, "Central");
            let names: Vec<_> = ids.iter().map(|id| format!("Peripheral {id}")).collect();
            assert_eq!(constants.split_battery_peripheral_user_descriptions, names);
        }
    }

    #[test]
    fn rejects_peripheral_battery_user_description_without_adc_pin() {
        let config = split_keyboard(Default::default(), vec![board_with_battery(None, Some("Right"))]);
        assert!(config.build_constants(&[]).is_ok());

        let err = match config.build_constants(&["split"]) {
            Ok(_) => panic!("expected battery user description validation failure"),
            Err(err) => err,
        };

        assert_eq!(
            err,
            "keyboard.toml: [[split.peripheral]] at index 0 requires battery_adc_pin when battery_user_description is set"
        );
    }

    #[test]
    fn resolves_custom_battery_user_descriptions() {
        let config = split_keyboard(
            board_with_battery(None, Some("Left")),
            vec![
                board_with_battery(Some("P0_02"), Some("Right")),
                board_with_battery(Some("P0_04"), None),
            ],
        );
        let constants = config.build_constants(&["split", "_ble"]).unwrap();

        assert_eq!(constants.central_battery_user_description, "Left");
        assert_eq!(
            constants.split_battery_peripheral_user_descriptions,
            ["Right", "Peripheral 1"]
        );
    }

    #[test]
    fn validates_passkey_timeout() {
        let ble = BleConfig {
            passkey_entry_timeout: Some(MIN_PASSKEY_ENTRY_TIMEOUT_SECS - 1),
            ..Default::default()
        };

        let err = match resolve_passkey_enabled(&ble) {
            Ok(_) => panic!("expected passkey timeout validation failure"),
            Err(err) => err,
        };
        assert_eq!(
            err,
            format!(
                "keyboard.toml: [ble.passkey_entry_timeout] must be at least {} seconds, got {}",
                MIN_PASSKEY_ENTRY_TIMEOUT_SECS,
                MIN_PASSKEY_ENTRY_TIMEOUT_SECS - 1
            )
        );
    }

    #[test]
    fn uses_default_timeout() {
        let ble = BleConfig::default();
        let passkey = resolve_passkey_enabled(&ble).unwrap();

        assert!(!passkey.enabled);
        assert_eq!(passkey.timeout_secs, DEFAULT_PASSKEY_ENTRY_TIMEOUT_SECS);
    }

    fn parse(toml: &str) -> crate::KeyboardTomlConfig {
        toml::from_str(toml).expect("Failed to parse keyboard config")
    }

    #[test]
    fn max_auto_mouse_layers_explicitly_too_small_is_rejected() {
        let toml = "[rmk]\nmax_auto_mouse_layers = 0\n\n[[behavior.auto_mouse_layer]]\ntarget_layer = 1\n";
        let err = match parse(toml).build_constants(&[]) {
            Ok(_) => panic!("expected max_auto_mouse_layers validation failure"),
            Err(err) => err,
        };
        assert!(err.contains("max_auto_mouse_layers"));
    }

    #[test]
    fn auto_mouse_layer_within_capacity_is_accepted() {
        let toml = "[rmk]\nmax_auto_mouse_layers = 1\n\n[[behavior.auto_mouse_layer]]\ntarget_layer = 1\nextra_mouse_keys = [\"LCtrl\"]\n";
        assert!(parse(toml).build_constants(&[]).is_ok());
    }

    #[test]
    fn deactivate_on_key_without_action_subs_is_rejected() {
        let toml = "[[behavior.auto_mouse_layer]]\ntarget_layer = 1\ndeactivate_on_key = true\n";
        let err = match parse(toml).build_constants(&[]) {
            Ok(_) => panic!("expected action subs validation failure"),
            Err(err) => err,
        };
        assert!(err.contains("[event.action]"));
    }

    #[test]
    fn deactivate_on_key_with_action_subs_set_is_accepted() {
        let toml = "[event.action]\nchannel_size = 16\npubs = 1\nsubs = 1\n\n[[behavior.auto_mouse_layer]]\ntarget_layer = 1\ndeactivate_on_key = true\n";
        assert!(parse(toml).build_constants(&[]).is_ok());
    }

    #[test]
    fn ble_reserves_advertising_timeout_wake_subscribers() {
        // ble/mod.rs subscribes to KeyboardEvent/PointingEvent when advertising
        // times out, on top of every permanent subscriber. Without a reserved
        // slot that call panics instead of sleeping until the next key press.
        let base = parse("").build_constants(&[]).unwrap();
        let ble = parse("").build_constants(&["_ble"]).unwrap();

        for event in ["keyboard", "pointing"] {
            assert_eq!(
                subscriber_count(&ble, event),
                subscriber_count(&base, event) + 1,
                "{event} needs a wake subscriber slot under _ble"
            );
        }
    }

    #[test]
    fn validates_capability_wire_widths() {
        assert!(validate_u8_capability("num_ble_profiles", 255).is_ok());
        assert_eq!(
            validate_u8_capability("num_ble_profiles", 256),
            Err("num_ble_profiles (256) exceeds the u8 host capability field (max 255)".to_string())
        );

        assert!(validate_u16_capability("rynk_buffer_size", 65535).is_ok());
        assert_eq!(
            validate_u16_capability("rynk_buffer_size", 65536),
            Err("rynk_buffer_size (65536) exceeds the u16 host capability field (max 65535)".to_string())
        );
    }
}
