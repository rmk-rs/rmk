#[cfg(feature = "subrating")]
use bt_hci::{cmd::le::LeSetHostFeature, controller::ControllerCmdSync};
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use rmk_types::connection::ConnectionStatus;
use trouble_host::prelude::*;

#[cfg(feature = "storage")]
use super::PeerAddress;
use super::{GattSplitMessage, SplitMessage};
use crate::ble::adv::{Adv, advertise};
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
    #[cfg(feature = "dfu_ble")]
    pub(crate) dfu_service: RynkDfuService,
}

/// BLE driver for split peripheral
pub(crate) struct BleSplitPeripheralDriver<'stack, 'server, 'c, P: PacketPool> {
    message_to_peripheral: Characteristic<GattSplitMessage>,
    message_to_central: Characteristic<GattSplitMessage>,
    conn: &'c GattConnection<'stack, 'server, P>,
}

impl<'stack, 'server, 'c, P: PacketPool> BleSplitPeripheralDriver<'stack, 'server, 'c, P> {
    pub(crate) fn new(server: &'server BleSplitPeripheralServer, conn: &'c GattConnection<'stack, 'server, P>) -> Self {
        Self {
            message_to_central: server.service.message_to_central.clone(),
            message_to_peripheral: server.service.message_to_peripheral.clone(),
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
pub async fn initialize_nrf_ble_split_peripheral_and_run<
    'b,
    's: 'b,
    #[cfg(feature = "subrating")] C: Controller + ControllerCmdSync<LeSetHostFeature>,
    #[cfg(not(feature = "subrating"))] C: Controller,
>(
    id: usize,
    stack: &'b Stack<'s, C, DefaultPacketPool>,
) {
    publish_event(CentralConnectedEvent { connected: false });

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    // First, read central address from storage
    let mut central_addr = crate::storage::read_peer_address(0)
        .await
        .filter(|a| a.is_valid)
        .map(|a| a.address);

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
                        if crate::storage::write_peer_address(PeerAddress {
                            peer_id: 0,
                            is_valid: true,
                            address: new_addr,
                        })
                        .await
                        {
                            central_addr = Some(new_addr);
                        }
                    }
                    peripheral.run().await;
                    info!("Disconnected from the central");
                }
                Err(BleHostError::BleHost(Error::Timeout)) => {
                    error!("Connect to central timeout");
                    #[cfg(feature = "dfu_ble")]
                    {
                        info!("Entering DFU advertising mode");
                        match split_peripheral_dfu_advertise(id, &mut peripheral, &server).await {
                            Ok(conn) => {
                                info!("DFU host connected");
                                run_dfu_session(&server, &conn).await;
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
    advertise(peripheral, &server.server, seeking, Duration::from_secs(300)).await
}

/// Advertise for a DFU host connection, using the peripheral's name.
///
/// Sends undirected advertisements with the name `"{product} per{N}"` for
/// up to 60 seconds, giving rynk-wtf time to discover and connect.
#[cfg(feature = "dfu_ble")]
async fn split_peripheral_dfu_advertise<'a, 'b, C: Controller>(
    id: usize,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b BleSplitPeripheralServer<'_>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut name_buf = [0u8; 16];
    let prefix = b"rmk per";
    name_buf[..prefix.len()].copy_from_slice(prefix);
    let id_str = {
        let mut tmp = [0u8; 4];
        let mut n = 0;
        let mut val = id;
        if val == 0 {
            tmp[0] = b'0';
            n = 1;
        } else {
            let mut start = 0;
            while val > 0 {
                tmp[start] = b'0' + (val % 10) as u8;
                val /= 10;
                start += 1;
            }
            for i in 0..start / 2 {
                tmp.swap(i, start - 1 - i);
            }
            n = start;
        }
        &tmp[..n]
    };
    let total_len = prefix.len() + id_str.len();
    name_buf[prefix.len()..total_len].copy_from_slice(id_str);
    let name = core::str::from_utf8(&name_buf[..total_len]).unwrap_or("rmk dfu");

    let adv = Adv::DfuPeripheral { name };
    advertise(peripheral, &server.server, adv, Duration::from_secs(60)).await
}

/// Run a minimal DFU session over the `RynkDfuService` GATT characteristics.
///
/// Decodes rynk-framed DFU commands from `dfu_output`, dispatches them to
/// [`ProxyRynkDfuHandler`](crate::host::rynk::handlers::dfu::ProxyRynkDfuHandler)
/// via [`dispatch_dfu_cmd`](crate::host::rynk::handlers::dfu::dispatch_dfu_cmd),
/// which sends them through `DFU_CHANNEL` for processing by `FlashDfuHandler`
/// in `run_all!()`.
#[cfg(feature = "dfu_ble")]
async fn run_dfu_session<'b, 's: 'b, C: Controller>(
    server: &'b BleSplitPeripheralServer<'_>,
    conn: &GattConnection<'b, 's, DefaultPacketPool>,
) {
    use rmk_types::protocol::rynk::command::Cmd;
    use rmk_types::protocol::rynk::{Deframer, RYNK_HEADER_SIZE, RynkHeader, encode_frame};

    let mut buf = [0u8; 512];
    let mut df = Deframer::new();

    let dfu_input = server.dfu_service.dfu_input.clone();
    let dfu_output = server.dfu_service.dfu_output.clone();

    loop {
        match conn.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                info!("DFU host disconnected: {:?}", reason);
                break;
            }
            GattConnectionEvent::Gatt { event } => match event {
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

                            // Only dispatch DFU commands; ignore non-DFU cmds.
                            let result = match header.cmd {
                                Cmd::DfuStart
                                | Cmd::DfuWrite
                                | Cmd::DfuCrcSync
                                | Cmd::DfuCrcRewind
                                | Cmd::DfuVerify
                                | Cmd::DfuFinish
                                | Cmd::DfuReset => {
                                    crate::host::rynk::handlers::dfu::dispatch_dfu_cmd(
                                        header.cmd,
                                        &buf[RYNK_HEADER_SIZE..frame_len],
                                    )
                                    .await
                                }
                                _ => {
                                    warn!("dfu_peri: non-DFU cmd {:?}", header.cmd);
                                    Err(rmk_types::protocol::rynk::RynkError::UnknownCmd)
                                }
                            };

                            let mut reply_buf = [0u8; 512];
                            match encode_frame(&mut reply_buf, header, &result) {
                                Ok(len) => {
                                    if dfu_input.notify(conn, &reply_buf[..len], true).await.is_err() {
                                        warn!("dfu_peri: notify failed");
                                    }
                                }
                                Err(e) => warn!("dfu_peri: encode error: {:?}", e),
                            }
                        }
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}
