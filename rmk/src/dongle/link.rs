//! The dongle's one keyboard link: reconnect to the bonded keyboard, or adopt
//! one that is seeking a dongle, then relay in both directions until disconnect.

use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy, LeSetScanParams};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use embassy_futures::select::{Either, select, select3};
use embassy_time::{Duration, Timer, with_timeout};
use rmk_types::protocol::rynk::{RYNK_BLE_CHUNK_SIZE, RYNK_INPUT_CHAR_UUID, RYNK_OUTPUT_CHAR_UUID, RYNK_SERVICE_UUID};
use trouble_host::prelude::*;
use usbd_hid::descriptor::{MediaKeyboardReport, MouseReport, SystemControlReport};

use super::router;
use crate::ble::scan::{DONGLE_SCAN_WINDOW, scan_config};
use crate::ble::{update_ble_phy, update_conn_params, wait_for_stack_started};
use crate::channel::send_hid_report;
use crate::event::{EventSubscriber, LedIndicatorEvent, SubscribableEvent};
use crate::hid::{KeyboardReport, Report};
use crate::storage::FlashOperationMessage;

/// Discovered services on this build's keyboards; HID + Rynk.
const MAX_SERVICES: usize = 8;
/// Characteristic budget for the HID service discovery.
const MAX_CHARACTERISTICS: usize = 16;

type Client<'a, C> = GattClient<'a, C, DefaultPacketPool, MAX_SERVICES>;

/// The dongle's whole state machine: serve the bond, and whenever it does not
/// answer, listen for a keyboard seeking a dongle. Seeking follows a deliberate
/// 5s hold on the keyboard, so a replacement is never adopted by accident.
pub(super) async fn link_task<C>(stack: &Stack<'_, C, DefaultPacketPool>) -> !
where
    C: Controller
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeSetScanParams>,
{
    wait_for_stack_started().await;
    loop {
        let mut relayed = false;
        if let Some(addr) = super::read_peer(|p| p.bond.as_ref().map(|b| b.identity.addr))
            && let Some(conn) = connect(stack, addr).await
        {
            relayed = true;
            run_link(stack, conn, false).await;
        }
        if !relayed
            && let Some((kind, addr)) = super::run_pairing_window(stack).await
            && let Some(conn) = connect(stack, Address { kind, addr }).await
        {
            run_link(stack, conn, true).await;
        }
        Timer::after_millis(500).await;
    }
}

/// Secure an established link, then relay until it drops. `pairing` bonds a
/// keyboard adopted from a window, replacing whatever this dongle had.
async fn run_link<'b, 's: 'b, C>(
    stack: &'b Stack<'s, C, DefaultPacketPool>,
    conn: Connection<'b, DefaultPacketPool>,
    pairing: bool,
) where
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    match secure(&conn, pairing).await {
        Ok(_) => {}
        Err(SecureError::KeyRejected) => {
            // The keyboard re-bonded elsewhere; our record can never work again
            // (design §2.5). Drop it and reopen for a fresh pairing.
            warn!("[dongle] encryption rejected, clearing the bond");
            forget_bond().await;
            conn.disconnect();
            return;
        }
        Err(_) => {
            info!("[dongle] securing failed");
            conn.disconnect();
            return;
        }
    }

    let client = match Client::new(stack, &conn).await {
        Ok(client) => client,
        Err(_) => {
            conn.disconnect();
            return;
        }
    };

    // The client task pumps notifications; everything else runs beside it and
    // wins the select when the session ends.
    let session = async {
        if let Err(e) = connected_session(stack, &conn, &client, pairing).await {
            info!("[dongle] session setup failed: {:?}", e);
        }
    };
    select(client.task(), session).await;

    conn.disconnect();

    // Release whatever the keyboard was holding when the link dropped.
    super::update_peer(|p| p.connected = false);
    router::LINK_DOWN.signal(());
    router::ROUTER_TX.clear();
    send_hid_report(Report::KeyboardReport(KeyboardReport::default())).await;
    send_hid_report(Report::MouseReport(MouseReport {
        buttons: 0,
        x: 0,
        y: 0,
        wheel: 0,
        pan: 0,
    }))
    .await;
    send_hid_report(Report::MediaKeyboardReport(MediaKeyboardReport { usage_id: 0 })).await;
    send_hid_report(Report::SystemControlReport(SystemControlReport { usage_id: 0 })).await;
    info!("[dongle] disconnected");
}

async fn connect<'b, 's: 'b, C>(
    stack: &'b Stack<'s, C, DefaultPacketPool>,
    address: Address,
) -> Option<Connection<'b, DefaultPacketPool>>
where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    let mut central = stack.central();
    // Secure-phase parameters. Peripheral latency MUST be 0 here: with
    // latency 30, an esp-radio keyboard's controller loses link sync during
    // the central's ~400ms P-256 pauses in SMP and both sides hit supervision
    // timeout (bisected on hardware; the 7.5 ms interval itself is fine).
    // The generous supervision timeout rides out slow keyboard-side ECDH.
    // `connected_session` applies the typing parameters once serving starts.
    let connect_params = RequestedConnParams {
        min_connection_interval: Duration::from_micros(7500),
        max_connection_interval: Duration::from_micros(7500),
        max_latency: 0,
        supervision_timeout: Duration::from_secs(30),
        ..Default::default()
    };
    let config = ConnectConfig {
        connect_params,
        scan_config: ScanConfig {
            filter_accept_list: &[address],
            ..scan_config(DONGLE_SCAN_WINDOW)
        },
    };
    // A keyboard that is away burns this whole timeout before a window opens.
    match with_timeout(Duration::from_secs(15), central.connect(&config)).await {
        Ok(Ok(conn)) => Some(conn),
        Ok(Err(e)) => {
            #[cfg(feature = "defmt")]
            let e = defmt::Debug2Format(&e);
            debug!("[dongle] connect error: {:?}", e);
            None
        }
        Err(_) => None,
    }
}

/// 7.5 ms interval — the same latency budget as a split link. The generous
/// supervision timeout trades reconnect latency after a dongle power-cycle
/// (rare) for radio-interference tolerance during normal use: the keyboard
/// only starts its directed reconnect advertising once this timer expires.
fn link_conn_params() -> RequestedConnParams {
    RequestedConnParams {
        min_connection_interval: Duration::from_micros(7500),
        max_connection_interval: Duration::from_micros(7500),
        max_latency: 30,
        supervision_timeout: Duration::from_secs(10),
        ..Default::default()
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum SecureError {
    /// The peer rejected our long-term key: the bond is stale for good.
    KeyRejected,
    Failed,
    Disconnected,
}

/// Pair (fresh keyboard) or encrypt (bonded keyboard), then wait for the link
/// to report it is secure.
async fn secure(conn: &Connection<'_, DefaultPacketPool>, pairing: bool) -> Result<(), SecureError> {
    if let Err(e) = conn.request_security() {
        warn!("[dongle] request_security error: {:?}", e);
        return Err(SecureError::Failed);
    }
    loop {
        match with_timeout(Duration::from_secs(30), conn.next()).await {
            Ok(ConnectionEvent::Encrypted { .. }) | Ok(ConnectionEvent::PairingComplete { .. }) => return Ok(()),
            Ok(ConnectionEvent::PairingFailed(e)) => {
                warn!("[dongle] pairing failed: {:?}", e);
                // A bonded peer that refuses our key has re-bonded elsewhere.
                return Err(if pairing {
                    SecureError::Failed
                } else {
                    SecureError::KeyRejected
                });
            }
            Ok(ConnectionEvent::Disconnected { .. }) => return Err(SecureError::Disconnected),
            Ok(_) => {}
            Err(_) => return Err(SecureError::Failed),
        }
    }
}

#[derive(Debug)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
enum SessionError {
    Discovery,
    Handshake,
}

/// The characteristics the relay needs on the keyboard.
struct KeyboardChars {
    input_keyboard: Characteristic<[u8]>,
    output_keyboard: Characteristic<[u8]>,
    mouse: Characteristic<[u8]>,
    media: Characteristic<[u8]>,
    system: Characteristic<[u8]>,
    rynk_input: Characteristic<[u8]>,
    rynk_output: Characteristic<[u8]>,
}

/// Discover → subscribe → commit the bond → relay.
async fn connected_session<C>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    conn: &Connection<'_, DefaultPacketPool>,
    client: &Client<'_, C>,
    pairing: bool,
) -> Result<(), SessionError>
where
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    // Same latency setup as a split link: 2M PHY + 7.5 ms interval.
    update_ble_phy(stack, conn, PhyKind::Le2M).await;
    update_conn_params(stack, conn, &link_conn_params()).await;

    let chars = discover(client).await.ok_or(SessionError::Discovery)?;

    // Subscribe by writing each CCCD once, then take a single catch-all
    // listener — one queue, routed by handle.
    for ch in [
        &chars.input_keyboard,
        &chars.mouse,
        &chars.media,
        &chars.system,
        &chars.rynk_input,
    ] {
        if let Some(cccd) = ch.cccd_handle
            && client.write_handle(cccd, &[0x01, 0x00]).await.is_err()
        {
            return Err(SessionError::Discovery);
        }
    }
    let mut listener = client.listen_all().map_err(|_| SessionError::Discovery)?;

    if pairing {
        // The bond was stored by the Encrypted/PairingComplete event; fetch the
        // stack's copy, replacing whatever this dongle was bonded to before.
        let identity = conn.peer_identity();
        let bond = stack
            .with_bond_information(|bonds| bonds.iter().find(|b| b.identity == identity).cloned())
            .ok_or(SessionError::Handshake)?;
        let replaced = super::update_peer(|p| p.bond.replace(bond.clone()));
        if let Some(old) = replaced.filter(|b| b.identity != identity) {
            info!("[dongle] replacing the bond with {:?}", identity.addr);
            let _ = super::REMOVED_BOND.try_send(old.identity);
        }
        crate::channel::FLASH_CHANNEL
            .send(FlashOperationMessage::ProfileInfo(crate::ble::profile::ProfileInfo {
                slot_num: 0,
                removed: false,
                info: bond,
                cccd_table: heapless::Vec::new(),
            }))
            .await;
    }

    // LINK_DOWN latches; clear it and the queue before this link's first forward.
    router::LINK_DOWN.reset();
    router::ROUTER_TX.clear();
    super::update_peer(|p| p.connected = true);
    info!("[dongle] relaying");
    serve(stack, conn, client, &mut listener, &chars).await;
    Ok(())
}

/// Discover the HID and Rynk services. The four HID input reports share UUID
/// 0x2A4D; both ends are RMK, so their declaration order is fixed:
/// input_keyboard, output_keyboard, mouse, media, system.
async fn discover<C: Controller>(client: &Client<'_, C>) -> Option<KeyboardChars> {
    let hid = client
        .services_by_uuid(&Uuid::new_short(0x1812))
        .await
        .ok()?
        .first()
        .cloned()?;
    let all: heapless::Vec<Characteristic<[u8]>, MAX_CHARACTERISTICS> =
        client.characteristics::<MAX_CHARACTERISTICS>(&hid).await.ok()?;
    let report_uuid = Uuid::new_short(0x2A4D);
    let mut reports = all.into_iter().filter(|c| c.uuid == report_uuid);
    let input_keyboard = reports.next()?;
    let output_keyboard = reports.next()?;
    let mouse = reports.next()?;
    let media = reports.next()?;
    let system = reports.next()?;

    let rynk = client
        .services_by_uuid(&Uuid::new_long(RYNK_SERVICE_UUID.to_le_bytes()))
        .await
        .ok()?
        .first()
        .cloned()?;
    let rynk_input = client
        .characteristic_by_uuid::<[u8]>(&rynk, &Uuid::new_long(RYNK_INPUT_CHAR_UUID.to_le_bytes()))
        .await
        .ok()?;
    let rynk_output = client
        .characteristic_by_uuid::<[u8]>(&rynk, &Uuid::new_long(RYNK_OUTPUT_CHAR_UUID.to_le_bytes()))
        .await
        .ok()?;

    Some(KeyboardChars {
        input_keyboard,
        output_keyboard,
        mouse,
        media,
        system,
        rynk_input,
        rynk_output,
    })
}

/// Largest single write/notify chunk on the Rynk characteristics.
fn rynk_chunk_size(conn: &Connection<'_, DefaultPacketPool>) -> usize {
    RYNK_BLE_CHUNK_SIZE
        .min((conn.att_mtu() as usize).saturating_sub(3))
        .max(1)
}

/// Relay until the link dies: notifications out to USB/router, LED state and
/// router frames back to the keyboard, plus a watchdog for disconnect.
async fn serve<C: Controller>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    conn: &Connection<'_, DefaultPacketPool>,
    client: &Client<'_, C>,
    listener: &mut NotificationListener<'_, 512>,
    chars: &KeyboardChars,
) {
    let chunk = rynk_chunk_size(conn);

    let rx = async {
        loop {
            let notification = listener.next().await;
            let handle = notification.handle();
            let data = notification.as_ref();
            // The BLE boot report and the USB one carry the same bytes; only
            // the Rynk stream is opaque, and it goes straight to the host.
            if handle == chars.input_keyboard.handle && data.len() >= 8 {
                send_hid_report(Report::KeyboardReport(KeyboardReport {
                    modifier: data[0],
                    reserved: 0,
                    leds: 0,
                    keycodes: data[2..8].try_into().unwrap(),
                }))
                .await;
            } else if handle == chars.mouse.handle && data.len() >= 5 {
                send_hid_report(Report::MouseReport(MouseReport {
                    buttons: data[0],
                    x: data[1] as i8,
                    y: data[2] as i8,
                    wheel: data[3] as i8,
                    pan: data[4] as i8,
                }))
                .await;
            } else if handle == chars.media.handle && data.len() >= 2 {
                send_hid_report(Report::MediaKeyboardReport(MediaKeyboardReport {
                    usage_id: u16::from_le_bytes([data[0], data[1]]),
                }))
                .await;
            } else if handle == chars.system.handle && !data.is_empty() {
                send_hid_report(Report::SystemControlReport(SystemControlReport { usage_id: data[0] })).await;
            } else if handle == chars.rynk_input.handle {
                router::forward_to_host(data);
            }
        }
    };

    let tx = async {
        let mut led_events = LedIndicatorEvent::subscriber();
        loop {
            match select(led_events.next_event(), router::ROUTER_TX.receive()).await {
                Either::First(event) => {
                    let _ = client
                        .write_characteristic_without_response(&chars.output_keyboard, &[event.0.into_bits()])
                        .await;
                }
                Either::Second(frame) => {
                    for part in frame.chunks(chunk) {
                        if client
                            .write_characteristic_without_response(&chars.rynk_output, part)
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
        }
    };

    // Consumes connection events (a peripheral's parameter request must be
    // answered or trouble drops it) and ends the serve on link loss or when
    // the bond is dropped out from under us.
    let watch = async {
        let events = async {
            loop {
                match conn.next().await {
                    ConnectionEvent::Disconnected { .. } => return,
                    ConnectionEvent::RequestConnectionParams(req) => {
                        // The keyboard asks for its host-link parameters; they
                        // match our typing parameters, so accept.
                        if let Err(e) = req.accept(None, stack).await {
                            debug!("[dongle] conn param accept error: {:?}", e);
                        }
                    }
                    _ => {}
                }
            }
        };
        let forget = async {
            loop {
                Timer::after_millis(500).await;
                if super::read_peer(|p| p.bond.is_none()) {
                    info!("[dongle] bond cleared, dropping the link");
                    conn.disconnect();
                    return;
                }
            }
        };
        select(events, forget).await;
    };

    select3(rx, tx, watch).await;
}

/// Forget the keyboard completely: RAM entry, persisted bond, and the stack's copy.
async fn forget_bond() {
    let identity = super::update_peer(|p| {
        p.connected = false;
        p.bond.take().map(|b| b.identity)
    });
    if let Some(identity) = identity {
        let _ = super::REMOVED_BOND.try_send(identity);
    }
    crate::channel::FLASH_CHANNEL
        .send(FlashOperationMessage::ClearSlot(0))
        .await;
}
