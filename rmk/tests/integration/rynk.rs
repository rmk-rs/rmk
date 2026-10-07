//! The Rynk bridge scenario files drive, plus the exchanges TOML cannot express.
//!
//! A scenario spells a request as data; `run_tests!` forwards it as JSON and
//! [`SimKeyboard::rynk`] deserializes it into the endpoint's own `Request` type
//! before handing the bytes to the harness. Encoding therefore happens here,
//! against the firmware's `rmk-types` build — not at macro expansion, where
//! `Action`'s `#[cfg(feature = "steno")]` variant would shift every postcard
//! discriminant after it.
//!
//! Endpoint coverage lives in `tests/scenarios/rynk_*.toml`. What stays below is
//! the case a scenario has no vocabulary for: two keyboards built over one
//! flash, which is a lifetime a timeline cannot describe.

use rmk::event::{
    ConnectionStatusChangeEvent, LedIndicatorEvent, SleepStateEvent, WpmUpdateEvent, publish_event_async,
};
use rmk::k;
use rmk::test_support::test_block_on;
use rmk::types::keycode::HidKeyCode;
use rmk_types::connection::ConnectionStatus;
use rmk_types::led_indicator::LedIndicator;
use rmk_types::protocol::rynk::command::Endpoint;
use rmk_types::protocol::rynk::{Cmd, RynkError, RynkHeader, command, encode_frame};

use crate::simulator::SimKeyboard;

/// What a scenario expects back from one Rynk request.
pub(crate) enum RynkReply<'a> {
    /// JSON for the endpoint's `Response`, wrapped in `Ok` on the wire.
    Ok(&'a str),
    Err(RynkError),
}

impl SimKeyboard {
    /// Send one endpoint request, spelled as JSON for its `Request` type.
    pub(crate) fn rynk<E: Endpoint>(&mut self, request: &str, reply: RynkReply<'_>) -> &mut Self {
        let request = frame(E::CMD, 1, &payload::<E::Request>(request, "request"));
        match reply {
            RynkReply::Ok(response) => {
                let response = payload::<E::Response>(response, "response");
                self.host_exchange(request, frame(E::CMD, 1, &Ok::<_, RynkError>(response)))
            }
            RynkReply::Err(error) => self.host_exchange(request, frame(E::CMD, 1, &Err::<E::Response, _>(error))),
        }
    }

    /// Assert the device's next frame is this topic push. Topics carry a bare
    /// payload at seq 0, not a `Result`. The topic table has no marker types, so
    /// `cmd` names the payload type here, the way it names the event below.
    pub(crate) fn rynk_topic(&mut self, cmd: Cmd, json: &str) -> &mut Self {
        let expected = match cmd {
            Cmd::LayerChange => frame(cmd, 0, &payload::<u8>(json, "topic")),
            Cmd::WpmUpdate => frame(cmd, 0, &payload::<u16>(json, "topic")),
            Cmd::ConnectionChange => frame(cmd, 0, &payload::<ConnectionStatus>(json, "topic")),
            Cmd::SleepState => frame(cmd, 0, &payload::<bool>(json, "topic")),
            Cmd::LedIndicatorChange => frame(cmd, 0, &payload::<LedIndicator>(json, "topic")),
            #[cfg(feature = "_ble")]
            Cmd::BatteryStatusChange => frame(cmd, 0, &payload::<rmk_types::battery::BatteryStatus>(json, "topic")),
            cmd => panic!("{cmd:?} is not a topic"),
        };
        self.expect_host_frame(expected)
    }

    /// Publish the internal event `cmd`'s topic forwards, to cause its push. Only
    /// the topics fed by state a timeline cannot otherwise reach need this;
    /// `LayerChange` is caused by pressing a layer key.
    pub(crate) fn rynk_publish(&mut self, cmd: Cmd, json: &str) -> &mut Self {
        match cmd {
            Cmd::WpmUpdate => self.publish(publish_event_async(WpmUpdateEvent(payload(json, "topic")))),
            Cmd::SleepState => self.publish(publish_event_async(SleepStateEvent(payload(json, "topic")))),
            Cmd::LedIndicatorChange => self.publish(publish_event_async(LedIndicatorEvent(payload(json, "topic")))),
            Cmd::ConnectionChange => {
                self.publish(publish_event_async(ConnectionStatusChangeEvent(payload(json, "topic"))))
            }
            #[cfg(feature = "_ble")]
            Cmd::BatteryStatusChange => self.publish(publish_event_async(rmk::event::BatteryStatusEvent(payload(
                json, "topic",
            )))),
            cmd => panic!("no internal event feeds topic {cmd:?}"),
        }
    }
}

fn frame<T: serde::Serialize>(cmd: Cmd, seq: u8, payload: &T) -> Vec<u8> {
    let mut buf = [0; rmk_types::constants::RYNK_BUFFER_SIZE];
    let n = encode_frame(&mut buf, RynkHeader { cmd, seq }, payload).unwrap();
    buf[..n].to_vec()
}

/// A scenario's payload is JSON because that keeps every serde shape — including
/// the `is_human_readable` ones such as `LedIndicator` — spelled the way TOML
/// spells it, while postcard still sees the typed value.
fn payload<T: serde::de::DeserializeOwned>(json: &str, what: &str) -> T {
    serde_json::from_str(json).unwrap_or_else(|e| panic!("rynk {what} payload `{json}`: {e}"))
}

/// A macro write lands in flash and outlives the keyboard; a write the flash
/// refuses is reported, and the slot stays as it was.
#[cfg(feature = "storage")]
#[test]
fn macro_write_survives_restart_and_reports_a_storage_fault() {
    const TAP_C: &str = r#"[{"Tap":{"Key":{"Hid":"C"}}}]"#;
    const SET_0: &str = r#"{"index":0,"macro_ops":[{"Tap":{"Key":{"Hid":"C"}}}]}"#;
    const SET_1: &str = r#"{"index":1,"macro_ops":[{"Char":97}]}"#;

    test_block_on(async {
        let flash = crate::simulator::Flash::new();
        {
            let mut keyboard = SimKeyboard::builder([[[rmk::macros!(0), rmk::macros!(1)]]])
                .build_with_flash(flash.clone())
                .await;
            keyboard
                .rynk::<command::SetMacro>(SET_0, RynkReply::Ok("null"))
                .run()
                .await;
            flash.fail_writes(true);
            keyboard
                .rynk::<command::SetMacro>(SET_1, RynkReply::Err(RynkError::StorageFault))
                .run()
                .await;
            flash.fail_writes(false);
        }
        let mut keyboard = SimKeyboard::builder([[[rmk::macros!(0), rmk::macros!(1)]]])
            .build_with_flash(flash)
            .await;
        keyboard
            .rynk::<command::GetMacro>("0", RynkReply::Ok(TAP_C))
            .rynk::<command::GetMacro>("1", RynkReply::Ok("[]"))
            .tap(0, 0, 10)
            .expect_keys([HidKeyCode::C])
            .expect_keys([])
            .run()
            .await;
    });
}

/// The write must land in storage, not just the live keymap: a keyboard built
/// over the same flash afterwards has only what was persisted.
#[cfg(feature = "storage")]
#[test]
fn keymap_write_survives_restart() {
    const SET_KEY_B: &str = r#"{"position":{"layer":0,"row":0,"col":0},"action":{"Single":{"Key":{"Hid":"B"}}}}"#;

    test_block_on(async {
        let flash = crate::simulator::Flash::new();
        {
            let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build_with_flash(flash.clone()).await;
            keyboard
                .rynk::<command::SetKeyAction>(SET_KEY_B, RynkReply::Ok("null"))
                .run()
                .await;
        }
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build_with_flash(flash).await;
        keyboard
            .tap(0, 0, 10)
            .expect_keys([HidKeyCode::B])
            .expect_keys([])
            .run()
            .await;
    });
}

/// A rejected write is reported, not hidden: the handler answers `StorageFault`
/// after the live keymap already holds the new binding, and a keyboard built
/// over the same flash afterwards still has the old one.
#[cfg(feature = "storage")]
#[test]
fn set_key_on_failing_flash_replies_storage_fault() {
    const SET_KEY_B: &str = r#"{"position":{"layer":0,"row":0,"col":0},"action":{"Single":{"Key":{"Hid":"B"}}}}"#;

    test_block_on(async {
        let flash = crate::simulator::Flash::new();
        {
            let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build_with_flash(flash.clone()).await;
            flash.fail_writes(true);
            keyboard
                .rynk::<command::SetKeyAction>(SET_KEY_B, RynkReply::Err(RynkError::StorageFault))
                .tap(0, 0, 10)
                .expect_keys([HidKeyCode::B])
                .expect_keys([])
                .run()
                .await;
            flash.fail_writes(false);
        }
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build_with_flash(flash).await;
        keyboard
            .tap(0, 0, 10)
            .expect_keys([HidKeyCode::A])
            .expect_keys([])
            .run()
            .await;
    });
}

#[cfg(feature = "storage")]
#[test]
fn sticky_profiles_survive_restart_and_report_storage_faults() {
    const NAMED: &str = r#"{"release_on":{"layer_activate":true},"ignore":[{"Hid":"LCtrl"}],"wait_timeout_ms":4321,"hold_timeout_ms":0}"#;
    const DEFAULT: &str = r#"{"release_on":{"before_next_press":true},"ignore":[{"Hid":"LShift"}],"wait_timeout_ms":0,"hold_timeout_ms":123}"#;
    const FAILED: &str = r#"{"release_on":{},"ignore":[],"wait_timeout_ms":12,"hold_timeout_ms":34}"#;
    let behavior = || {
        let mut b = rmk::config::BehaviorConfig::default();
        b.sticky_key
            .profiles
            .push(rmk_types::sticky::StickyProfile::default())
            .unwrap();
        b
    };
    test_block_on(async {
        let flash = crate::simulator::Flash::new();
        {
            let mut keyboard = SimKeyboard::builder([[[k!(A)]]])
                .behavior_config(behavior())
                .build_with_flash(flash.clone())
                .await;
            keyboard
                .rynk::<command::SetStickyProfile>(&format!(r#"{{"index":0,"config":{NAMED}}}"#), RynkReply::Ok("null"))
                .rynk::<command::SetStickyProfile>(
                    &format!(r#"{{"index":255,"config":{DEFAULT}}}"#),
                    RynkReply::Ok("null"),
                )
                .run()
                .await;
            flash.fail_writes(true);
            for idx in [0, 255] {
                keyboard
                    .rynk::<command::SetStickyProfile>(
                        &format!(r#"{{"index":{idx},"config":{FAILED}}}"#),
                        RynkReply::Err(RynkError::StorageFault),
                    )
                    .rynk::<command::GetStickyProfile>(&idx.to_string(), RynkReply::Ok(FAILED))
                    .run()
                    .await;
            }
            flash.fail_writes(false);
        }
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]])
            .behavior_config(behavior())
            .build_with_flash(flash)
            .await;
        keyboard
            .rynk::<command::GetStickyProfile>("0", RynkReply::Ok(NAMED))
            .rynk::<command::GetStickyProfile>("255", RynkReply::Ok(DEFAULT))
            .run()
            .await;
    });
}

#[test]
fn oversized_sticky_ignore_list_is_rejected_without_changing_ram() {
    test_block_on(async {
        let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build().await;
        let ignore = vec![rmk_types::keycode::KeyCode::Hid(HidKeyCode::A); rmk_types::constants::STICKY_IGNORE_MAX + 1];
        let oversized = (255u8, (0u8, ignore, 0u16, 0u16));
        keyboard
            .host_exchange(
                frame(Cmd::SetStickyProfile, 1, &oversized),
                frame(Cmd::SetStickyProfile, 1, &Err::<(), _>(RynkError::Malformed)),
            )
            .rynk::<command::GetStickyProfile>("255", RynkReply::Ok(r#"{"release_on":{"after_next_release":true},"ignore":[],"wait_timeout_ms":1000,"hold_timeout_ms":250}"#))
            .run().await;
    });
}
