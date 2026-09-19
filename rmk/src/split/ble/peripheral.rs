#[cfg(feature = "subrating")]
use bt_hci::cmd::le::LeSetHostFeature;
#[cfg(any(feature = "subrating", feature = "dfu_ble"))]
use bt_hci::controller::ControllerCmdSync;
#[cfg(feature = "dfu_ble")]
use bt_hci::{
    cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy},
    controller::ControllerCmdAsync,
};
use embassy_futures::join::join;
#[cfg(any(feature = "custom_message", feature = "dfu_ble"))]
use embassy_futures::select::select;
use embassy_time::{Duration, Timer};
#[cfg(feature = "custom_message")]
use postcard::experimental::max_size::MaxSize;
use rmk_types::connection::ConnectionStatus;
use trouble_host::prelude::*;

#[cfg(feature = "storage")]
use super::PeerAddress;
use super::{GattSplitMessage, SplitMessage};
use crate::ble::adv::{Adv, advertise};
#[cfg(feature = "custom_message")]
use crate::custom_message::{CustomMessage, CustomMessageTarget, forward};
use crate::event::{CentralConnectedEvent, KeyboardEvent, SleepStateEvent, SubscribableEvent, publish_event};
use crate::split::driver::{SplitDriverError, SplitReader, SplitWriter};
use crate::split::peripheral::SplitPeripheral;
use crate::state::update_status;

/// Gatt service used in split peripheral to send split message to central
#[gatt_service(uuid = "4dd5fbaa-18e5-4b07-bf0a-353698659946")]
pub(crate) struct SplitBleService {
    #[characteristic(uuid = "0e6313e3-bd0b-45c2-8d2e-37a2e8128bc3", read, notify, indicate)]
    pub(crate) message_to_central: GattSplitMessage,

    #[characteristic(uuid = "4b3514fb-cae4-4d38-a097-3a2a3d1c3b9c", write_without_response, read, notify)]
    pub(crate) message_to_peripheral: GattSplitMessage,

    #[cfg(feature = "custom_message")]
    #[characteristic(uuid = "5f2a7c14-9b3e-4a51-8d76-2c1e4b8a6f03", read, notify)]
    pub(crate) custom_to_central: heapless::Vec<u8, { crate::custom_message::CustomMessage::POSTCARD_MAX_SIZE }>,
    #[cfg(feature = "custom_message")]
    #[characteristic(uuid = "5f2a7c15-9b3e-4a51-8d76-2c1e4b8a6f03", write_without_response, read)]
    pub(crate) custom_to_peripheral: heapless::Vec<u8, { crate::custom_message::CustomMessage::POSTCARD_MAX_SIZE }>,
}

/// Minimal rynk DFU GATT service for split peripherals.
///
/// Active only in DFU mode when the peripheral could not find its central.
#[cfg(feature = "dfu_ble")]
#[gatt_service(uuid = "4dd5fbaa-18e5-4b07-bf0a-353698659947")]
pub(crate) struct RynkDfuService {
    /// Rynk-framed DFU responses (firmware → host).
    #[characteristic(uuid = "0e6313e3-bd0b-45c2-8d2e-37a2e8128bc4", read, notify)]
    pub(crate) dfu_input: heapless::Vec<u8, 244>,

    /// Rynk-framed DFU commands (host → firmware).
    #[characteristic(uuid = "4b3514fb-cae4-4d38-a097-3a2a3d1c3b9d", write_without_response)]
    pub(crate) dfu_output: heapless::Vec<u8, 244>,
}

/// Gatt server in split peripheral
#[gatt_server]
pub(crate) struct BleSplitPeripheralServer {
    pub(crate) service: SplitBleService,
    /// Minimal rynk DFU GATT service. Active only with `dfu_ble` when the
    /// peripheral enters DFU mode after a central connection timeout.
    #[cfg(feature = "dfu_ble")]
    pub(crate) dfu_service: RynkDfuService,
}

/// BLE driver for split peripheral
pub(crate) struct BleSplitPeripheralDriver<'stack, 'server, 'c, P: PacketPool> {
    message_to_peripheral: Characteristic<GattSplitMessage>,
    message_to_central: Characteristic<GattSplitMessage>,
    #[cfg(feature = "custom_message")]
    custom_to_peripheral:
        Characteristic<heapless::Vec<u8, { crate::custom_message::CustomMessage::POSTCARD_MAX_SIZE }>>,
    conn: &'c GattConnection<'stack, 'server, P>,
}

impl<'stack, 'server, 'c, P: PacketPool> BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    pub(crate) fn new(server: &'server BleSplitPeripheralServer, conn: &'c GattConnection<'stack, 'server, P>) -> Self {
        Self {
            message_to_central: server.service.message_to_central.clone(),
            message_to_peripheral: server.service.message_to_peripheral.clone(),
            #[cfg(feature = "custom_message")]
            custom_to_peripheral: server.service.custom_to_peripheral.clone(),
            conn,
        }
    }
}

impl<'stack, 'server, 'c, P: PacketPool> SplitReader for BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    async fn read(&mut self) -> Result<SplitMessage, SplitDriverError> {
        let message = loop {
            match self.conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    error!("Disconnected from central: {:?}", reason);
                    update_status(|c| *c = ConnectionStatus::new());
                    return Err(SplitDriverError::Disconnected);
                }
                GattConnectionEvent::Gatt { event: gatt_event } => {
                    match &gatt_event {
                        GattEvent::Read(event) => {
                            info!("Gatt read event: {:?}", event.handle());
                        }
                        GattEvent::Write(event) => {
                            // Write to peripheral
                            if event.handle() == self.message_to_peripheral.handle {
                                let parsed = event.with_data(|_, data| {
                                    trace!("Got message from central: {:?}", data);
                                    postcard::from_bytes::<SplitMessage>(data)
                                });
                                match parsed {
                                    Ok(message) => {
                                        trace!("Message from central: {:?}", message);
                                        break message;
                                    }
                                    Err(e) => error!("Postcard deserialize split message error: {}", e),
                                }
                            } else if cfg!(feature = "custom_message") && {
                                #[cfg(feature = "custom_message")]
                                {
                                    event.handle() == self.custom_to_peripheral.handle
                                }
                                #[cfg(not(feature = "custom_message"))]
                                {
                                    false
                                }
                            } {
                                // Not a `SplitMessage`, so the read goes on.
                                // A peripheral has nowhere to forward to.
                                #[cfg(feature = "custom_message")]
                                event.with_data(|_, data| match postcard::from_bytes::<CustomMessage>(data) {
                                    // An end of the chain: it delivers what names it and
                                    // has nowhere to relay the rest to.
                                    Ok(message) => match message.target {
                                        CustomMessageTarget::Peripherals => publish_event(message),
                                        _ => (),
                                    },
                                    Err(_) => warn!("[split] undecodable custom message dropped"),
                                });
                            } else {
                                info!("Gatt write other event: {:?}", event.handle());
                            }
                        }
                        _ => debug!("Other gatt event"),
                    };
                    match gatt_event.accept() {
                        Ok(r) => r.send().await,
                        Err(e) => warn!("[gatt] error sending response: {:?}", e),
                    }
                }
                GattConnectionEvent::ConnectionParamsUpdated {
                    conn_interval,
                    peripheral_latency,
                    supervision_timeout,
                } => info!(
                    "[split] params updated: interval {:?}us, latency {:?}, timeout {:?}ms",
                    conn_interval.as_micros(),
                    peripheral_latency,
                    supervision_timeout.as_millis()
                ),
                GattConnectionEvent::SubratingParamsUpdated {
                    subrate_factor,
                    peripheral_latency,
                    continuation_number,
                    supervision_timeout,
                } => info!(
                    "[split] subrating updated: subrate {:?}, latency {:?}, continuation {:?}, timeout {:?}ms",
                    subrate_factor,
                    peripheral_latency,
                    continuation_number,
                    supervision_timeout.as_millis()
                ),
                GattConnectionEvent::PhyUpdated { tx_phy, rx_phy } => {
                    info!("[split] PHY updated: {:?}, {:?}", tx_phy, rx_phy)
                }
                _ => (),
            }
        };
        Ok(message)
    }
}

impl<'stack, 'server, 'c, P: PacketPool> SplitWriter for BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    async fn write(&mut self, message: &SplitMessage) -> Result<usize, SplitDriverError> {
        let gatt_msg = GattSplitMessage::try_from(message)?;
        debug!("Writing split message to central: {:?}", message);
        self.message_to_central
            .notify(self.conn, &gatt_msg, true)
            .await
            .map_err(|e| {
                error!("BLE notify error: {:?}", e);
                SplitDriverError::BleError(1)
            })?;
        Ok(gatt_msg.len)
    }
}

/// Let the controller accept the central's subrate requests on the split link.
///
/// Must run concurrently with `ble_task()` (whose runner serves the HCI command)
/// and before any advertising, since the flag only applies to links opened after
/// it is set.
#[cfg(feature = "subrating")]
async fn init_subrating_host_feature<C: Controller + ControllerCmdSync<LeSetHostFeature>>(
    stack: &Stack<'_, C, impl PacketPool>,
) {
    const CONN_SUBRATING_HOST_BIT: u8 = 38;
    let cmd = LeSetHostFeature::new(CONN_SUBRATING_HOST_BIT, 1);
    if let Err(e) = stack.command(cmd).await {
        error!("[split_peri] error setting subrating host feature flag: {:?}", e);
    }
}

/// Initialize and run the nRF peripheral keyboard service via BLE.
///
/// # Arguments
///
/// * `id` - The id of the peripheral
/// * `stack` - The BLE stack (owns the controller and address)
/// * `dfu_name` - (optional) BLE advertise name in DFU mode. `None` keeps the default `rmk per{id}`.
///   This argument is enabled only with `dfu_ble`
pub async fn initialize_nrf_ble_split_peripheral_and_run<
    'b,
    's: 'b,
    #[cfg(feature = "dfu_ble")] 'a,
    #[cfg(all(feature = "subrating", feature = "dfu_ble"))] C: Controller
        + ControllerCmdSync<LeSetHostFeature>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdAsync<LeSetPhy>,
    #[cfg(all(feature = "subrating", not(feature = "dfu_ble")))] C: Controller + ControllerCmdSync<LeSetHostFeature>,
    #[cfg(all(not(feature = "subrating"), feature = "dfu_ble"))] C: Controller + ControllerCmdSync<LeReadLocalSupportedFeatures> + ControllerCmdAsync<LeSetPhy>,
    #[cfg(all(not(feature = "subrating"), not(feature = "dfu_ble")))] C: Controller,
>(
    id: usize,
    stack: &'b Stack<'s, C, DefaultPacketPool>,
    #[cfg(feature = "dfu_ble")] dfu_name: Option<&'a str>,
) {
    publish_event(CentralConnectedEvent { connected: false });

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    // First, read central address from storage
    let mut central_addr = match crate::storage::read(crate::storage::StorageKey::PeerAddress(0)).await {
        Ok(Some(crate::storage::StorageValue::PeerAddress(a))) if a.is_valid => Some(a.address),
        _ => None,
    };

    let peri_task = async {
        // Set subrating host support before any advertising/connecting
        #[cfg(feature = "subrating")]
        init_subrating_host_feature(stack).await;

        let server = BleSplitPeripheralServer::new_default("rmk").unwrap();
        loop {
            update_status(|c| *c = ConnectionStatus::new());
            publish_event(CentralConnectedEvent { connected: false });
            publish_event(SleepStateEvent::new(false));
            match split_peripheral_advertise(id, central_addr, &mut peripheral, &server).await {
                Ok(conn) => {
                    info!("Connected to the central");
                    publish_event(CentralConnectedEvent { connected: true });
                    let mut peripheral = SplitPeripheral::new(BleSplitPeripheralDriver::new(&server, &conn));
                    let new_addr = conn.raw().peer_address().addr.into_inner();
                    if central_addr != Some(new_addr) {
                        info!("Saving central address to storage");
                        // RAM only follows flash here: a peer we cannot persist must be
                        // rediscovered after a reboot rather than silently trusted.
                        if crate::storage::store(crate::storage::StorageItem::PeerAddress(PeerAddress::new(
                            0, true, new_addr,
                        )))
                        .await
                        .is_ok()
                        {
                            central_addr = Some(new_addr);
                        }
                    }
                    #[cfg(not(feature = "custom_message"))]
                    peripheral.run().await;
                    #[cfg(feature = "custom_message")]
                    select(peripheral.run(), {
                        // A peripheral has one link, so everything queued goes out on it.
                        let custom_to_central = &server.service.custom_to_central;
                        forward(None, async |encoded| {
                            custom_to_central.notify_raw(&conn, encoded, false).await
                        })
                    })
                    .await;
                    info!("Disconnected from the central");
                }
                Err(BleHostError::BleHost(Error::Timeout)) => {
                    error!("Connect to central timeout");
                    #[cfg(feature = "dfu_ble")]
                    {
                        info!("Entering DFU advertising mode");
                        match split_peripheral_dfu_advertise(id, &mut peripheral, &server, dfu_name).await {
                            Ok(conn) => {
                                info!("DFU host connected");
                                run_dfu_session(stack, &server, &conn).await;
                                info!("DFU session ended");
                            }
                            Err(BleHostError::BleHost(Error::Timeout)) => {
                                info!("No DFU host, entering soft sleep");
                            }
                            Err(e) => {
                                #[cfg(feature = "defmt")]
                                let e = defmt::Debug2Format(&e);
                                error!("DFU advertise error: {:?}", e);
                            }
                        }
                    }
                    publish_event(SleepStateEvent::new(true));
                    let mut sub = KeyboardEvent::subscriber();
                    sub.clear();
                    let _ = sub.next_message_pure().await;
                    continue;
                }
                Err(e) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Advertise error: {:?}", e);
                    Timer::after_millis(500).await;
                    continue;
                }
            };
        }
    };

    join(crate::ble::ble_task(runner, &crate::ble::NoopHandler), peri_task).await;
}

/// Reconnect to the saved central, falling back to seeking any central when it
/// does not answer.
async fn split_peripheral_advertise<'a, 'b, C: Controller>(
    id: usize,
    central_addr: Option<[u8; 6]>,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b BleSplitPeripheralServer<'_>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    if let Some(addr) = central_addr {
        let directed = Adv::Directed(Address::random(addr));
        match advertise(peripheral, &server.server, directed, Duration::from_secs(10)).await {
            Err(BleHostError::BleHost(Error::Timeout)) => warn!("[adv] Try update central_addr"),
            result => return result,
        }
    }
    let seeking = Adv::SplitPeripheral { id: id as u8 };
    #[cfg(feature = "dfu_ble")]
    let seeking_duration = Duration::from_secs(10);
    #[cfg(not(feature = "dfu_ble"))]
    let seeking_duration = Duration::from_secs(300);
    advertise(peripheral, &server.server, seeking, seeking_duration).await
}

/// Advertise for a DFU host connection, using the peripheral's name.
///
/// Sends undirected advertisements for up to 60 seconds, giving rynk-wtf time
/// to discover and connect. `dfu_name` overrides the default `rmk per{id}`;
/// longer names are truncated to the 31-byte advertisement budget.
#[cfg(feature = "dfu_ble")]
async fn split_peripheral_dfu_advertise<'a, 'b, C: Controller>(
    id: usize,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b BleSplitPeripheralServer<'_>,
    dfu_name: Option<&str>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    let prefix = b"rmk per";
    let (name_buf, total_len) = {
        let mut buf = [0u8; 16];
        buf[..prefix.len()].copy_from_slice(prefix);
        let mut pos = prefix.len();
        let mut val = id;
        // 20 digits cover usize::MAX on 64-bit; ids are small in practice.
        let mut digits = [0u8; 20];
        let mut n = 0;
        if val == 0 {
            digits[0] = b'0';
            n = 1;
        } else {
            while val > 0 && n < digits.len() {
                digits[n] = b'0' + (val % 10) as u8;
                val /= 10;
                n += 1;
            }
            digits[..n].reverse();
        }
        // buf holds prefix + up to 9 digits; truncate longer ids.
        let n = n.min(buf.len() - pos);
        buf[pos..pos + n].copy_from_slice(&digits[..n]);
        pos += n;
        (buf, pos)
    };
    let full = dfu_name.unwrap_or(core::str::from_utf8(&name_buf[..total_len]).unwrap_or("rmk dfu"));
    // Legacy advertisements fit 31 bytes; truncate instead of failing the
    // whole DFU advertise with `InsufficientSpace`.
    use rmk_types::ble::BLE_ADV_NAME_MAX_LEN;
    let mut len = full.len().min(BLE_ADV_NAME_MAX_LEN);
    while !full.is_char_boundary(len) {
        len -= 1;
    }
    if len < full.len() {
        warn!(
            "dfu_peri: DFU name exceeds {} adv bytes, advertising as {:?}",
            BLE_ADV_NAME_MAX_LEN,
            &full[..len]
        );
    }
    let name = &full[..len];

    let adv = Adv::DfuPeripheral { name };
    advertise(peripheral, &server.server, adv, Duration::from_secs(60)).await
}

/// Run a minimal DFU session over the `RynkDfuService` GATT characteristics.
///
/// Decodes rynk-framed DFU commands from `dfu_output`, dispatches them to
/// [`ProxyRynkDfuHandler`](crate::host::rynk::handlers::dfu::ProxyRynkDfuHandler)
/// via [`dispatch_dfu_cmd`](crate::host::rynk::handlers::dfu::dispatch_dfu_cmd),
/// which sends them through `DfuCmdEvent` for processing by `FlashDfuHandler`
/// in `run_all!()`. Responses are collected while the GATT event is borrowed
/// and notified after the borrow ends, before the event is accepted.
#[cfg(feature = "dfu_ble")]
async fn run_dfu_session<'b, 's: 'b, C>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    server: &'b BleSplitPeripheralServer<'_>,
    conn: &GattConnection<'b, 's, DefaultPacketPool>,
) where
    C: Controller + ControllerCmdSync<LeReadLocalSupportedFeatures> + ControllerCmdAsync<LeSetPhy>,
{
    use rmk_types::constants::RYNK_BUFFER_SIZE;
    use rmk_types::protocol::rynk::command::Cmd;
    use rmk_types::protocol::rynk::{
        Deframer, DeviceCapabilities, LockStatus, ProtocolVersion, RYNK_HEADER_SIZE, RYNK_MAX_PAYLOAD_SIZE, RynkError,
        RynkHeader, encode_frame,
    };

    // Sized like the central's session buffer: DfuWrite frames are ~2 KB, a
    // smaller buffer would discard them as overflow.
    let mut buf = [0u8; RYNK_BUFFER_SIZE];
    let mut df = Deframer::new();

    let dfu_input = server.dfu_service.dfu_input.clone();
    let dfu_output = server.dfu_service.dfu_output.clone();

    // Run `set_conn_params` alongside the GATT event loop via `select`.
    // `set_conn_params` subscribes to `DfuStatusEvent` and races each
    // 5-second timer against `Started` events.  When the host sends
    // DfuStart, `dispatch_dfu_cmd` publishes `DfuStatus::Started` from
    // inside the event loop below, and `set_conn_params` picks it up to
    // switch to 7.5 ms / 0 latency / 20 s timeout.
    let dfu_events = async {
        loop {
            match conn.next().await {
                GattConnectionEvent::Disconnected { reason } => {
                    info!("DFU host disconnected: {:?}", reason);
                    break;
                }
                GattConnectionEvent::Gatt { event: gatt_event } => {
                    // Collect owned responses inside the event borrow and notify
                    // after it ends: no `.await` on the connection runs while the
                    // incoming write is still borrowed. The central does the same
                    // split — `gatt_events_task` accepts first, the session task
                    // notifies separately.
                    let mut pending: heapless::Vec<heapless::Vec<u8, 244>, 4> = heapless::Vec::new();
                    match &gatt_event {
                        GattEvent::Write(write_event) => {
                            if write_event.handle() == dfu_output.handle {
                                write_event.with_data(|_, data| {
                                    trace!("DFU write: {} bytes", data.len());
                                    let tail = df.tail(&mut buf);
                                    let n = data.len().min(tail.len());
                                    tail[..n].copy_from_slice(&data[..n]);
                                    df.commit(n);
                                });

                                while let Some(frame_len) = df.next(&mut buf) {
                                    let header = RynkHeader::parse(buf[..RYNK_HEADER_SIZE].try_into().unwrap());
                                    debug!("dfu_peri: frame cmd={:?}", header.cmd);

                                    let mut reply_buf = [0u8; 512];
                                    let encoded = match header.cmd {
                                        Cmd::DfuStart
                                        | Cmd::DfuWrite
                                        | Cmd::DfuCrcSync
                                        | Cmd::DfuCrcRewind
                                        | Cmd::DfuVerify
                                        | Cmd::DfuFinish
                                        | Cmd::DfuReset => {
                                            let result = crate::host::rynk::handlers::dfu::dispatch_dfu_cmd(
                                                header.cmd,
                                                &buf[RYNK_HEADER_SIZE..frame_len],
                                            )
                                            .await;
                                            debug!("dfu_peri: dispatch {:?} -> {:?}", header.cmd, result);
                                            encode_frame(&mut reply_buf, header, &result)
                                        }
                                        Cmd::GetVersion => {
                                            let result: Result<ProtocolVersion, RynkError> =
                                                Ok(ProtocolVersion::CURRENT);
                                            encode_frame(&mut reply_buf, header, &result)
                                        }
                                        Cmd::GetCapabilities => {
                                            let result: Result<DeviceCapabilities, RynkError> =
                                                Ok(DeviceCapabilities {
                                                    dfu_enabled: true,
                                                    is_split: true,
                                                    ble_enabled: true,
                                                    max_payload_size: RYNK_MAX_PAYLOAD_SIZE as u16,
                                                    ..Default::default()
                                                });
                                            encode_frame(&mut reply_buf, header, &result)
                                        }
                                        Cmd::GetLockStatus => {
                                            let result: Result<LockStatus, RynkError> = Ok(LockStatus {
                                                locked: false,
                                                unlocking: false,
                                                remaining_keys: 0,
                                                key_positions: Default::default(),
                                            });
                                            encode_frame(&mut reply_buf, header, &result)
                                        }
                                        _ => {
                                            warn!("dfu_peri: non-DFU cmd {:?}", header.cmd);
                                            let result: Result<(), RynkError> = Err(RynkError::UnknownCmd);
                                            encode_frame(&mut reply_buf, header, &result)
                                        }
                                    };

                                    match encoded {
                                        Ok(len) => {
                                            debug!("dfu_peri: sending response for {:?}, {} bytes", header.cmd, len);
                                            let response: heapless::Vec<u8, 244> = match heapless::Vec::from_slice(
                                                &reply_buf[..len],
                                            ) {
                                                Ok(response) => response,
                                                Err(_) => {
                                                    warn!(
                                                        "dfu_peri: response for {:?} ({} bytes) exceeds notify capacity",
                                                        header.cmd, len
                                                    );
                                                    continue;
                                                }
                                            };
                                            if pending.push(response).is_err() {
                                                warn!("dfu_peri: dropping response for {:?}, pending full", header.cmd);
                                            }
                                        }
                                        Err(e) => warn!("dfu_peri: encode error: {:?}", e),
                                    }
                                }
                            }
                        }
                        _ => {}
                    };
                    // The event borrow ended above, so no GATT-event borrow is
                    // held while notifying — only then is the write accepted.
                    for response in pending {
                        if dfu_input.notify(conn, &response, true).await.is_err() {
                            warn!("dfu_peri: notify failed");
                        } else {
                            debug!("dfu_peri: notify ok");
                        }
                    }
                    match gatt_event.accept() {
                        Ok(r) => {
                            debug!("dfu_peri: accept ok, sending ATT response");
                            r.send().await;
                            debug!("dfu_peri: ATT response sent");
                        }
                        Err(e) => warn!("[gatt] error sending response: {:?}", e),
                    }
                }
                _ => {}
            }
        }
    };

    select(dfu_events, crate::ble::set_conn_params(stack, conn)).await;
}
