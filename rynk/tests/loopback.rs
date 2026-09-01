//! Cross-stack end-to-end test: the real [`rynk::Client`] against the real
//! firmware [`rmk::host::HostService::run_session`] over an in-memory duplex.
//!
//! Every other Rynk test mocks one half; this one runs both production halves,
//! locking the conventions the shared `rmk-types` codec alone does not pin:
//! the version handshake, the advertised `max_payload_size`, seq correlation
//! and cmd echo, the `Result<T, RynkError>` response envelope, and topic push
//! decoding.
//!
//! It runs on tokio: `run_session` is executor-agnostic and the rynk path
//! reads no clock, so no embassy-time pump is needed.

use embassy_futures::select::{Either, select};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::pipe::Pipe;
use rmk::config::{BehaviorConfig, PositionalConfig, RmkConfig};
use rmk::event::{LayerChangeEvent, publish_event};
use rmk::host::HostService as RynkService;
use rmk::keymap::{KeyMap, KeymapData};
use rmk_types::action::KeyAction;
use rmk_types::combo::Combo;
use rmk_types::constants::{MACRO_DATA_SIZE, RYNK_BUFFER_SIZE};
use rmk_types::protocol::rynk::{MacroData, ProtocolVersion, RYNK_MAX_PAYLOAD_SIZE, RynkError, StorageResetMode};
use rynk::layout::{Key, Rect, Variant};
use rynk::{Client, LayoutInfo, RynkDevice, RynkHostError, TopicEvent};

/// One direction of the in-memory link. Sized to a full Rynk buffer so any
/// single legal frame fits without the writer blocking on an un-polled reader.
type Link = Pipe<NoopRawMutex, RYNK_BUFFER_SIZE>;

/// The host side of the two pipes as a device — reads device→host, writes
/// host→device — so the test connects through [`RynkDevice::connect`]. `&Pipe`
/// implements the embedded-io traits itself; both sides use it directly.
struct DuplexDevice<'p> {
    rx: &'p Link,
    tx: &'p Link,
}

impl<'p> RynkDevice for DuplexDevice<'p> {
    type Read = &'p Link;
    type Write = &'p Link;

    fn label(&self) -> String {
        "loopback".into()
    }

    async fn open(self) -> Result<(&'p Link, &'p Link), RynkHostError> {
        Ok((self.rx, self.tx))
    }
}

/// Connect over the duplex and run `script` with the driver pumping; panics if
/// the link dies before the script finishes.
async fn with_session(device: DuplexDevice<'_>, script: impl AsyncFnOnce(&Client)) {
    let (client, mut driver) = device.connect().await.expect("handshake should succeed");
    match select(driver.run(&client), script(&client)).await {
        Either::First(err) => panic!("link died before the script finished: {err}"),
        Either::Second(()) => {}
    }
}

#[tokio::test(flavor = "current_thread")]
async fn client_against_run_session() {
    // Firmware side.
    let mut behavior = BehaviorConfig::default();
    let positional: PositionalConfig<2, 2> = PositionalConfig::default();
    let mut data: KeymapData<2, 2, 2, 0> = KeymapData::new([[[KeyAction::No; 2]; 2]; 2]);
    let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;
    // Keep the lock gate open; lock behavior is covered elsewhere.
    let mut config: RmkConfig<'static> = RmkConfig::default();
    config.lock_config.insecure = true;

    // Built-in physical-layout blob; leaked to match firmware's `'static` config.
    let k = |row: u8, col: u8, x: f32| Key {
        row,
        col,
        rect: Rect {
            x,
            y: 0.5,
            w: 1.0,
            h: 1.0,
        },
        r: 0.0,
        rect2: None,
        pivot: None,
    };
    let layout_info = LayoutInfo {
        default_variant: 1,
        variants: vec![
            Variant {
                name: "a".into(),
                keys: vec![k(0, 0, 0.5), k(0, 1, 1.5), k(0, 2, 2.5)],
                encoders: vec![],
            },
            Variant {
                name: "b".into(),
                keys: vec![k(0, 0, 0.5), k(0, 2, 1.5)],
                encoders: vec![],
            },
        ],
    };
    let raw = postcard::to_allocvec(&layout_info).unwrap();
    let blob: &'static [u8] = Box::leak(miniz_oxide::deflate::compress_to_vec(&raw, 6).into_boxed_slice());
    config.layout_blob = blob;
    let service = RynkService::new(&keymap, &config);

    // In-memory duplex: h2d requests, d2h responses/topics.
    let h2d = Link::new();
    let d2h = Link::new();
    let mut dev_rx: &Link = &h2d;
    let mut dev_tx: &Link = &d2h;

    let device = DuplexDevice { rx: &d2h, tx: &h2d };

    // Host side.
    let script = with_session(device, async |client| {
        // Version handshake crosses both real stacks.
        assert_eq!(client.get_version().await.unwrap(), ProtocolVersion::CURRENT);

        let caps = client.get_capabilities().await.unwrap();
        assert_eq!((caps.num_layers, caps.num_rows, caps.num_cols), (2, 2, 2));
        // Client consumes the firmware-advertised payload limit.
        assert_eq!(caps.max_payload_size as usize, RYNK_MAX_PAYLOAD_SIZE);

        let info = client.get_device_info().await.unwrap();
        assert_eq!(info.manufacturer.as_str(), "RMK");
        assert_eq!(info.product_name.as_str(), "RMK Keyboard");

        // Get round-trip: seq correlation, cmd echo, Ok envelope.
        assert_eq!(client.get_current_layer().await.unwrap(), 0);

        // Get with request payload and typed response decode.
        assert_eq!(client.get_key(0, 0, 0).await.unwrap(), KeyAction::No);

        // Set + readback through the real persistence path. The host-driven
        // default-layer change also surfaces on the topic stream.
        client.set_default_layer(1).await.unwrap();
        assert_eq!(client.get_default_layer().await.unwrap(), 1);
        let ev = client.next_topic().await;
        assert!(
            matches!(ev, TopicEvent::LayerChange(1)),
            "expected LayerChange(1), got {ev:?}"
        );

        // Round-trip representative remaining domains.
        client.set_key(0, 1, 1, KeyAction::Morse(2)).await.unwrap();
        assert_eq!(client.get_key(0, 1, 1).await.unwrap(), KeyAction::Morse(2));

        let mut beh = client.get_behavior().await.unwrap();
        beh.combo_timeout_ms = beh.combo_timeout_ms.wrapping_add(7);
        beh.tap_interval_ms = beh.tap_interval_ms.wrapping_add(3);
        beh.morse_default_profile = beh.morse_default_profile.with_enable_flow_tap(Some(true));
        beh.morse_prior_idle_time_ms = beh.morse_prior_idle_time_ms.wrapping_add(5);
        client.set_behavior(beh).await.unwrap();
        assert_eq!(client.get_behavior().await.unwrap(), beh);

        // Macro zero-fill chunk contract.
        let mut macro_bytes: heapless::Vec<u8, MACRO_DATA_SIZE> = heapless::Vec::new();
        macro_bytes.extend_from_slice(&[1, 2, 3, 4]).unwrap();
        client.set_macro(0, MacroData { data: macro_bytes }).await.unwrap();
        let got = client.get_macro(0).await.unwrap();
        assert_eq!(got.data.len(), caps.macro_chunk_size as usize, "reply is a full chunk");
        assert_eq!(&got.data[..4], &[1, 2, 3, 4], "written prefix preserved");
        assert!(got.data[4..].iter().all(|&b| b == 0), "tail zero-filled past the write");

        // Combo round-trip, guarded on advertised count.
        if caps.max_combos > 0 {
            let combo = Combo::new([KeyAction::Morse(1), KeyAction::Morse(2)], KeyAction::Morse(3), Some(0));
            client.set_combo(0, combo.clone()).await.unwrap();
            assert_eq!(client.get_combo(0).await.unwrap(), combo);
        }

        let _ = client.get_wpm().await.unwrap();
        let _ = client.get_sleep_state().await.unwrap();
        let _ = client.get_connection_type().await.unwrap();
        let _ = client.get_led_indicator().await.unwrap();

        // Device rejection flattens to RynkHostError::Rejected.
        let rejected = client.storage_reset(StorageResetMode::LayoutOnly).await;
        assert!(
            matches!(rejected, Err(RynkHostError::Rejected(RynkError::Unimplemented))),
            "expected Rejected(Unimplemented), got {rejected:?}"
        );

        // Layout blob crosses serve, page, inflate, and decode.
        let layout = client.get_layout().await.unwrap();
        assert_eq!(
            layout, layout_info,
            "the layout blob round-trips through the real stack"
        );
        assert_eq!(layout.variants.len(), 2, "two render variants");
        assert_eq!(layout.variants[1].keys.len(), 2, "variant b hides one key");

        publish_event(LayerChangeEvent::new(3));
        let ev = client.next_topic().await;
        assert!(
            matches!(ev, TopicEvent::LayerChange(3)),
            "expected LayerChange(3), got {ev:?}"
        );
    });

    // Drain flash writes; the session should not finish before the script.
    let device = select(
        service.run_session(&mut dev_rx, &mut dev_tx),
        rmk::channel::drain_flash_channel_for_test(),
    );
    match select(device, script).await {
        Either::First(_) => panic!("run_session ended before the client script finished"),
        Either::Second(()) => {}
    }
}

/// The lock gate end to end: a locked device refuses a gated command with
/// `RynkError::Locked` flattened to [`RynkHostError::Rejected`], advertises its
/// challenge over the wire, and serves the three lock endpoints. The key-hold →
/// unlock transition needs matrix simulation and is covered by the firmware
/// `host::lock` / `host::rynk` tests.
#[tokio::test(flavor = "current_thread")]
async fn lock_gate_rejects_and_reports() {
    let mut behavior = BehaviorConfig::default();
    let positional: PositionalConfig<2, 2> = PositionalConfig::default();
    let mut data: KeymapData<2, 2, 1, 0> = KeymapData::new([[[KeyAction::No; 2]; 2]; 1]);
    let keymap = KeyMap::new(&mut data, &mut behavior, &positional).await;

    // Challenge configured but never held (the test can't drive the matrix), so
    // the device stays locked throughout.
    const UNLOCK_KEYS: &[(u8, u8)] = &[(0, 0)];
    let mut config: RmkConfig<'static> = RmkConfig::default();
    config.lock_config.unlock_keys = UNLOCK_KEYS;
    let service = RynkService::new(&keymap, &config);

    let h2d = Link::new();
    let d2h = Link::new();
    let mut dev_rx: &Link = &h2d;
    let mut dev_tx: &Link = &d2h;
    let device = DuplexDevice { rx: &d2h, tx: &h2d };

    let script = with_session(device, async |client| {
        // GetLockStatus is open and advertises the challenge across the wire
        // (the `heapless::Vec<(u8,u8)>` round-trips intact).
        let status = client.get_lock_status().await.unwrap();
        assert!(status.locked);
        assert!(!status.unlocking);
        assert_eq!(status.key_positions.as_slice(), &[(0, 0)]);

        // A hard-locked command flattens `RynkError::Locked` to `Rejected` end to end.
        let gated = client.get_matrix_state().await;
        assert!(
            matches!(gated, Err(RynkHostError::Rejected(RynkError::Locked))),
            "expected Rejected(Locked), got {gated:?}"
        );

        // Polling arms the attempt but cannot unlock without the configured key.
        let polled = client.unlock_poll().await.unwrap();
        assert!(polled.locked);
        assert!(polled.unlocking);
        assert_eq!(polled.remaining_keys, 1);

        // Lock is always dispatchable.
        client.lock().await.unwrap();
    });

    let device = select(
        service.run_session(&mut dev_rx, &mut dev_tx),
        rmk::channel::drain_flash_channel_for_test(),
    );
    match select(device, script).await {
        Either::First(_) => panic!("run_session ended before the client script finished"),
        Either::Second(()) => {}
    }
}

/// A silent peer must keep `connect` pending: the client is runtime-free and
/// owns no handshake timeout — callers wrap one. Inherited from the retired
/// serial transport's PTY test.
#[tokio::test(flavor = "current_thread")]
async fn connect_stays_pending_on_silent_peer() {
    let h2d = Link::new();
    let d2h = Link::new();
    let device = DuplexDevice { rx: &d2h, tx: &h2d };

    let timed_out = tokio::time::timeout(std::time::Duration::from_millis(200), device.connect()).await;
    assert!(timed_out.is_err(), "connect must not resolve against a silent peer");
}
