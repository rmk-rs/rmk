use std::collections::VecDeque;
use std::time::Duration;

use embassy_futures::join::join;
use rmk_types::action::KeyAction;
use rmk_types::battery::BatteryStatus;
use rmk_types::ble::BleStatus;
use rmk_types::connection::{ConnectionStatus, ConnectionType};
use rmk_types::protocol::rynk::{
    GetComboBulkResponse, GetKeymapBulkResponse, GetMorseBulkResponse, PeripheralStatus, ProtocolVersion,
    SetComboBulkRequest, SetKeymapBulkRequest, SetMorseBulkRequest,
};
use tokio::time::timeout;

use super::*;
use crate::device::RynkDevice;

/// The scripted link wrapped as a device, so tests connect through the one
/// entry point: [`RynkDevice::connect`].
struct MockDevice(Vec<Step>);

impl RynkDevice for MockDevice {
    type Read = MockRead;
    type Write = MockWrite;

    fn label(&self) -> String {
        "mock".into()
    }

    async fn open(self) -> Result<(MockRead, MockWrite), RynkHostError> {
        Ok(mock_halves(self.0, false))
    }
}

enum Step {
    /// Deliver these bytes to the reader (across one or more reads).
    Chunk(Vec<u8>),
    /// Park the reader until the writer has completed this many frame
    /// writes — paces scripted replies like real firmware. The count
    /// equals the reply SEQ, since both count requests from session start.
    AwaitWrites(usize),
    /// Park the reader forever (the script drives the session to its end).
    Hang,
}

/// A scripted link's two halves, sharing a written-frame counter (a `watch`:
/// the reader awaits it without spinning, which would starve tokio's
/// paused-clock auto-advance).
fn mock_halves(steps: Vec<Step>, fail_send: bool) -> (MockRead, MockWrite) {
    let (writes, reads) = tokio::sync::watch::channel(0usize);
    (
        MockRead {
            steps: steps.into(),
            pending: Vec::new(),
            pos: 0,
            writes: reads,
        },
        MockWrite { fail_send, writes },
    )
}

struct MockRead {
    steps: VecDeque<Step>,
    pending: Vec<u8>,
    pos: usize,
    writes: tokio::sync::watch::Receiver<usize>,
}

impl embedded_io_async::ErrorType for MockRead {
    type Error = ErrorKind;
}

impl Read for MockRead {
    async fn read(&mut self, buf: &mut [u8]) -> Result<usize, ErrorKind> {
        while self.pos >= self.pending.len() {
            // Consume a step only after it completes so a cancelled read resumes the same step.
            match self.steps.front_mut() {
                Some(Step::Chunk(c)) => {
                    self.pending = std::mem::take(c);
                    self.pos = 0;
                    self.steps.pop_front();
                }
                Some(&mut Step::AwaitWrites(n)) => {
                    self.writes.wait_for(|&c| c >= n).await.unwrap();
                    self.steps.pop_front();
                }
                Some(Step::Hang) => std::future::pending().await,
                None => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len() - self.pos);
        buf[..n].copy_from_slice(&self.pending[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

struct MockWrite {
    fail_send: bool,
    writes: tokio::sync::watch::Sender<usize>,
}

impl embedded_io_async::ErrorType for MockWrite {
    type Error = ErrorKind;
}

impl Write for MockWrite {
    async fn write(&mut self, buf: &[u8]) -> Result<usize, ErrorKind> {
        if self.fail_send {
            return Err(ErrorKind::Other);
        }
        // Skip the driver's leading lone-0x00 sync write; every other write
        // carries exactly one frame.
        if !matches!(buf, [0]) {
            self.writes.send_modify(|c| *c += 1);
        }
        Ok(buf.len())
    }

    async fn flush(&mut self) -> Result<(), ErrorKind> {
        Ok(())
    }
}

/// An unhandshaked session with a generous send budget, for tests that
/// exercise the wire without `connect`.
fn raw_session(steps: Vec<Step>) -> (Client, Driver<MockRead, MockWrite>) {
    let mut client = Client::new();
    client.capabilities.max_payload_size = 4096;
    let (reader, writer) = mock_halves(steps, false);
    (client, Driver::new(reader, writer))
}

/// Run `fut` with the driver pumping, expecting the link to outlive it.
async fn drive<F: Future>(driver: &mut Driver<MockRead, MockWrite>, client: &Client, fut: F) -> F::Output {
    match select(driver.run(client), fut).await {
        Either::First(err) => panic!("driver died during test: {err}"),
        Either::Second(v) => v,
    }
}

/// A bare frame: header + postcard(value). Used for raw headers and topics;
/// sized so tests can build multi-kilobyte bulk replies.
fn frame<T: Serialize>(cmd: Cmd, seq: u8, value: &T) -> Vec<u8> {
    let mut buf = vec![0u8; 32 * 1024];
    let n = encode_frame(&mut buf, RynkHeader { cmd, seq }, value).unwrap();
    buf.truncate(n);
    buf
}

/// An `Ok` response frame, enveloped as the firmware sends it.
fn reply<T: Serialize>(cmd: Cmd, seq: u8, value: T) -> Vec<u8> {
    frame(cmd, seq, &Ok::<T, RynkError>(value))
}

/// A topic push frame (bare payload, SEQ 0).
fn topic<T: Serialize>(cmd: Cmd, value: T) -> Vec<u8> {
    frame(cmd, 0, &value)
}

fn caps() -> DeviceCapabilities {
    DeviceCapabilities {
        num_layers: 4,
        num_rows: 6,
        num_cols: 14,
        max_combos: 8,
        max_combo_keys: 4,
        macro_space_size: 1024,
        max_morse: 4,
        max_patterns_per_key: 4,
        max_forks: 4,
        storage_enabled: true,
        max_payload_size: 256,
        macro_chunk_size: 64,
        ..Default::default()
    }
}

/// Bulk-capable caps for a single-layer 1x10 keymap. `max_bulk_keys` of 4 scales
/// down to a `spacing` of 3, so `read_all` speculates at offsets 0/3/6/9.
fn ten_key_caps() -> DeviceCapabilities {
    DeviceCapabilities {
        bulk_transfer_supported: true,
        max_bulk_keys: 4,
        num_layers: 1,
        num_rows: 1,
        num_cols: 10,
        ..caps()
    }
}

/// A bulk page of `n` key actions from `base`. `n == 0` is what a device returns
/// for a start past its last key.
fn keymap_page(base: u8, n: u8) -> GetKeymapBulkResponse {
    GetKeymapBulkResponse {
        actions: (0..n).map(|i| KeyAction::Morse(base + i)).collect(),
    }
}

/// The handshake reply pair every `connect` consumes first.
fn handshake_steps(capabilities: DeviceCapabilities) -> Vec<Step> {
    vec![
        Step::AwaitWrites(1),
        Step::Chunk(reply(Cmd::GetVersion, 1, ProtocolVersion::CURRENT)),
        Step::AwaitWrites(2),
        Step::Chunk(reply(Cmd::GetCapabilities, 2, capabilities)),
    ]
}

#[tokio::test]
async fn reply_round_trip() {
    let (client, mut driver) = raw_session(vec![
        Step::AwaitWrites(1),
        Step::Chunk(reply(Cmd::GetWpm, 1, 42u16)),
        Step::Hang,
    ]);
    let got = drive(&mut driver, &client, client.get_wpm()).await.unwrap();
    assert_eq!(got, 42);
}

#[tokio::test]
async fn rejected_response_flattens() {
    let (client, mut driver) = raw_session(vec![
        Step::AwaitWrites(1),
        Step::Chunk(frame(
            Cmd::SetDefaultLayer,
            1,
            &Err::<(), RynkError>(RynkError::Invalid),
        )),
        Step::Hang,
    ]);
    let r = drive(&mut driver, &client, client.set_default_layer(9)).await;
    assert!(matches!(r, Err(RynkHostError::Rejected(RynkError::Invalid))));
}

#[tokio::test]
async fn trailing_bytes_rejected() {
    // A reply whose payload is a valid `Ok(42)` followed by stray bytes: the host
    // must reject it rather than silently accept the prefix.
    let chunk = frame(Cmd::GetWpm, 1, &(Ok::<u16, RynkError>(42u16), [0xAAu8, 0xBB]));
    let (client, mut driver) = raw_session(vec![Step::AwaitWrites(1), Step::Chunk(chunk), Step::Hang]);
    let r = drive(&mut driver, &client, client.get_wpm()).await;
    assert!(matches!(r, Err(RynkHostError::TrailingBytes { cmd: Cmd::GetWpm })));
}

#[tokio::test]
async fn driver_resyncs_after_garbage() {
    // Undecodable bytes terminated by a delimiter arrive before the real reply;
    // the Deframer skips them and the reply still round-trips.
    let (client, mut driver) = raw_session(vec![
        Step::AwaitWrites(1),
        Step::Chunk(vec![0xDE, 0xAD, 0xBE, 0xEF, 0x00]),
        Step::Chunk(reply(Cmd::GetWpm, 1, 42u16)),
        Step::Hang,
    ]);
    let got = drive(&mut driver, &client, client.get_wpm()).await.unwrap();
    assert_eq!(got, 42);
}

#[tokio::test]
async fn multi_kilobyte_reply_round_trips() {
    // A reply spanning many transport reads must reassemble within the fixed
    // RX buffer instead of being dropped as overflow.
    let mut supported = caps();
    supported.bulk_transfer_supported = true;
    supported.max_bulk_keys = 255;
    let big = GetKeymapBulkResponse {
        actions: (0..4000u32).map(|i| KeyAction::Morse((i % 256) as u8)).collect(),
    };
    let (client, mut driver) = connect_session(
        handshake_steps(supported),
        vec![
            Step::AwaitWrites(3),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 3, big.clone())),
            Step::Hang,
        ],
    )
    .await;
    let got = drive(&mut driver, &client, client.get_keymap_bulk(0, 0, 0))
        .await
        .unwrap();
    assert_eq!(got, big);
}

#[tokio::test]
async fn unknown_topic_skipped() {
    let mut chunk = frame(Cmd::from_raw(0x80ff), 0, &[1u8, 2, 3]);
    chunk.extend_from_slice(&topic(Cmd::LayerChange, 3u8));
    let (client, mut driver) = raw_session(vec![Step::Chunk(chunk), Step::Hang]);
    let ev = drive(&mut driver, &client, client.next_topic()).await;
    assert!(matches!(ev, TopicEvent::LayerChange(3)));
}

#[tokio::test]
async fn unknown_response_cmd_mismatch_detected() {
    let (client, mut driver) = raw_session(vec![
        Step::AwaitWrites(1),
        Step::Chunk(reply(Cmd::from_raw(0x7fff), 1, 42u16)),
        Step::Hang,
    ]);
    let r = drive(&mut driver, &client, client.get_wpm()).await;
    assert!(matches!(
        r,
        Err(RynkHostError::CmdMismatch {
            sent: Cmd::GetWpm,
            got,
        }) if got == Cmd::from_raw(0x7fff)
    ));
}

#[tokio::test]
async fn eof_ends_driver() {
    let (client, mut driver) = raw_session(vec![]);
    match select(driver.run(&client), client.get_wpm()).await {
        Either::First(err) => assert!(matches!(err, RynkHostError::Disconnected)),
        Either::Second(r) => panic!("request should not resolve on a dead link: {r:?}"),
    }
}

#[tokio::test]
async fn send_failure_ends_driver() {
    let mut client = Client::new();
    client.capabilities.max_payload_size = 4096;
    let (reader, writer) = mock_halves(vec![Step::Hang], true);
    let mut driver = Driver::new(reader, writer);
    match select(driver.run(&client), client.get_wpm()).await {
        Either::First(err) => assert!(matches!(err, RynkHostError::Io(_))),
        Either::Second(r) => panic!("request should not resolve on a dead link: {r:?}"),
    }
}

#[tokio::test]
async fn request_and_next_topic_run_full_duplex() {
    let mut chunk = topic(Cmd::LayerChange, 7u8);
    chunk.extend_from_slice(&reply(Cmd::GetWpm, 1, 42u16));
    let (client, mut driver) = raw_session(vec![Step::AwaitWrites(1), Step::Chunk(chunk), Step::Hang]);
    let (wpm, ev) = drive(&mut driver, &client, join(client.get_wpm(), client.next_topic())).await;
    assert_eq!(wpm.unwrap(), 42);
    assert!(matches!(ev, TopicEvent::LayerChange(7)));
}

#[tokio::test]
async fn stale_seq_reply_dropped() {
    let mut chunk = reply(Cmd::GetWpm, 0xEE, 99u16);
    chunk.extend_from_slice(&reply(Cmd::GetWpm, 1, 42u16));
    let (client, mut driver) = raw_session(vec![Step::AwaitWrites(1), Step::Chunk(chunk), Step::Hang]);
    let got = drive(&mut driver, &client, client.get_wpm()).await.unwrap();
    assert_eq!(got, 42);
}

#[tokio::test(start_paused = true)]
async fn stale_reply_of_cancelled_request_is_dropped() {
    // Request 1 is cancelled mid-wait: its slot frees, its late reply matches
    // no slot and is dropped, and request 2 round-trips unobstructed.
    let (client, mut driver) = raw_session(vec![
        Step::AwaitWrites(2),
        Step::Chunk(reply(Cmd::GetWpm, 1, 99u16)),
        Step::Chunk(reply(Cmd::GetWpm, 2, 42u16)),
        Step::Hang,
    ]);
    let got = drive(&mut driver, &client, async {
        let cancelled = timeout(Duration::from_millis(10), client.get_wpm()).await;
        assert!(cancelled.is_err(), "request 1 should time out");
        client.get_wpm().await
    })
    .await
    .unwrap();
    assert_eq!(got, 42);
}

#[tokio::test]
async fn concurrent_requests_route_out_of_order_replies() {
    // Both requests hit the wire before any reply (the reader parks until two
    // writes complete), and the replies return out of order: SEQ routing must
    // still deliver each to its caller.
    let mut replies = reply(Cmd::GetSleepState, 2, true);
    replies.extend_from_slice(&reply(Cmd::GetWpm, 1, 42u16));
    let (client, mut driver) = raw_session(vec![Step::AwaitWrites(2), Step::Chunk(replies), Step::Hang]);
    let (wpm, sleeping) = drive(&mut driver, &client, join(client.get_wpm(), client.get_sleep_state())).await;
    assert_eq!(wpm.unwrap(), 42);
    assert!(sleeping.unwrap());
}

#[tokio::test]
async fn fire_and_forget_orphan_reply_is_dropped() {
    // On targets where the reset is a no-op the firmware answers `Reboot`;
    // that echo matches no slot and is dropped while the next request runs.
    let (client, mut driver) = raw_session(vec![
        Step::AwaitWrites(1),
        Step::Chunk(reply(Cmd::Reboot, 1, ())),
        Step::AwaitWrites(2),
        Step::Chunk(reply(Cmd::GetWpm, 2, 42u16)),
        Step::Hang,
    ]);
    let got = drive(&mut driver, &client, async {
        client.reboot().await.unwrap();
        client.get_wpm().await
    })
    .await
    .unwrap();
    assert_eq!(got, 42);
}

#[tokio::test]
async fn topic_queue_overflow_drops_oldest() {
    let mut chunk = Vec::new();
    for i in 0..=TOPIC_QUEUE_CAPACITY as u8 {
        chunk.extend_from_slice(&topic(Cmd::LayerChange, i));
    }
    let (client, mut driver) = raw_session(vec![Step::Chunk(chunk), Step::Hang]);
    let first = drive(&mut driver, &client, client.next_topic()).await;
    assert!(matches!(first, TopicEvent::LayerChange(1)), "oldest topic evicted");
}

#[tokio::test]
async fn next_topic_decodes_typed_payload() {
    let status = ConnectionStatus {
        ble: BleStatus {
            bonded: true,
            ..Default::default()
        },
        preferred: ConnectionType::Ble,
        ..Default::default()
    };
    let (client, mut driver) = raw_session(vec![Step::Chunk(topic(Cmd::ConnectionChange, status)), Step::Hang]);
    let ev = drive(&mut driver, &client, client.next_topic()).await;
    match ev {
        TopicEvent::ConnectionChange(s) => {
            assert_eq!(s.preferred, ConnectionType::Ble);
            assert!(s.ble.bonded);
        }
        other => panic!("expected ConnectionChange, got {other:?}"),
    }
}

async fn connect_session(mut steps: Vec<Step>, trailing: Vec<Step>) -> (Client, Driver<MockRead, MockWrite>) {
    steps.extend(trailing);
    MockDevice(steps).connect().await.expect("connect should succeed")
}

#[tokio::test]
async fn capability_gate_rejects_without_wire_send() {
    let (client, _driver) = connect_session(handshake_steps(caps()), vec![Step::Hang]).await;
    assert!(!client.capabilities.ble_enabled);
    let r = client.get_battery_status().await;
    assert!(matches!(r, Err(RynkHostError::Unsupported(Cmd::GetBatteryStatus, _))));
}

#[tokio::test]
async fn wired_split_peripheral_status_is_supported() {
    let status = PeripheralStatus {
        connected: true,
        battery: BatteryStatus::Unavailable,
    };
    let mut capabilities = caps();
    capabilities.is_split = true;
    capabilities.ble_enabled = false;
    let (client, mut driver) = connect_session(
        handshake_steps(capabilities),
        vec![
            Step::AwaitWrites(3),
            Step::Chunk(reply(Cmd::GetPeripheralStatus, 3, status)),
            Step::Hang,
        ],
    )
    .await;
    assert_eq!(
        drive(&mut driver, &client, client.get_peripheral_status(0))
            .await
            .unwrap(),
        status
    );
}

#[tokio::test]
async fn oversized_request_rejected_locally() {
    let mut tiny = caps();
    tiny.max_payload_size = 4;
    let (client, _driver) = connect_session(handshake_steps(tiny), vec![Step::Hang]).await;
    let r = client.set_key(0, 0, 0, KeyAction::Morse(3)).await;
    assert!(matches!(r, Err(RynkHostError::Encode(Cmd::SetKeyAction))));
}

#[tokio::test]
async fn bulk_methods_gate_without_wire_send() {
    let (client, _driver) = connect_session(handshake_steps(caps()), vec![Step::Hang]).await;
    assert!(!client.capabilities.bulk_transfer_supported);

    let keymap_req = SetKeymapBulkRequest {
        layer: 0,
        start_row: 0,
        start_col: 0,
        actions: Default::default(),
    };
    let combo_req = SetComboBulkRequest {
        start_index: 0,
        configs: Default::default(),
    };
    let morse_req = SetMorseBulkRequest {
        start_index: 0,
        configs: Default::default(),
    };

    assert!(matches!(
        client.get_keymap_bulk(0, 0, 0).await,
        Err(RynkHostError::Unsupported(Cmd::GetKeymapBulk, _))
    ));
    assert!(matches!(
        client.set_keymap_bulk(keymap_req).await,
        Err(RynkHostError::Unsupported(Cmd::SetKeymapBulk, _))
    ));
    assert!(matches!(
        client.get_combo_bulk(0).await,
        Err(RynkHostError::Unsupported(Cmd::GetComboBulk, _))
    ));
    assert!(matches!(
        client.set_combo_bulk(combo_req).await,
        Err(RynkHostError::Unsupported(Cmd::SetComboBulk, _))
    ));
    assert!(matches!(
        client.get_morse_bulk(0).await,
        Err(RynkHostError::Unsupported(Cmd::GetMorseBulk, _))
    ));
    assert!(matches!(
        client.set_morse_bulk(morse_req).await,
        Err(RynkHostError::Unsupported(Cmd::SetMorseBulk, _))
    ));
}

#[tokio::test]
async fn bulk_methods_round_trip_when_supported() {
    let mut supported = caps();
    supported.bulk_transfer_supported = true;
    supported.max_bulk_keys = 8;

    let keymap_resp = GetKeymapBulkResponse {
        actions: Default::default(),
    };
    let combo_resp = GetComboBulkResponse {
        configs: Default::default(),
    };
    let morse_resp = GetMorseBulkResponse {
        configs: Default::default(),
    };
    let (client, mut driver) = connect_session(
        handshake_steps(supported),
        vec![
            Step::AwaitWrites(3),
            Step::Chunk(reply(Cmd::SetKeymapBulk, 3, ())),
            Step::AwaitWrites(4),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 4, keymap_resp.clone())),
            Step::AwaitWrites(5),
            Step::Chunk(reply(Cmd::SetComboBulk, 5, ())),
            Step::AwaitWrites(6),
            Step::Chunk(reply(Cmd::GetComboBulk, 6, combo_resp.clone())),
            Step::AwaitWrites(7),
            Step::Chunk(reply(Cmd::SetMorseBulk, 7, ())),
            Step::AwaitWrites(8),
            Step::Chunk(reply(Cmd::GetMorseBulk, 8, morse_resp.clone())),
            Step::Hang,
        ],
    )
    .await;

    drive(&mut driver, &client, async {
        client
            .set_keymap_bulk(SetKeymapBulkRequest {
                layer: 0,
                start_row: 0,
                start_col: 0,
                actions: Default::default(),
            })
            .await
            .unwrap();
        assert_eq!(client.get_keymap_bulk(0, 0, 0).await.unwrap(), keymap_resp);

        client
            .set_combo_bulk(SetComboBulkRequest {
                start_index: 0,
                configs: Default::default(),
            })
            .await
            .unwrap();
        assert_eq!(client.get_combo_bulk(0).await.unwrap(), combo_resp);

        client
            .set_morse_bulk(SetMorseBulkRequest {
                start_index: 0,
                configs: Default::default(),
            })
            .await
            .unwrap();
        assert_eq!(client.get_morse_bulk(0).await.unwrap(), morse_resp);
    })
    .await;
}

#[tokio::test]
async fn read_all_keymap_concatenates_pages() {
    // Full pages overlap the speculative offsets; reassembly trims the overlaps.
    let (client, mut driver) = connect_session(
        handshake_steps(ten_key_caps()),
        vec![
            Step::AwaitWrites(6),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 3, keymap_page(0, 4))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 4, keymap_page(3, 4))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 5, keymap_page(6, 4))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 6, keymap_page(9, 1))),
            Step::AwaitWrites(7),
            Step::Chunk(reply(Cmd::GetWpm, 7, 42u16)),
            Step::Hang,
        ],
    )
    .await;
    drive(&mut driver, &client, async {
        assert_eq!(client.read_all_keymap().await.unwrap(), keymap_page(0, 10).actions);
        // The distinct trailing command detects an unexpected fifth fetch.
        assert_eq!(client.get_wpm().await.unwrap(), 42);
    })
    .await;
}

#[tokio::test]
async fn read_all_refetches_a_squeezed_page() {
    // A concurrent bulk write can squeeze a reply below the 3-item window without
    // emptying it. Reading the shortfall as the end of the keymap would silently
    // return 2 keys instead of 10, so the lane re-fetches where its short page stopped.
    let (client, mut driver) = connect_session(
        handshake_steps(ten_key_caps()),
        vec![
            Step::AwaitWrites(6),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 3, keymap_page(0, 2))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 4, keymap_page(3, 4))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 5, keymap_page(6, 4))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 6, keymap_page(9, 1))),
            Step::AwaitWrites(7),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 7, keymap_page(2, 4))),
            Step::AwaitWrites(8),
            Step::Chunk(reply(Cmd::GetWpm, 8, 42u16)),
            Step::Hang,
        ],
    )
    .await;
    drive(&mut driver, &client, async {
        assert_eq!(client.read_all_keymap().await.unwrap(), keymap_page(0, 10).actions);
        // Only the squeezed window re-fetches; the other three stay at one fetch each.
        assert_eq!(client.get_wpm().await.unwrap(), 42);
    })
    .await;
}

#[tokio::test]
async fn read_all_stops_on_clamped_empty_page() {
    // Speculative offsets 0/3/6/9 are all fetched before the empty page is
    // seen; the clamp shows up as result truncation, not as skipped fetches.
    let (client, mut driver) = connect_session(
        handshake_steps(ten_key_caps()),
        vec![
            Step::AwaitWrites(6),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 3, keymap_page(0, 4))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 4, keymap_page(0, 0))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 5, keymap_page(0, 0))),
            Step::Chunk(reply(Cmd::GetKeymapBulk, 6, keymap_page(0, 0))),
            Step::AwaitWrites(7),
            Step::Chunk(reply(Cmd::GetWpm, 7, 7u16)),
            Step::Hang,
        ],
    )
    .await;
    drive(&mut driver, &client, async {
        assert_eq!(client.read_all_keymap().await.unwrap().len(), 4);
        // The distinct trailing command pins the fetch count to the
        // speculative offsets alone.
        assert_eq!(client.get_wpm().await.unwrap(), 7);
    })
    .await;
}

#[tokio::test]
async fn write_all_keymap_packs_pages_to_the_payload_budget() {
    let mut supported = caps();
    supported.bulk_transfer_supported = true;
    // Pages pack by encoded size, not by `max_bulk_keys`: 3 fixed bytes +
    // 1 count varint + 2 bytes per `KeyAction::Morse` — an 8-byte budget
    // holds exactly 2 keys, so 5 keys page as 2+2+1.
    supported.max_payload_size = 8;

    let (client, mut driver) = connect_session(
        handshake_steps(supported),
        vec![
            Step::AwaitWrites(3),
            Step::Chunk(reply(Cmd::SetKeymapBulk, 3, ())),
            Step::AwaitWrites(4),
            Step::Chunk(reply(Cmd::SetKeymapBulk, 4, ())),
            Step::AwaitWrites(5),
            Step::Chunk(reply(Cmd::SetKeymapBulk, 5, ())),
            Step::AwaitWrites(6),
            Step::Chunk(reply(Cmd::GetWpm, 6, 99u16)),
            Step::Hang,
        ],
    )
    .await;
    drive(&mut driver, &client, async {
        let actions: Vec<KeyAction> = (0u8..5).map(KeyAction::Morse).collect();
        client.write_all_keymap(actions).await.unwrap();
        assert_eq!(client.get_wpm().await.unwrap(), 99);
    })
    .await;
}

#[tokio::test]
async fn connect_rejects_newer_major() {
    let newer = ProtocolVersion {
        major: ProtocolVersion::CURRENT.major + 1,
        minor: 0,
    };
    // The firmware answers both handshake requests (they ride one round
    // trip); the version gate must still reject before caps are released.
    let err = MockDevice(vec![
        Step::AwaitWrites(2),
        Step::Chunk(reply(Cmd::GetVersion, 1, newer)),
        Step::Chunk(reply(Cmd::GetCapabilities, 2, caps())),
        Step::Hang,
    ])
    .connect()
    .await
    .err()
    .expect("connect must fail");
    assert!(matches!(err, RynkHostError::VersionMismatch { .. }));
}

#[tokio::test]
async fn connect_accepts_newer_minor() {
    let newer = ProtocolVersion {
        major: ProtocolVersion::CURRENT.major,
        minor: ProtocolVersion::CURRENT.minor + 1,
    };
    MockDevice(vec![
        Step::AwaitWrites(1),
        Step::Chunk(reply(Cmd::GetVersion, 1, newer)),
        Step::AwaitWrites(2),
        Step::Chunk(reply(Cmd::GetCapabilities, 2, caps())),
        Step::Hang,
    ])
    .connect()
    .await
    .expect("same-major newer-minor must connect");
}

#[tokio::test(start_paused = true)]
async fn caller_can_timeout_silent_connect() {
    let err = timeout(Duration::from_millis(10), MockDevice(vec![Step::Hang]).connect()).await;
    assert!(err.is_err());
}

#[tokio::test]
async fn rynk_device_trait_drives_lifecycle() {
    // Generic `RynkDevice` consumers should not name the transport type.
    async fn run_first<D: RynkDevice>(d: D) -> u16 {
        assert_eq!(d.label(), "mock");
        let (client, mut driver) = d.connect().await.unwrap();
        assert_eq!(client.capabilities.num_cols, 14);
        match select(driver.run(&client), client.get_wpm()).await {
            Either::First(err) => panic!("driver died: {err}"),
            Either::Second(wpm) => wpm.unwrap(),
        }
    }

    let mut steps = handshake_steps(caps());
    steps.extend([
        Step::AwaitWrites(3),
        Step::Chunk(reply(Cmd::GetWpm, 3, 7u16)),
        Step::Hang,
    ]);
    assert_eq!(run_first(MockDevice(steps)).await, 7);
}
