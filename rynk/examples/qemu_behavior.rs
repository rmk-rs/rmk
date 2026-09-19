//! Strict Rynk behavior verifier for the deterministic QEMU fixture.
//!
//! Unlike `hw_test`, this is not a generic hardware smoke test. It assumes the
//! fixture in `examples/use_rust/qemu-riscv-rynk` and asserts exact protocol
//! behavior: expected capabilities, known initial state, mutations, restore
//! paths, and firmware-side rejection behavior.

use std::fmt::Debug;
use std::time::Duration;

use embassy_futures::select::{Either, select};
use embedded_io_adapters::tokio_1::FromTokio;
use rynk::rmk_types::action::{Action, EncoderAction, KeyAction};
use rynk::rmk_types::ble::{BleState, BleStatus};
use rynk::rmk_types::combo::Combo;
use rynk::rmk_types::connection::{ConnectionStatus, ConnectionType, UsbState};
use rynk::rmk_types::fork::{Fork, StateBits};
use rynk::rmk_types::keycode::{HidKeyCode, KeyCode};
use rynk::rmk_types::led_indicator::LedIndicator;
use rynk::rmk_types::modifier::ModifierCombination;
use rynk::rmk_types::morse::{Morse, MorseProfile};
use rynk::rmk_types::protocol::rynk::{MacroData, ProtocolVersion, RynkError, StorageResetMode};
use rynk::{Client, LayoutInfo, RynkDevice, RynkHostError, TopicEvent};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(1);
const DEFAULT_ADDR: &str = "127.0.0.1:9000";

/// The QEMU TCP serial as a device, connecting through [`RynkDevice::connect`].
struct TcpDevice(TcpStream);

impl RynkDevice for TcpDevice {
    type Read = FromTokio<OwnedReadHalf>;
    type Write = FromTokio<OwnedWriteHalf>;

    fn label(&self) -> String {
        "qemu".into()
    }

    async fn open(self) -> Result<(Self::Read, Self::Write), RynkHostError> {
        let (reader, writer) = self.0.into_split();
        Ok((FromTokio::new(reader), FromTokio::new(writer)))
    }
}

fn key(code: HidKeyCode) -> KeyAction {
    KeyAction::Single(Action::Key(KeyCode::Hid(code)))
}

fn encoder(clockwise: HidKeyCode, counter_clockwise: HidKeyCode) -> EncoderAction {
    EncoderAction::new(key(clockwise), key(counter_clockwise))
}

fn fixture_fork(positive_output: KeyAction) -> Fork {
    Fork::new(
        key(HidKeyCode::A),
        key(HidKeyCode::B),
        positive_output,
        StateBits::default(),
        StateBits::default(),
        ModifierCombination::default(),
        true,
    )
}

fn empty_morse(profile: MorseProfile) -> Morse {
    Morse {
        profile,
        actions: heapless::LinearMap::new(),
    }
}

fn expect_rejected<T: Debug>(label: &str, res: Result<T, RynkHostError>, expected: RynkError) {
    match res {
        Err(RynkHostError::Rejected(actual)) if actual == expected => {}
        other => panic!("{label}: expected Rejected({expected:?}), got {other:?}"),
    }
}

fn expect_unsupported<T: Debug>(label: &str, res: Result<T, RynkHostError>) {
    match res {
        Err(RynkHostError::Unsupported(_, _)) => {}
        other => panic!("{label}: expected Unsupported, got {other:?}"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::args().nth(1).unwrap_or_else(|| DEFAULT_ADDR.into());
    let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addr))
        .await
        .map_err(|_| format!("connect timed out to QEMU TCP serial at {addr}"))??;
    stream.set_nodelay(true)?;
    let (client, mut driver) = tokio::time::timeout(CONNECT_TIMEOUT, TcpDevice(stream).connect())
        .await
        .map_err(|_| format!("Rynk handshake timed out over QEMU TCP serial at {addr}"))??;

    match select(driver.run(&client), script(&client)).await {
        Either::First(err) => Err(format!("link died during the script: {err}").into()),
        Either::Second(res) => res,
    }
}

async fn script(client: &Client) -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(client.get_version().await?, ProtocolVersion::CURRENT);
    let caps = client.get_capabilities().await?;
    assert_eq!((caps.num_layers, caps.num_rows, caps.num_cols), (2, 3, 3));
    assert_eq!(caps.num_encoders, 1);
    assert_eq!(caps.max_combos, 8);
    assert_eq!(caps.max_combo_keys, 4);
    assert_eq!(caps.macro_space_size, 256);
    assert_eq!(caps.macro_chunk_size, 64);
    assert_eq!(caps.max_morse, 8);
    assert_eq!(caps.max_patterns_per_key, 8);
    assert_eq!(caps.max_forks, 8);
    assert!(!caps.storage_enabled);
    assert!(!caps.ble_enabled);
    assert!(!caps.is_split);
    assert!(caps.bulk_transfer_supported);
    assert!(caps.max_bulk_keys > 0);
    assert!(caps.max_bulk_items > 0);

    assert_eq!(client.get_capabilities().await?, caps);

    // Concurrent requests must both complete: replies route by SEQ, not send
    // order. Bounded so a routing regression fails instead of hanging.
    let (wpm, sleeping) = tokio::time::timeout(REQUEST_TIMEOUT, async {
        let (wpm, sleeping) = tokio::join!(client.get_wpm(), client.get_sleep_state());
        Ok::<_, RynkHostError>((wpm?, sleeping?))
    })
    .await
    .map_err(|_| "concurrent get_wpm + get_sleep_state timed out over QEMU UART")??;
    assert_eq!(wpm, 0);
    assert!(!sleeping);

    assert_eq!(client.get_default_layer().await?, 0);
    client.set_default_layer(1).await?;
    assert_eq!(client.get_default_layer().await?, 1);
    expect_rejected(
        "set_default_layer out of range",
        client.set_default_layer(2).await,
        RynkError::Invalid,
    );
    client.set_default_layer(0).await?;
    assert_eq!(client.get_default_layer().await?, 0);

    // Host-driven default-layer changes surface on the topic stream (the
    // activated layer follows the default); the rejected set publishes nothing
    // in between.
    for expected in [1u8, 0] {
        let ev = tokio::time::timeout(REQUEST_TIMEOUT, client.next_topic())
            .await
            .map_err(|_| format!("no LayerChange({expected}) topic within timeout"))?;
        assert!(
            matches!(ev, TopicEvent::LayerChange(l) if l == expected),
            "expected LayerChange({expected}), got {ev:?}"
        );
    }

    let expected_layers = [
        [
            [key(HidKeyCode::Kp1), key(HidKeyCode::Kp2), key(HidKeyCode::Kp3)],
            [key(HidKeyCode::Kp4), key(HidKeyCode::Kp5), key(HidKeyCode::Kp6)],
            [key(HidKeyCode::Kp7), key(HidKeyCode::Kp8), key(HidKeyCode::Kp9)],
        ],
        [
            [key(HidKeyCode::A), key(HidKeyCode::B), key(HidKeyCode::C)],
            [key(HidKeyCode::D), key(HidKeyCode::E), key(HidKeyCode::F)],
            [key(HidKeyCode::G), key(HidKeyCode::H), key(HidKeyCode::I)],
        ],
    ];
    for (layer, rows) in expected_layers.iter().enumerate() {
        for (row, cols) in rows.iter().enumerate() {
            for (col, action) in cols.iter().enumerate() {
                assert_eq!(client.get_key(layer as u8, row as u8, col as u8).await?, *action);
            }
        }
    }

    expect_rejected(
        "get_key row out of range",
        client.get_key(0, 3, 0).await,
        RynkError::Invalid,
    );
    expect_rejected(
        "set_key layer out of range",
        client.set_key(2, 0, 0, KeyAction::No).await,
        RynkError::Invalid,
    );
    client.set_key(0, 0, 0, key(HidKeyCode::Kp2)).await?;
    assert_eq!(client.get_key(0, 0, 0).await?, key(HidKeyCode::Kp2));
    client.set_key(0, 0, 0, key(HidKeyCode::Kp1)).await?;
    assert_eq!(client.get_key(0, 0, 0).await?, key(HidKeyCode::Kp1));

    assert_eq!(
        client.get_encoder(0, 0).await?,
        encoder(HidKeyCode::KpPlus, HidKeyCode::KpMinus)
    );
    expect_rejected(
        "get_encoder id out of range",
        client.get_encoder(1, 0).await,
        RynkError::Invalid,
    );
    let swapped_encoder = encoder(HidKeyCode::KpMinus, HidKeyCode::KpPlus);
    client.set_encoder(0, 0, swapped_encoder).await?;
    assert_eq!(client.get_encoder(0, 0).await?, swapped_encoder);
    client
        .set_encoder(0, 0, encoder(HidKeyCode::KpPlus, HidKeyCode::KpMinus))
        .await?;

    let mut macro_bytes = heapless::Vec::new();
    macro_bytes.extend_from_slice(&[1, 2, 3, 4]).unwrap();
    client.set_macro(0, MacroData { data: macro_bytes }).await?;
    let got_macro = client.get_macro(0).await?;
    assert_eq!(got_macro.data.len(), caps.macro_chunk_size as usize);
    assert_eq!(&got_macro.data[..4], &[1, 2, 3, 4]);
    assert!(got_macro.data[4..].iter().all(|&b| b == 0));

    assert_eq!(client.get_combo(0).await?, Combo::empty());
    let changed_combo = Combo::new(
        [key(HidKeyCode::Kp1), key(HidKeyCode::Kp4)],
        key(HidKeyCode::Kp1),
        Some(1),
    );
    client.set_combo(0, changed_combo.clone()).await?;
    assert_eq!(client.get_combo(0).await?, changed_combo);
    client.set_combo(0, Combo::empty()).await?;
    assert_eq!(client.get_combo(0).await?, Combo::empty());
    expect_rejected(
        "get_combo out of range",
        client.get_combo(250).await,
        RynkError::Invalid,
    );

    assert_eq!(client.get_fork(0).await?, fixture_fork(key(HidKeyCode::C)));
    let changed_fork = fixture_fork(key(HidKeyCode::Kp2));
    client.set_fork(0, changed_fork).await?;
    assert_eq!(client.get_fork(0).await?, changed_fork);
    client.set_fork(0, fixture_fork(key(HidKeyCode::C))).await?;
    expect_rejected("get_fork out of range", client.get_fork(250).await, RynkError::Invalid);

    assert_eq!(client.get_morse(0).await?, empty_morse(MorseProfile::const_default()));
    let changed_morse = empty_morse(MorseProfile::const_default().with_hold_timeout_ms(Some(180)));
    client.set_morse(0, changed_morse.clone()).await?;
    assert_eq!(client.get_morse(0).await?, changed_morse);
    client.set_morse(0, empty_morse(MorseProfile::const_default())).await?;
    expect_rejected(
        "get_morse out of range",
        client.get_morse(250).await,
        RynkError::Invalid,
    );

    let mut behavior = client.get_behavior().await?;
    let original_behavior = behavior;
    behavior.combo_timeout_ms += 5;
    client.set_behavior(behavior).await?;
    assert_eq!(client.get_behavior().await?, behavior);
    client.set_behavior(original_behavior).await?;
    assert_eq!(client.get_behavior().await?, original_behavior);

    assert_eq!(client.get_current_layer().await?, 0);
    assert!(
        client
            .get_matrix_state()
            .await?
            .pressed_bitmap
            .iter()
            .all(|&byte| byte == 0)
    );
    assert_eq!(client.get_wpm().await?, 0);
    assert!(!client.get_sleep_state().await?);
    assert_eq!(client.get_led_indicator().await?, LedIndicator::default());
    assert_eq!(client.get_connection_type().await?, ConnectionType::Usb);
    assert_eq!(
        client.get_connection_status().await?,
        ConnectionStatus {
            usb: UsbState::Disabled,
            ble: BleStatus {
                profile: 0,
                state: BleState::Inactive,
                bonded: false,
            },
            preferred: ConnectionType::Usb,
        }
    );

    // Every capability-gated command the fixture lacks is rejected client-side,
    // before touching the wire.
    expect_unsupported("get_ble_status", client.get_ble_status().await);
    expect_unsupported("switch_ble_profile", client.switch_ble_profile(0).await);
    expect_unsupported("clear_ble_profile", client.clear_ble_profile(0).await);
    expect_unsupported("get_battery_status", client.get_battery_status().await);
    expect_unsupported("get_peripheral_status", client.get_peripheral_status(0).await);
    expect_unsupported(
        "storage_reset",
        client.storage_reset(StorageResetMode::LayoutOnly).await,
    );

    // Device identity — the fixture uses the default USB config.
    let info = client.get_device_info().await?;
    assert_eq!(info.manufacturer.as_str(), "RMK");
    assert_eq!(info.product_name.as_str(), "RMK Keyboard");

    // The fixture is `insecure`, so the lock gate stays open: no challenge, no
    // armed attempt, and `lock` is a no-op that leaves the device unlocked.
    let status = client.get_lock_status().await?;
    assert!(!status.locked);
    assert!(!status.unlocking);
    assert_eq!(status.remaining_keys, 0);
    assert!(status.key_positions.is_empty());
    assert!(!client.unlock_poll().await?.locked);
    client.lock().await?;
    assert!(!client.get_lock_status().await?.locked);

    // The second encoder layer is independently addressable; an out-of-range
    // layer is rejected. Round-trip a probe so the exact layer-1 config need not
    // be hardcoded here.
    let layer1_encoder = client.get_encoder(0, 1).await?;
    let probe_encoder = encoder(HidKeyCode::Kp1, HidKeyCode::Kp2);
    client.set_encoder(0, 1, probe_encoder).await?;
    assert_eq!(client.get_encoder(0, 1).await?, probe_encoder);
    client.set_encoder(0, 1, layer1_encoder).await?;
    assert_eq!(client.get_encoder(0, 1).await?, layer1_encoder);
    expect_rejected(
        "get_encoder layer out of range",
        client.get_encoder(0, 2).await,
        RynkError::Invalid,
    );

    // Single-page bulk read matches the head of the keymap; an out-of-geometry
    // start position is rejected.
    let keymap_bulk = client.get_keymap_bulk(0, 0, 0).await?;
    assert_eq!(keymap_bulk.actions.first(), Some(&key(HidKeyCode::Kp1)));
    expect_rejected(
        "get_keymap_bulk out of range",
        client.get_keymap_bulk(0, 3, 0).await,
        RynkError::Invalid,
    );

    // Whole-keymap paging: `read_all` reassembles the flat, layer-major keymap,
    // `write_all` pages a mutation back, then restores the original.
    let flat_expected: Vec<KeyAction> = expected_layers.iter().flatten().flatten().copied().collect();
    let all_keys = client.read_all_keymap().await?;
    assert_eq!(all_keys, flat_expected);
    let mut mutated_keys = all_keys.clone();
    mutated_keys[0] = key(HidKeyCode::Kp9);
    client.write_all_keymap(mutated_keys.clone()).await?;
    assert_eq!(client.read_all_keymap().await?, mutated_keys);
    client.write_all_keymap(all_keys).await?;
    assert_eq!(client.read_all_keymap().await?, flat_expected);

    // Combo table: every slot pages back (empty slots as the empty config), a
    // page-wide write round-trips, then restores.
    let all_combos = client.read_all_combos().await?;
    assert_eq!(all_combos.len(), caps.max_combos as usize);
    assert!(all_combos.iter().all(|c| *c == Combo::empty()));
    let mut mutated_combos = all_combos.clone();
    mutated_combos[0] = Combo::new(
        [key(HidKeyCode::Kp1), key(HidKeyCode::Kp4)],
        key(HidKeyCode::Kp1),
        Some(1),
    );
    client.write_all_combos(mutated_combos.clone()).await?;
    assert_eq!(client.read_all_combos().await?, mutated_combos);
    client.write_all_combos(all_combos).await?;

    // Morse table pages back every slot (the list is padded to `max_morse`);
    // slot 0 carries the configured profile, and a page at the slot count is empty.
    let all_morses = client.read_all_morses().await?;
    assert_eq!(all_morses.len(), caps.max_morse as usize);
    assert_eq!(all_morses[0], empty_morse(MorseProfile::const_default()));
    assert!(client.get_morse_bulk(caps.max_morse).await?.configs.is_empty());
    let mut mutated_morses = all_morses.clone();
    mutated_morses[0] = empty_morse(MorseProfile::const_default().with_hold_timeout_ms(Some(200)));
    client.write_all_morses(mutated_morses.clone()).await?;
    assert_eq!(client.read_all_morses().await?, mutated_morses);
    client.write_all_morses(all_morses).await?;

    // The fixture has no `[layout].map`, so the served blob is empty and decodes
    // to the empty layout rather than erroring.
    assert_eq!(client.get_layout().await?, LayoutInfo::empty());

    // Fire-and-forget commands: no-ops on this riscv fixture (the reset path is
    // cortex-m/esp only), so the session survives and the orphaned reply is
    // dropped by the client's SEQ routing while the next request proceeds.
    client.reboot().await?;
    assert_eq!(client.get_version().await?, ProtocolVersion::CURRENT);
    client.bootloader_jump().await?;
    assert_eq!(client.get_version().await?, ProtocolVersion::CURRENT);

    println!("QEMU Rynk behavior verification passed.");
    Ok(())
}
