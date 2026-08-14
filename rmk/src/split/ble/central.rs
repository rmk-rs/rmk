use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy, LeSetScanParams};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer, with_timeout};
use trouble_host::prelude::*;

use super::GattSplitMessage;
use crate::ble::adv::Adv;
use crate::ble::scan::{SPLIT_CENTRAL_SCAN_WINDOW, scan_config, start_scan};
use crate::ble::sleep::report_activity;
use crate::ble::{update_ble_phy, update_conn_params, wait_for_stack_started};
use crate::channel::FLASH_CHANNEL;
use crate::event::{EventSubscriber, SleepStateEvent, SubscribableEvent};
use crate::split::ble::PeerAddress;
use crate::split::driver::{PeripheralManager, SplitDriverError, SplitReader, SplitWriter, set_peripheral_connected};
use crate::split::{PeripheralMatrixConfig, SPLIT_MESSAGE_MAX_SIZE, SplitMessage};
use crate::storage::FlashOperationMessage;

static PERIPHERAL_FOUND: Signal<crate::RawMutex, (u8, BdAddr)> = Signal::new();

/// One peripheral link's lifecycle, owned by [`scan_and_connect_peripherals`].
enum SlotState {
    /// Unknown address: discover the peripheral by scanning.
    NoAddr,
    /// Known address without a live session: connect the peripheral.
    Disconnected([u8; 6]),
    /// The link is up and handed to the slot's session task.
    Connected([u8; 6]),
}

// The split service and its two characteristics, declared by `#[gatt_service]`
// in `split::ble::peripheral` and discovered by UUID here.
const SPLIT_SERVICE_UUID: u128 = 0x4dd5fbaa_18e5_4b07_bf0a_353698659946;
const MESSAGE_TO_CENTRAL_UUID: u128 = 0x0e6313e3_bd0b_45c2_8d2e_37a2e8128bc3;
const MESSAGE_TO_PERIPHERAL_UUID: u128 = 0x4b3514fb_cae4_4d38_a097_3a2a3d1c3b9c;

/// Scan for peripheral addresses, connect them, and hand each connection to
/// that slot's session; sessions report back on `ended`.
pub(crate) async fn scan_and_connect_peripherals<'a, C: Controller + ControllerCmdSync<LeSetScanParams>>(
    stack: &'a Stack<'_, C, DefaultPacketPool>,
    conns: &[Channel<NoopRawMutex, Connection<'a, DefaultPacketPool>, 1>; crate::SPLIT_PERIPHERALS_NUM],
    ended: &Channel<NoopRawMutex, usize, { crate::SPLIT_PERIPHERALS_NUM }>,
) {
    // Load each peripheral's stored address first.
    let mut peripheral_slots: [SlotState; crate::SPLIT_PERIPHERALS_NUM] = core::array::from_fn(|_| SlotState::NoAddr);
    for (id, slot) in peripheral_slots.iter_mut().enumerate() {
        if let Some(peer) = crate::storage::read_peer_address(id as u8)
            .await
            .filter(|peer| peer.is_valid)
        {
            *slot = SlotState::Disconnected(peer.address);
        }
    }

    let mut central = stack.central();
    wait_for_stack_started().await;
    loop {
        // The radio is the biggest draw while the keyboard sleeps, and a peripheral
        // that isn't connected can't wake it anyway. Established links stay up.
        while crate::state::current_sleep_state() {
            Timer::after_secs(1).await;
        }

        // Mark ended sessions `Disconnected`.
        while let Ok(id) = ended.try_receive() {
            if let SlotState::Connected(addr) = peripheral_slots[id] {
                peripheral_slots[id] = SlotState::Disconnected(addr);
            }
        }

        // Put every pending address in one accept list: the controller connects
        // to whichever peripheral answers first, so an absent one doesn't block
        // a present one for the whole timeout window.
        let mut pending: heapless::Vec<(usize, [u8; 6]), { crate::SPLIT_PERIPHERALS_NUM }> = heapless::Vec::new();
        for (id, slot) in peripheral_slots.iter().enumerate() {
            if let SlotState::Disconnected(addr) = slot {
                let _ = pending.push((id, *addr));
            }
        }
        if !pending.is_empty() {
            // If there're `Disconnected` peripherals, connect to them first.
            let targets: heapless::Vec<Address, { crate::SPLIT_PERIPHERALS_NUM }> =
                pending.iter().map(|(_, addr)| Address::random(*addr)).collect();
            let config = ConnectConfig {
                connect_params: default_central_conn_param(),
                scan_config: ScanConfig {
                    filter_accept_list: &targets,
                    ..scan_config(SPLIT_CENTRAL_SCAN_WINDOW)
                },
            };
            info!("Start connecting, {} peripheral(s) pending", pending.len());
            match with_timeout(Duration::from_secs(15), central.connect(&config)).await {
                Ok(Ok(conn)) => {
                    let peer = conn.peer_address();
                    if let Some(&(id, addr)) = pending.iter().find(|(_, addr)| Address::random(*addr) == peer) {
                        info!("Connected to peripheral {}", id);
                        peripheral_slots[id] = SlotState::Connected(addr);
                        conns[id].send(conn).await;
                    } else {
                        warn!("Connected peer {:?} matches no pending slot", peer.addr);
                    }
                }
                Ok(Err(e)) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Connect error: {:?}", e);
                    Timer::after_millis(500).await;
                }
                Err(_) => {
                    // None answered: forget the addresses and rediscover the
                    // peripherals when they come back.
                    warn!("Connect timeout, clearing {} address(es)", pending.len());
                    for &(id, _) in &pending {
                        peripheral_slots[id] = SlotState::NoAddr;
                    }
                }
            }
        } else if peripheral_slots.iter().any(|s| matches!(s, SlotState::NoAddr)) {
            // No `Disconnected` peripherals, check `NoAddr` peripheral slot and scan new peripherals
            info!("Start scanning peripherals");
            let session = start_scan(stack, SPLIT_CENTRAL_SCAN_WINDOW, &[]).await;
            let event = select(PERIPHERAL_FOUND.wait(), ended.ready_to_receive()).await;
            // Wait until the controller has confirmed the stop: it refuses an
            // initiator until then.
            session.stop().await;
            info!("Stop scanning");
            if let Either::First((id, addr)) = event {
                // The id comes off the air — bounds-check it. Keep the first
                // address seen for a slot; an occupied slot is cleared only
                // when connecting to it times out.
                match peripheral_slots.get_mut(id as usize) {
                    Some(slot) if matches!(slot, SlotState::NoAddr) => {
                        let addr = addr.into_inner();
                        info!("Scanned new peripheral {:?}", addr);
                        *slot = SlotState::Disconnected(addr);
                        FLASH_CHANNEL
                            .send(FlashOperationMessage::PeerAddress(PeerAddress::new(id, true, addr)))
                            .await;
                    }
                    _ => {}
                }
            }
        } else {
            // Fully linked: park until a session ends.
            ended.ready_to_receive().await;
        }
    }
}

// When no peripheral address is saved, the central should first scan for peripheral.
// This handler is used to handle the scan result.
pub(crate) struct ScanHandler;

impl EventHandler for ScanHandler {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = it.next() {
            let Some(Adv::SplitPeripheral { id }) = Adv::decode(report.data) else {
                continue;
            };
            info!("Found split peripheral: id={:?}, addr={:?}", id, report.addr);
            PERIPHERAL_FOUND.signal((id, report.addr));
            break;
        }
    }
}

/// Serve one peripheral slot: take each link established by
/// [`scan_and_connect_peripherals`], run the split session over it until it ends, and
/// report back so the radio reconnects or rediscovers the peripheral.
pub(crate) async fn run_peripheral_session<
    'a,
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
>(
    id: usize,
    conns: &Channel<NoopRawMutex, Connection<'a, DefaultPacketPool>, 1>,
    ended: &Channel<NoopRawMutex, usize, { crate::SPLIT_PERIPHERALS_NUM }>,
    stack: &'a Stack<'_, C, DefaultPacketPool>,
    matrix_config: PeripheralMatrixConfig,
) {
    trace!("SPLIT_MESSAGE_MAX_SIZE: {}", SPLIT_MESSAGE_MAX_SIZE);
    loop {
        let conn = conns.receive().await;
        set_peripheral_connected(id, true);
        if let Err(e) = run_central_manager_task(id, stack, &conn, matrix_config).await {
            #[cfg(feature = "defmt")]
            let e = defmt::Debug2Format(&e);
            error!("BLE central error: {:?}", e);
        }
        set_peripheral_connected(id, false);
        // Dropping the last handle files a disconnect request that the stack
        // runner serves on its own; the pause lets that finish and lets the
        // peripheral advertise again, so the reconnect finds a clean state.
        drop(conn);
        Timer::after_millis(500).await;
        ended.send(id).await;
    }
}

fn default_central_conn_param() -> RequestedConnParams {
    RequestedConnParams {
        min_connection_interval: Duration::from_micros(7500),
        max_connection_interval: Duration::from_micros(7500),
        max_latency: 10, // 75ms
        supervision_timeout: Duration::from_secs(5),
        ..Default::default()
    }
}

/// Parameters for the central -> peripheral link while the central sleeps.
///
/// With a host connected, the central's radio is busy serving the host link
/// anyway, so keep a short interval — the first key after wake-up arrives
/// quickly, and the peripheral still saves power through its latency. With no
/// host, a long interval also cuts the central-side radio wakeups.
fn sleep_central_conn_param() -> RequestedConnParams {
    if crate::state::active_transport().is_some() {
        RequestedConnParams {
            min_connection_interval: Duration::from_millis(20),
            max_connection_interval: Duration::from_millis(20),
            max_latency: 200, // 4s
            supervision_timeout: Duration::from_secs(9),
            ..Default::default()
        }
    } else {
        RequestedConnParams {
            min_connection_interval: Duration::from_millis(200),
            max_connection_interval: Duration::from_millis(200),
            max_latency: 25, // 5s
            supervision_timeout: Duration::from_secs(11),
            ..Default::default()
        }
    }
}

async fn run_central_manager_task<
    'b,
    's: 'b,
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    P: PacketPool,
>(
    id: usize,
    stack: &'b Stack<'s, C, P>,
    conn: &Connection<'b, P>,
    matrix_config: PeripheralMatrixConfig,
) -> Result<(), BleHostError<C::Error>> {
    let client = GattClient::<C, P, 10>::new(stack, conn).await?;

    // Split link uses 2M PHY always.
    update_ble_phy(stack, conn, PhyKind::Le2M).await;

    info!("Updating connection parameters for peripheral");
    update_conn_params(stack, conn, &default_central_conn_param()).await;

    let (Either3::First(e) | Either3::Second(e) | Either3::Third(e)) = select3(
        ble_central_task(&client, conn),
        discover_and_run_manager(id, &client, matrix_config),
        follow_sleep_state(stack, conn),
    )
    .await;
    e
}

async fn ble_central_task<'a, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool>(
    client: &GattClient<'a, C, P, 10>,
    conn: &Connection<'a, P>,
) -> Result<(), BleHostError<C::Error>> {
    // Watch for the disconnect; draining the other events keeps the small
    // per-connection queue from sitting full and dropping it.
    let conn_events = async {
        loop {
            if let ConnectionEvent::Disconnected { reason } = conn.next().await {
                info!("Connection lost: {:?}", reason);
                break;
            }
        }
    };

    match select(client.task(), conn_events).await {
        Either::First(e) => e,
        Either::Second(()) => Ok(()),
    }
}

/// Discover the split service on the connected peripheral, then run its
/// [`PeripheralManager`] over the GATT link.
async fn discover_and_run_manager<C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool>(
    id: usize,
    client: &GattClient<'_, C, P, 10>,
    matrix_config: PeripheralMatrixConfig,
) -> Result<(), BleHostError<C::Error>> {
    let services = client
        .services_by_uuid(&Uuid::new_long(SPLIT_SERVICE_UUID.to_le_bytes()))
        .await?;
    info!("Services found");
    let Some(service) = services.first() else {
        return Ok(());
    };
    let message_to_central = client
        .characteristic_by_uuid::<GattSplitMessage>(service, &Uuid::new_long(MESSAGE_TO_CENTRAL_UUID.to_le_bytes()))
        .await?;
    info!("Message to central found");
    let message_to_peripheral = client
        .characteristic_by_uuid::<GattSplitMessage>(service, &Uuid::new_long(MESSAGE_TO_PERIPHERAL_UUID.to_le_bytes()))
        .await?;
    info!("Subscribing notifications");
    let listener = client.subscribe(&message_to_central, false).await?;
    let split_ble_driver = BleSplitCentralDriver {
        listener,
        message_to_peripheral,
        client,
    };
    PeripheralManager::new(split_ble_driver, id, matrix_config).run().await;
    info!("Peripheral manager stopped");
    Ok(())
}

/// [`SplitReader`]/[`SplitWriter`] over the peripheral's GATT link: reads are
/// notifications on `message_to_central`, writes go to `message_to_peripheral`.
struct BleSplitCentralDriver<'a, 'b, 'c, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool> {
    listener: NotificationListener<'b, 512>,
    message_to_peripheral: Characteristic<GattSplitMessage>,
    client: &'c GattClient<'a, C, P, 10>,
}

impl<'a, 'b, 'c, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool> SplitReader
    for BleSplitCentralDriver<'a, 'b, 'c, C, P>
{
    async fn read(&mut self) -> Result<SplitMessage, SplitDriverError> {
        let data = self.listener.next().await;
        let message = postcard::from_bytes(data.as_ref()).map_err(|_| SplitDriverError::DeserializeError)?;
        info!("Received split message: {:?}", message);

        // Key events from the peripheral count as activity for sleep management
        if matches!(message, SplitMessage::Key(_) | SplitMessage::Pointing(_)) {
            debug!("Activity {:?} detected from peripheral", &message);
            report_activity();
        }

        Ok(message)
    }
}

impl<'a, 'b, 'c, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool> SplitWriter
    for BleSplitCentralDriver<'a, 'b, 'c, C, P>
{
    async fn write(&mut self, message: &SplitMessage) -> Result<usize, SplitDriverError> {
        let gatt_msg = GattSplitMessage::try_from(message)?;
        if let Err(e) = self
            .client
            .write_characteristic_without_response(&self.message_to_peripheral, gatt_msg.as_gatt())
            .await
        {
            if let BleHostError::BleHost(Error::NotFound) = e {
                error!("Peripheral disconnected");
                return Err(SplitDriverError::Disconnected);
            }
            #[cfg(feature = "defmt")]
            let e = defmt::Debug2Format(&e);
            error!("BLE message_to_peripheral_write error: {:?}", e);
        }

        Ok(gatt_msg.len)
    }
}

/// Keep one peripheral link's connection parameters in sync with the keyboard's
/// sleep state, published as [`SleepStateEvent`] by `crate::ble::sleep`. Runs
/// for as long as the link is up, so every link ends up with the same
/// parameters.
async fn follow_sleep_state<
    'b,
    's: 'b,
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    P: PacketPool,
>(
    stack: &'b Stack<'s, C, P>,
    conn: &Connection<'b, P>,
) -> Result<(), BleHostError<C::Error>> {
    let mut sleep_events = SleepStateEvent::subscriber();

    // A peripheral coming up is activity in its own right: it needs the fast
    // parameters for service discovery, and waking the whole keyboard keeps
    // every link on the same state.
    report_activity();

    // What this link's controller last accepted. `run_central_manager_task` just
    // applied the default (awake) parameters; tracking the applied value retries
    // a rejected update on the next state change instead of leaving the link at
    // the wrong interval until it reconnects.
    let mut applied = false;
    loop {
        let sleeping = sleep_events.next_event().await.0;
        if sleeping == applied {
            continue;
        }
        let params = if sleeping {
            sleep_central_conn_param()
        } else {
            default_central_conn_param()
        };
        if update_conn_params(stack, conn, &params).await {
            applied = sleeping;
        }
    }
}
