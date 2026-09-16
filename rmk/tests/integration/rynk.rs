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

/// The write must land in storage, not just the live keymap: a keyboard built
/// over the same flash afterwards has only what was persisted.
#[cfg(feature = "storage")]
#[test]
fn keymap_write_survives_restart() {
    const SET_KEY_B: &str = r#"{"position":{"layer":0,"row":0,"col":0},"action":{"Single":{"Key":{"Hid":"B"}}}}"#;

    test_block_on(async {
        let flash = crate::simulator::flash::InMemoryFlash::new();
        {
            let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build_with_flash(flash.clone()).await;
            keyboard
                .rynk::<command::SetKeyAction>(SET_KEY_B, RynkReply::Ok("null"))
                .wait_storage()
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

#[cfg(feature = "storage")]
#[test]
fn hold_trigger_positions_survive_restart() {
    const SET_POSITIONS: &str = r#"{"positions":[{"profile":255,"row":0,"col":0},{"profile":0,"row":0,"col":1}]}"#;
    const STORED_STATE: &str =
        r#"{"capacity":16,"positions":[{"profile":255,"row":0,"col":0},{"profile":0,"row":0,"col":1}]}"#;

    test_block_on(async {
        let flash = crate::simulator::flash::InMemoryFlash::new();
        {
            let mut keyboard = SimKeyboard::builder([[[k!(A), k!(B)]]])
                .build_with_flash(flash.clone())
                .await;
            keyboard
                .rynk::<command::SetMorseHoldTriggerPositions>(SET_POSITIONS, RynkReply::Ok("null"))
                .wait_storage()
                .run()
                .await;
        }
        let mut keyboard = SimKeyboard::builder([[[k!(A), k!(B)]]]).build_with_flash(flash).await;
        keyboard
            .rynk::<command::GetMorseHoldTriggerPositions>("null", RynkReply::Ok(STORED_STATE))
            .run()
            .await;
    });
}
