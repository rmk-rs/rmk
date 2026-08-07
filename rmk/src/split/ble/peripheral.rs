use bt_hci::cmd::le::LeSetPhy;
use bt_hci::controller::ControllerCmdAsync;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer, with_timeout};
use rmk_types::connection::ConnectionStatus;
use trouble_host::prelude::*;

#[cfg(feature = "storage")]
use super::PeerAddress;
use super::{GattSplitMessage, SplitMessage};
use crate::event::{CentralConnectedEvent, KeyboardEvent, SubscribableEvent, publish_event};
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

/// Gatt server in split peripheral
#[gatt_server]
pub(crate) struct BleSplitPeripheralServer {
    pub(crate) service: SplitBleService,
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
                } => {
                    info!(
                        "Connection parameters updated: {:?}ms, {:?}, {:?}ms",
                        conn_interval.as_millis(),
                        peripheral_latency,
                        supervision_timeout.as_millis()
                    );
                }
                GattConnectionEvent::PhyUpdated { tx_phy, rx_phy } => {
                    info!("PHY updated: {:?}, {:?}", tx_phy, rx_phy);
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
        info!("Writing split message to central: {:?}", message);
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

/// Initialize and run the nRF peripheral keyboard service via BLE.
///
/// # Arguments
///
/// * `id` - The id of the peripheral
/// * `central_addr` - The address of the central
/// * `stack` - The stack to use
pub async fn initialize_nrf_ble_split_peripheral_and_run<'b, 's: 'b, C: Controller + ControllerCmdAsync<LeSetPhy>>(
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
        let server = BleSplitPeripheralServer::new_default("rmk").unwrap();
        loop {
            update_status(|c| *c = ConnectionStatus::new());
            publish_event(CentralConnectedEvent { connected: false });
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
                    // Timeout, wait new keys to continue
                    error!("Connect to central timeout");
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

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn split_peripheral_advertise<'a, 'b, C: Controller>(
    id: usize,
    central_addr: Option<[u8; 6]>,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b BleSplitPeripheralServer<'_>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    let advertisement = get_peri_advertiser::<C>(id, central_addr, &mut advertiser_data)?;

    let advertiser = peripheral
        .advertise(&AdvertisementParameters::default(), advertisement)
        .await?;

    match with_timeout(Duration::from_secs(10), advertiser.accept()).await {
        Ok(conn_res) => {
            let conn = conn_res?.with_attribute_server(server)?;
            info!("[adv] connection established");
            Ok(conn)
        }
        Err(_) => {
            warn!("[adv] Try update central_addr");
            // Advertise without central addr
            let advertisement = get_peri_advertiser::<C>(id, None, &mut advertiser_data)?;
            let advertiser = peripheral
                .advertise(&AdvertisementParameters::default(), advertisement)
                .await?;
            match with_timeout(Duration::from_secs(300), advertiser.accept()).await {
                Ok(re) => Ok(re?.with_attribute_server(server)?),
                Err(_e) => Err(BleHostError::BleHost(Error::Timeout)),
            }
        }
    }
}

fn get_peri_advertiser<'a, C: Controller>(
    id: usize,
    central_addr: Option<[u8; 6]>,
    advertiser_data: &'a mut [u8; 31],
) -> Result<Advertisement<'a>, BleHostError<C::Error>> {
    let advertisement = match central_addr {
        Some(addr) => Advertisement::ConnectableNonscannableDirected {
            peer: Address::random(addr),
        },
        None => {
            info!("No central address provided, so we advertise as undirected");
            // No central address provided, so we advertise as undirected
            AdStructure::encode_slice(
                &[
                    AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                    AdStructure::CompleteServiceUuids128(&[
                        // uuid: 4dd5fbaa-18e5-4b07-bf0a-353698659946
                        [
                            70u8, 153u8, 101u8, 152u8, 54u8, 53u8, 10u8, 191u8, 7u8, 75u8, 229u8, 24u8, 170u8, 251u8,
                            213u8, 77u8,
                        ],
                    ]),
                    AdStructure::ManufacturerSpecificData {
                        company_identifier: rmk_types::ble::RMK_ADV_COMPANY_ID,
                        payload: &[id as u8],
                    },
                ],
                &mut advertiser_data[..],
            )?;
            trace!("Advertising data: {:?}", advertiser_data);
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..],
                scan_data: &[],
            }
        }
    };
    Ok(advertisement)
}
