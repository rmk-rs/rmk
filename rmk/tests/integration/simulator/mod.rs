//! Host-side simulation of a complete keyboard device.
//!
//! Drives the same keyboard task firmware runs: publishes input events,
//! captures HID reports, and dispatches host requests through the production
//! protocol services. It stops at RMK's transport boundaries — no USB
//! enumeration, BLE radio state, or GPIO electrical behavior.
//!
//! Lives in the test crate, so it drives only rmk's public API (plus the
//! `#[doc(hidden)]` `rmk::test_support` gate for a few internal signals).

#[cfg(feature = "storage")]
pub mod flash;

use core::future::Future;
use core::pin::Pin;

#[cfg(feature = "storage")]
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_futures::select::{Either, select};
use embassy_futures::yield_now;
use embassy_time::{Duration, Timer};
#[cfg(feature = "storage")]
use embedded_storage::nor_flash::NorFlash;
// Every `_no_usb` chip feature also turns on `_ble`, so exactly one report
// channel always exists.
#[cfg(feature = "_no_usb")]
use rmk::channel::BLE_REPORT_CHANNEL as REPORT_CHANNEL;
#[cfg(not(feature = "_no_usb"))]
use rmk::channel::USB_REPORT_CHANNEL as REPORT_CHANNEL;
#[cfg(feature = "host")]
use rmk::config::RmkConfig;
#[cfg(feature = "storage")]
use rmk::config::StorageConfig;
use rmk::config::{BehaviorConfig, Hand, PositionalConfig};
use rmk::core_traits::Runnable;
use rmk::event::{AsyncEventPublisher, AsyncPublishableEvent, KeyboardEvent, KeyboardEventPos};
use rmk::hid::{KeyboardReport, Report};
use rmk::input_device::rotary_encoder::Direction;
use rmk::keyboard::Keyboard;
use rmk::keymap::{KeyMap, KeymapData};
use rmk::types::action::{EncoderAction, KeyAction};
use rmk_types::keycode::HidKeyCode;

const TIMEOUT_SECS: u64 = 5;
const TIMEOUT: Duration = Duration::from_secs(TIMEOUT_SECS);

// `#[cfg]` rather than `cfg!`: `RYNK_BUFFER_SIZE` only exists with the `rynk`
// feature, and both arms of the latter must still resolve.
#[cfg(feature = "rynk")]
const LINK_BYTES: usize = rmk_types::constants::RYNK_BUFFER_SIZE;
#[cfg(not(feature = "rynk"))]
const LINK_BYTES: usize = 64;

/// One direction of the in-memory duplex a host session runs on.
type Link = embassy_sync::pipe::Pipe<embassy_sync::blocking_mutex::raw::NoopRawMutex, LINK_BYTES>;

/// Reset the process-global state a run observes. `embassy-time`'s `MockDriver`
/// forces one test per process, but a test may still build several keyboards.
fn reset() {
    KeyboardEvent::publisher_async()
        .expect("free publisher slot at reset")
        .clear();

    rmk::test_support::reset_connection_status();
    #[cfg(not(feature = "_no_usb"))]
    rmk::state::set_usb_state(rmk_types::connection::UsbState::Configured);
    #[cfg(feature = "_no_usb")]
    rmk::test_support::set_ble_state(rmk_types::ble::BleState::Connected);

    #[cfg(not(feature = "_no_usb"))]
    rmk::channel::USB_REPORT_CHANNEL.clear();
    #[cfg(feature = "_ble")]
    rmk::channel::BLE_REPORT_CHANNEL.clear();

    #[cfg(feature = "storage")]
    rmk::test_support::clear_flash_channel();
}

pub struct SimKeyboardBuilder<const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize> {
    keymap: [[[KeyAction; COL]; ROW]; NUM_LAYER],
    encoder_map: [[EncoderAction; NUM_ENCODER]; NUM_LAYER],
    behavior_config: BehaviorConfig,
    positional_config: PositionalConfig<ROW, COL>,
    #[cfg(feature = "host")]
    rmk_config: RmkConfig<'static>,
}

impl<const ROW: usize, const COL: usize, const NUM_LAYER: usize, const NUM_ENCODER: usize>
    SimKeyboardBuilder<ROW, COL, NUM_LAYER, NUM_ENCODER>
{
    pub fn hands(mut self, hands: [[Hand; COL]; ROW]) -> Self {
        self.positional_config.hand = hands;
        self
    }

    pub fn behavior_config(mut self, behavior_config: BehaviorConfig) -> Self {
        self.behavior_config = behavior_config;
        self
    }

    /// The production config the host session serves from: the `[host]` lock
    /// gate, whose unlock keys are physical positions a scenario can press, and
    /// the compressed layout blob `GetLayout` pages out.
    #[cfg(feature = "host")]
    pub fn rmk_config(mut self, rmk_config: RmkConfig<'static>) -> Self {
        self.rmk_config = rmk_config;
        self
    }

    pub fn encoders<const NEW_NUM_ENCODER: usize>(
        self,
        encoder_map: [[EncoderAction; NEW_NUM_ENCODER]; NUM_LAYER],
    ) -> SimKeyboardBuilder<ROW, COL, NUM_LAYER, NEW_NUM_ENCODER> {
        SimKeyboardBuilder {
            keymap: self.keymap,
            encoder_map,
            behavior_config: self.behavior_config,
            positional_config: self.positional_config,
            #[cfg(feature = "host")]
            rmk_config: self.rmk_config,
        }
    }

    pub async fn build(self) -> SimKeyboard {
        let data = Box::leak(Box::new(KeymapData::new_with_encoder(self.keymap, self.encoder_map)));
        let behavior = Box::leak(Box::new(self.behavior_config));
        let positional = Box::leak(Box::new(self.positional_config));
        let keymap = Box::leak(Box::new(KeyMap::new(data, behavior, positional).await));
        SimKeyboard {
            keyboard: Keyboard::new(keymap),
            #[cfg(feature = "host")]
            keymap,
            #[cfg(feature = "host")]
            rmk_config: self.rmk_config,
            steps: Vec::new(),
            storage: None,
        }
    }

    /// Build with persistent storage backed by `flash`, so a later build over
    /// the same flash sees what this one wrote.
    #[cfg(feature = "storage")]
    pub async fn build_with_flash<F: NorFlash + 'static>(self, flash: F) -> SimKeyboard {
        let data = Box::leak(Box::new(KeymapData::new_with_encoder(self.keymap, self.encoder_map)));
        let (keymap, mut storage) = rmk::initialize_keymap_and_storage(
            data,
            BlockingAsync::new(flash),
            &StorageConfig::default(),
            Box::leak(Box::new(self.behavior_config)),
            Box::leak(Box::new(self.positional_config)),
        )
        .await;
        let keymap = Box::leak(Box::new(keymap));
        SimKeyboard {
            keyboard: Keyboard::new(keymap),
            #[cfg(feature = "host")]
            keymap,
            #[cfg(feature = "host")]
            rmk_config: self.rmk_config,
            steps: Vec::new(),
            storage: Some(Box::pin(async move { storage.run().await })),
        }
    }
}

enum SimStep {
    Event(KeyboardEvent),
    Delay(Duration),
    ExpectReport(Report),
    ExpectNoReport(Duration),
    HostSend(Vec<u8>),
    ExpectHostFrame(Vec<u8>),
    ExpectNoHostReply(Duration),
    /// The internal event behind a topic push, already built as its publish
    /// future — so the harness needs no vocabulary of its own for event types.
    #[cfg(feature = "rynk")]
    Publish(Pin<Box<dyn Future<Output = ()>>>),
    #[cfg(feature = "storage")]
    WaitStorage,
    #[cfg(feature = "passkey_entry")]
    BeginPasskeyEntry,
    #[cfg(feature = "passkey_entry")]
    ExpectPasskeyResponse(Option<u32>),
    #[cfg(feature = "passkey_entry")]
    EndPasskeyEntry,
}

/// Simulator for a complete keyboard device, excluding physical input and
/// transport I/O. The methods below queue a timeline; [`SimKeyboard::run`]
/// plays it against a live keyboard task.
pub struct SimKeyboard {
    keyboard: Keyboard<'static>,
    #[cfg(feature = "host")]
    keymap: &'static KeyMap<'static>,
    #[cfg(feature = "host")]
    rmk_config: RmkConfig<'static>,
    steps: Vec<SimStep>,
    storage: Option<Pin<Box<dyn Future<Output = ()>>>>,
}

impl SimKeyboard {
    pub fn builder<const ROW: usize, const COL: usize, const NUM_LAYER: usize>(
        keymap: [[[KeyAction; COL]; ROW]; NUM_LAYER],
    ) -> SimKeyboardBuilder<ROW, COL, NUM_LAYER, 0> {
        SimKeyboardBuilder {
            keymap,
            encoder_map: [const { [] }; NUM_LAYER],
            behavior_config: BehaviorConfig::default(),
            positional_config: PositionalConfig::default(),
            #[cfg(feature = "host")]
            rmk_config: RmkConfig::default(),
        }
    }

    fn event(&mut self, event: KeyboardEvent) -> &mut Self {
        self.steps.push(SimStep::Event(event));
        self
    }

    pub fn press(&mut self, row: u8, col: u8) -> &mut Self {
        self.event(KeyboardEvent::key(row, col, true))
    }

    pub fn release(&mut self, row: u8, col: u8) -> &mut Self {
        self.event(KeyboardEvent::key(row, col, false))
    }

    pub fn tap(&mut self, row: u8, col: u8, hold_ms: u64) -> &mut Self {
        self.press(row, col).delay(hold_ms).release(row, col)
    }

    pub fn delay(&mut self, ms: u64) -> &mut Self {
        self.steps.push(SimStep::Delay(Duration::from_millis(ms)));
        self
    }

    pub fn rotary_cw(&mut self, id: u8) -> &mut Self {
        self.rotary(id, Direction::Clockwise)
    }

    pub fn rotary_ccw(&mut self, id: u8) -> &mut Self {
        self.rotary(id, Direction::CounterClockwise)
    }

    fn rotary(&mut self, id: u8, direction: Direction) -> &mut Self {
        self.event(KeyboardEvent::rotary_encoder(id, direction, true))
            .event(KeyboardEvent::rotary_encoder(id, direction, false))
    }

    pub fn expect_keys<const N: usize>(&mut self, keycodes: [HidKeyCode; N]) -> &mut Self {
        self.expect_keys_with_mods(0, keycodes)
    }

    /// The keyboard report a scenario's `expect` array spells: modifier bits
    /// plus the keycodes that are down, in any order.
    pub fn expect_keys_with_mods<const N: usize>(&mut self, modifier: u8, keycodes: [HidKeyCode; N]) -> &mut Self {
        let mut report = KeyboardReport {
            modifier,
            ..KeyboardReport::default()
        };
        let max = report.keycodes.len();
        assert!(
            N <= max,
            "keyboard HID reports carry at most {max} simultaneous keycodes"
        );
        for (slot, keycode) in report.keycodes.iter_mut().zip(keycodes) {
            *slot = keycode as u8;
        }
        self.expect_report(Report::KeyboardReport(report))
    }

    pub fn expect_report(&mut self, report: Report) -> &mut Self {
        self.steps.push(SimStep::ExpectReport(report));
        self
    }

    /// Assert nothing reaches the host for `ms`.
    pub fn expect_no_report(&mut self, ms: u64) -> &mut Self {
        self.steps.push(SimStep::ExpectNoReport(Duration::from_millis(ms)));
        self
    }

    /// Send `request` to the device's host session and assert its reply.
    pub(crate) fn host_exchange(&mut self, request: impl Into<Vec<u8>>, expected: impl Into<Vec<u8>>) -> &mut Self {
        self.host_send(request).expect_host_frame(expected)
    }

    /// Send bytes to the host session without reading a reply. Nothing frames or
    /// validates them, so a scenario can feed the session garbage.
    pub(crate) fn host_send(&mut self, request: impl Into<Vec<u8>>) -> &mut Self {
        self.steps.push(SimStep::HostSend(request.into()));
        self
    }

    /// Assert the device's next `expected.len()` reply bytes are exactly these.
    /// Length-driven rather than delimited, because Vial replies are fixed-size
    /// raw reports while Rynk's are COBS frames.
    pub(crate) fn expect_host_frame(&mut self, expected: impl Into<Vec<u8>>) -> &mut Self {
        self.steps.push(SimStep::ExpectHostFrame(expected.into()));
        self
    }

    /// Assert the device stays silent for `ms`.
    pub(crate) fn expect_no_host_reply(&mut self, ms: u64) -> &mut Self {
        self.steps.push(SimStep::ExpectNoHostReply(Duration::from_millis(ms)));
        self
    }

    /// Queue an internal event for the timeline to publish. Topic pushes whose
    /// cause no matrix input reaches need this.
    #[cfg(feature = "rynk")]
    pub(crate) fn publish(&mut self, event: impl Future<Output = ()> + 'static) -> &mut Self {
        self.steps.push(SimStep::Publish(Box::pin(event)));
        self
    }

    #[cfg(feature = "storage")]
    pub fn wait_storage(&mut self) -> &mut Self {
        self.steps.push(SimStep::WaitStorage);
        self
    }

    #[cfg(feature = "passkey_entry")]
    pub fn begin_passkey_entry(&mut self) -> &mut Self {
        self.steps.push(SimStep::BeginPasskeyEntry);
        self
    }

    #[cfg(feature = "passkey_entry")]
    pub fn expect_passkey_response(&mut self, expected: Option<u32>) -> &mut Self {
        self.steps.push(SimStep::ExpectPasskeyResponse(expected));
        self
    }

    #[cfg(feature = "passkey_entry")]
    pub fn end_passkey_entry(&mut self) -> &mut Self {
        self.steps.push(SimStep::EndPasskeyEntry);
        self
    }

    /// Play the queued timeline, then assert the keyboard ended clean.
    pub async fn run(&mut self) {
        reset();
        let steps = core::mem::take(&mut self.steps);
        let keyboard = &mut self.keyboard;
        let storage = self.storage.as_mut();
        #[cfg(feature = "host")]
        let (keymap, rmk_config) = (self.keymap, &self.rmk_config);
        // Every host step needs the session; a scenario without one leaves it
        // parked so nothing competes for the link.
        #[cfg(feature = "host")]
        let has_host_step = steps.iter().any(|step| {
            matches!(
                step,
                SimStep::HostSend(_) | SimStep::ExpectHostFrame(_) | SimStep::ExpectNoHostReply(_)
            )
        });

        // Nothing else drains `FLASH_CHANNEL`, so without a storage task the
        // keyboard would block on it once full.
        let flash = async {
            match storage {
                Some(storage) => storage.await,
                None => rmk::channel::drain_flash_channel_for_test().await,
            }
        };

        // A host connection is just a byte stream: drive the production
        // `run_session` over an in-memory duplex, exactly as a USB/BLE transport
        // hands it one. `to_device` carries host→device requests, `from_device`
        // the device→host responses.
        let (to_device, from_device) = (Link::new(), Link::new());
        #[cfg(feature = "host")]
        let session = async {
            if !has_host_step {
                return core::future::pending().await;
            }
            let service = rmk::host::HostService::new(keymap, rmk_config);
            let (mut rx, mut tx): (&Link, &Link) = (&to_device, &from_device);
            service.run_session(&mut rx, &mut tx).await;
        };
        #[cfg(not(feature = "host"))]
        let session = core::future::pending::<()>();

        // None of these ever return; the timeline does, and dropping them is how
        // a run ends. One resolving first means a task died or, for the session,
        // that a framing guard rejected the stream.
        let background = select(keyboard.run(), select(flash, session));
        match select(background, run_steps(steps, &to_device, &from_device)).await {
            Either::First(_) => panic!("a background task ended before the scripted steps finished"),
            Either::Second(()) => {}
        }

        assert!(
            self.keyboard.held_buffer.is_empty(),
            "leak after buffer cleanup, buffer contains {:?}",
            self.keyboard.held_buffer
        );
        assert!(
            self.keyboard.unprocessed_events.is_empty(),
            "simulator ended with unprocessed keyboard events: {:?}",
            self.keyboard.unprocessed_events
        );
    }
}

/// Play `steps` in order, asserting as it goes. `to_device`/`from_device` are
/// the host end of the link the caller's session runs on.
async fn run_steps(steps: Vec<SimStep>, to_device: &Link, from_device: &Link) {
    let sender = KeyboardEvent::publisher_async().expect("free publisher slot for the scenario driver");
    let mut pressed_inputs = Vec::<KeyboardEventPos>::new();
    // Codegen queues one step per `expect` entry, in order, so the nth
    // expectation played is the scenario's `expect[n]`.
    let mut expects = 0usize;
    let mut replies = 0usize;

    for step in steps {
        match step {
            SimStep::Event(event) => {
                if event.pressed {
                    assert!(
                        !pressed_inputs.contains(&event.pos),
                        "input {} was pressed twice without a release",
                        input(event.pos)
                    );
                    pressed_inputs.push(event.pos);
                } else {
                    let Some(pos) = pressed_inputs.iter().position(|pressed| *pressed == event.pos) else {
                        panic!("input {} was released without a matching press", input(event.pos));
                    };
                    pressed_inputs.swap_remove(pos);
                }
                let waiting = format!("publishing {} blocked for {TIMEOUT_SECS}s", input(event.pos));
                with_timeout(sender.publish_async(event), &waiting).await;
            }
            SimStep::Delay(duration) => Timer::after(duration).await,
            SimStep::ExpectReport(expected) => {
                let at = format!("expect[{expects}]");
                expects += 1;
                let waiting = format!(
                    "{at}: no HID report within {TIMEOUT_SECS}s, expected {}",
                    summary(&expected)
                );
                let actual = with_timeout(REPORT_CHANNEL.receive(), &waiting).await;
                if !same_report(&expected, &actual) {
                    panic!(
                        "{at}: HID report mismatch\n  expected {}\n    actual {}",
                        summary(&expected),
                        summary(&actual)
                    );
                }
            }
            SimStep::ExpectNoReport(duration) => {
                let at = format!("expect[{expects}]");
                expects += 1;
                if let Either::Second(report) = select(Timer::after(duration), REPORT_CHANNEL.receive()).await {
                    panic!("{at}: unexpected HID report {}", summary(&report));
                }
            }
            SimStep::HostSend(request) => {
                #[cfg(feature = "storage")]
                rmk::test_support::reset_flash_operation();
                let blocked = format!(
                    "host request of {} bytes blocked for {TIMEOUT_SECS}s: {}",
                    request.len(),
                    hex(&request)
                );
                with_timeout(to_device.write_all(&request), &blocked).await;
            }
            SimStep::ExpectHostFrame(expected) => {
                let at = format!("reply[{replies}]");
                replies += 1;
                // `read_exact` would park on a short reply until the timeout,
                // hiding what did arrive, so fill the reply a chunk at a time.
                let mut actual = vec![0; expected.len()];
                let mut got = 0;
                while got < actual.len() {
                    let waiting = format!(
                        "{at}: no host reply within {TIMEOUT_SECS}s, got {got} of {} bytes: {}",
                        actual.len(),
                        hex(&actual[..got])
                    );
                    got += with_timeout(from_device.read(&mut actual[got..]), &waiting).await;
                }
                if expected != actual {
                    // Equal lengths by construction, so the leading run of equal
                    // bytes ends at the first difference.
                    let byte = expected.iter().zip(&actual).take_while(|(e, a)| e == a).count();
                    panic!(
                        "{at}: host reply differs at byte {byte}: expected {:#04x}, got {:#04x}\n  expected {}\n    actual {}",
                        expected[byte],
                        actual[byte],
                        hex(&expected),
                        hex(&actual)
                    );
                }
            }
            SimStep::ExpectNoHostReply(duration) => {
                let at = format!("reply[{replies}]");
                replies += 1;
                let mut reply = [0; LINK_BYTES];
                let read = from_device.read(&mut reply);
                if let Either::Second(n) = select(Timer::after(duration), read).await {
                    panic!("{at}: expected no host reply, got {n} byte(s): {}", hex(&reply[..n]));
                }
            }
            #[cfg(feature = "rynk")]
            SimStep::Publish(event) => event.await,
            #[cfg(feature = "storage")]
            SimStep::WaitStorage => {
                let waiting = format!("no storage write within {TIMEOUT_SECS}s");
                let written = with_timeout(rmk::test_support::flash_operation_finished(), &waiting).await;
                assert!(written, "storage write failed");
            }
            #[cfg(feature = "passkey_entry")]
            SimStep::BeginPasskeyEntry => rmk::ble::passkey::begin_passkey_entry_session(),
            #[cfg(feature = "passkey_entry")]
            SimStep::ExpectPasskeyResponse(expected) => {
                // A scenario spells the cancelled response `"cancelled"`.
                let render = |passkey: Option<u32>| match passkey {
                    Some(passkey) => passkey.to_string(),
                    None => "cancelled".to_string(),
                };
                let at = format!("expect[{expects}]");
                expects += 1;
                let waiting = format!(
                    "{at}: no passkey response within {TIMEOUT_SECS}s, expected {}",
                    render(expected)
                );
                let response = rmk::ble::passkey::PASSKEY_RESPONSE.wait();
                let actual = with_timeout(response, &waiting).await;
                if expected != actual {
                    panic!(
                        "{at}: passkey mismatch\n  expected {}\n    actual {}",
                        render(expected),
                        render(actual)
                    );
                }
            }
            #[cfg(feature = "passkey_entry")]
            SimStep::EndPasskeyEntry => rmk::ble::passkey::end_passkey_entry_session(),
        }
    }

    let drained = async {
        while !sender.is_empty() {
            yield_now().await;
        }
    };
    let waiting = format!("keyboard events still undrained {TIMEOUT_SECS}s after the final step");
    with_timeout(drained, &waiting).await;

    // The queue becomes empty when the keyboard receives the final event; allow
    // that in-flight processing to finish before checking state.
    Timer::after(Duration::from_millis(1)).await;
    if let Ok(report) = REPORT_CHANNEL.try_receive() {
        panic!(
            "unexpected trailing HID report after the final step: {} — add it to `expect`",
            summary(&report)
        );
    }
    let mut trailing = [0; LINK_BYTES];
    if let Ok(n) = from_device.try_read(&mut trailing) {
        panic!("unexpected {n} trailing host reply byte(s): {}", hex(&trailing[..n]));
    }
    if !pressed_inputs.is_empty() {
        let held: Vec<String> = pressed_inputs.iter().map(|pos| input(*pos)).collect();
        panic!("simulator ended with pressed inputs: {}", held.join(", "));
    }
}

/// Whether two reports say the same thing. Keycode order within a keyboard
/// report is not part of the contract; every other page compares by its wire
/// bytes, so only differences the HID descriptor can express count.
fn same_report(expected: &Report, actual: &Report) -> bool {
    match (expected, actual) {
        (Report::KeyboardReport(expected), Report::KeyboardReport(actual)) => {
            let down = |report: &KeyboardReport| {
                let mut keycodes: Vec<u8> = report.keycodes.iter().copied().filter(|code| *code != 0).collect();
                keycodes.sort_unstable();
                (report.modifier, keycodes)
            };
            down(expected) == down(actual)
        }
        _ => core::mem::discriminant(expected) == core::mem::discriminant(actual) && bytes(expected) == bytes(actual),
    }
}

fn bytes(report: &Report) -> Vec<u8> {
    use usbd_hid::descriptor::AsInputReport;

    let mut buf = [0u8; 64];
    let len = report.serialize(&mut buf).expect("serialize report");
    buf[..len].to_vec()
}

/// A keyboard report as the flat key array a scenario's `expect` writes, so a
/// mismatch can be pasted straight back into the TOML. Sorted, so expected and
/// actual read in the same order.
fn keys(modifier: u8, keycodes: &[u8]) -> String {
    let mut down: Vec<u8> = keycodes.iter().copied().filter(|code| *code != 0).collect();
    down.sort_unstable();
    // Modifier bit i is keycode LCtrl + i (HID 0xE0..=0xE7).
    let mods = (0..8u8).filter(|bit| modifier & (1 << bit) != 0);
    let names: Vec<String> = mods
        .map(|bit| HidKeyCode::LCtrl as u8 + bit)
        .chain(down)
        .map(|code| match HidKeyCode::from_repr(code) {
            Some(key) => format!("\"{key:?}\""),
            None => format!("0x{code:02x}"),
        })
        .collect();
    format!("[{}]", names.join(", "))
}

/// A report as key names, falling back to the fields of the pages that carry no
/// keycodes. Those print as the inner report, since `Report`'s own `Debug`
/// repeats the page name.
fn summary(report: &Report) -> String {
    match report {
        Report::KeyboardReport(report) => keys(report.modifier, &report.keycodes),
        Report::MouseReport(report) => format!("{report:?}"),
        Report::MediaKeyboardReport(report) => format!("{report:?}"),
        Report::SystemControlReport(report) => format!("{report:?}"),
        #[cfg(feature = "steno")]
        Report::StenoReport(report) => format!("{report:?}"),
    }
}

/// An input position in the `[row, col]` vocabulary a scenario's steps write.
fn input(pos: KeyboardEventPos) -> String {
    match pos {
        KeyboardEventPos::Key(key) => format!("key [{}, {}]", key.row, key.col),
        KeyboardEventPos::RotaryEncoder(pos) => format!("encoder {} {:?}", pos.id, pos.direction),
    }
}

/// Wire bytes as hex, capped so a full-size reply cannot bury the message.
fn hex(bytes: &[u8]) -> String {
    const SHOWN: usize = 32;
    let shown: Vec<String> = bytes.iter().take(SHOWN).map(|byte| format!("{byte:02x}")).collect();
    let mut hex = shown.join(" ");
    if bytes.len() > SHOWN {
        hex.push_str(&format!(" … ({} bytes)", bytes.len()));
    }
    hex
}

/// Await `future`, failing the step rather than hanging until the mock clock's
/// kill switch. `stalled` is the whole panic message, so it can name what the
/// step was waiting for.
async fn with_timeout<T>(future: impl Future<Output = T>, stalled: &str) -> T {
    match select(Timer::after(TIMEOUT), future).await {
        Either::First(_) => panic!("{stalled}"),
        Either::Second(value) => value,
    }
}

/// The harness's own guards: a scenario that forgets an assertion has to fail
/// rather than pass quietly.
mod self_check {
    use rmk::k;
    use rmk::test_support::test_block_on;

    use super::{HidKeyCode, SimKeyboard};

    #[test]
    #[should_panic(expected = "unexpected trailing HID report")]
    fn unasserted_trailing_report_is_rejected() {
        test_block_on(async {
            let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build().await;

            keyboard.press(0, 0).run().await;
        });
    }

    #[test]
    #[should_panic(expected = "simulator ended with pressed inputs")]
    fn unreleased_input_is_rejected() {
        test_block_on(async {
            let mut keyboard = SimKeyboard::builder([[[k!(A)]]]).build().await;

            keyboard.press(0, 0).expect_keys([HidKeyCode::A]).run().await;
        });
    }
}
