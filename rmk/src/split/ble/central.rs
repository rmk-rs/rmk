use core::cell::Cell;

use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy, LeSetScanParams};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer, with_timeout};
use rmk_types::ble::RMK_ADV_COMPANY_ID;
use trouble_host::prelude::*;

use super::GattSplitMessage;
use crate::ble::sleep::report_activity;
use crate::ble::{update_ble_phy, update_conn_params};
use crate::channel::FLASH_CHANNEL;
use crate::event::{EventSubscriber, SleepStateEvent, SubscribableEvent};
use crate::split::ble::PeerAddress;
use crate::split::driver::{PeripheralManager, SplitDriverError, SplitReader, SplitWriter, set_peripheral_connected};
use crate::split::{PeripheralMatrixConfig, SPLIT_MESSAGE_MAX_SIZE, SplitMessage};
use crate::storage::FlashOperationMessage;

pub(crate) static STACK_STARTED: Signal<crate::RawMutex, bool> = Signal::new();
static PERIPHERAL_FOUND: Signal<crate::RawMutex, (u8, BdAddr)> = Signal::new();

// Signals and mutex for syncing scanning state between scanning task and peripheral manager
static START_SCANNING: Signal<crate::RawMutex, ()> = Signal::new();
static STOP_SCANNING: Signal<crate::RawMutex, ()> = Signal::new();
static SCANNING_MUTEX: Mutex<crate::RawMutex, ()> = Mutex::new(());

/// The split GATT service (4dd5fbaa-18e5-4b07-bf0a-353698659946) hosted by the
/// peripheral, little-endian. The central only discovers it, so the service is
/// defined in `split::ble::peripheral`.
const SPLIT_SERVICE_UUID: [u8; 16] = [
    70u8, 153u8, 101u8, 152u8, 54u8, 53u8, 10u8, 191u8, 7u8, 75u8, 229u8, 24u8, 170u8, 251u8, 213u8, 77u8,
];

pub(crate) async fn scan_peripherals<
    C: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>,
>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    addrs: &[Cell<Option<[u8; 6]>>],
) {
    loop {
        // Wait unitil `START_SCANNING` is signaled
        START_SCANNING.wait().await;
        // Check whether the scanning is needed, aka there's empty slot in the addr list.
        let need_scan = !addrs.iter().all(|a| a.get().is_some());
        if need_scan {
            let scanning_fut = async {
                loop {
                    let mut central = stack.central();
                    wait_for_stack_started().await;
                    let mut scanner = Scanner::new(&mut central);
                    let scan_config = ScanConfig {
                        active: false,
                        interval: Duration::from_millis(100),
                        window: Duration::from_millis(30),
                        ..Default::default()
                    };
                    let _guard = SCANNING_MUTEX.lock().await;
                    if let Ok(_session) = scanner.scan(&scan_config).await {
                        info!("Start scanning peripherals");
                        STOP_SCANNING.wait().await;
                        info!("Stop scanning");
                    }
                }
            };
            let update_addrs_fut = async {
                loop {
                    let (found_peripheral_id, addr) = PERIPHERAL_FOUND.wait().await;
                    let scanned_addr = addr.into_inner();
                    // The id comes off the air — bounds-check it.
                    let Some(slot) = addrs.get(found_peripheral_id as usize) else {
                        continue;
                    };
                    if slot.get() == Some(scanned_addr) {
                        continue;
                    }

                    // Keep the first address seen for a slot; an occupied slot is
                    // cleared only when connecting to it times out.
                    if slot.get().is_none() {
                        info!("Scanned new peripheral {:?}", scanned_addr);
                        slot.set(Some(scanned_addr));
                        FLASH_CHANNEL
                            .send(FlashOperationMessage::PeerAddress(PeerAddress::new(
                                found_peripheral_id,
                                true,
                                scanned_addr,
                            )))
                            .await;
                    }

                    if addrs.iter().all(|a| a.get().is_some()) {
                        break;
                    }
                }
            };

            // Scan until all peripherals are scanned
            // TODO: Timeout?
            select(scanning_fut, update_addrs_fut).await;
        }
    }
}

// When no peripheral address is saved, the central should first scan for peripheral.
// This handler is used to handle the scan result.
pub(crate) struct ScanHandler {}

impl EventHandler for ScanHandler {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = it.next() {
            // Check advertisement data, `report.data[25]` is read below
            if report.data.len() < 26 {
                continue;
            }
            if report.data[4] == 0x07
                && report.data[5..].starts_with(&SPLIT_SERVICE_UUID)
                && report.data[21..23] == [0x04, 0xff]
                && report.data[23..25] == RMK_ADV_COMPANY_ID.to_le_bytes()
            {
                // Uuid and manufacturer specific data check passed
                let peripheral_id = report.data[25];
                info!("Found split peripheral: id={:?}, addr={:?}", peripheral_id, report.addr);
                PERIPHERAL_FOUND.signal((peripheral_id, report.addr));
                break;
            }
        }
    }
}

pub(crate) async fn run_ble_peripheral_manager<
    C: Controller
        + ControllerCmdSync<LeSetScanParams>
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>,
>(
    peri_id: usize,
    slot: &Cell<Option<[u8; 6]>>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    matrix_config: PeripheralMatrixConfig,
) {
    trace!("SPLIT_MESSAGE_MAX_SIZE: {}", SPLIT_MESSAGE_MAX_SIZE);

    loop {
        // Check until the address is available
        let address = loop {
            if let Some(addr) = slot.get() {
                break Address::random(addr);
            }
            if !START_SCANNING.signaled() {
                START_SCANNING.signal(());
            }
            // Check again after 500ms
            Timer::after_millis(500).await;
        };
        info!("Peripheral peer address: {:?}", address);

        let mut central = stack.central();
        let config = ConnectConfig {
            connect_params: default_central_conn_param(),
            scan_config: ScanConfig {
                filter_accept_list: &[address],
                active: false,
                interval: Duration::from_millis(100),
                window: Duration::from_millis(30),
                ..Default::default()
            },
        };
        wait_for_stack_started().await;

        set_peripheral_connected(peri_id, false);

        // Connect to peripheral
        match with_timeout(Duration::from_secs(15), async {
            if let Ok(_guard) = SCANNING_MUTEX.try_lock() {
                info!("Start connecting to peripheral {}", peri_id);
                central.connect(&config).await
            } else {
                STOP_SCANNING.signal(());
                let _guard = SCANNING_MUTEX.lock().await;
                // Wait a little bit to ensure that the scanning has been fully stopped
                Timer::after_millis(100).await;
                info!("Start connecting to peripheral {}", peri_id);
                central.connect(&config).await
            }
        })
        .await
        {
            Ok(Ok(conn)) => {
                info!("Connected to peripheral {}", peri_id);

                set_peripheral_connected(peri_id, true);

                if let Err(e) = run_central_manager_task(peri_id, stack, &conn, matrix_config).await {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("BLE central error: {:?}", e);
                }
            }
            Ok(Err(e)) => {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                error!("Connect to peripheral {} error: {:?}", peri_id, e);
            }
            Err(_) => {
                // Connect to peripheral timeout
                warn!("Connect to peripheral {} timeout, clearing", peri_id);
                slot.set(None);
            }
        }
        // Reconnect after 500ms
        Timer::after_millis(500).await;
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

    match select3(
        ble_central_task(&client, conn),
        discover_and_run_manager(id, &client, matrix_config),
        follow_sleep_state(stack, conn),
    )
    .await
    {
        Either3::First(e) => e,
        Either3::Second(e) => e,
        Either3::Third(e) => e,
    }
}

async fn ble_central_task<'a, C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool>(
    client: &GattClient<'a, C, P, 10>,
    conn: &Connection<'a, P>,
) -> Result<(), BleHostError<C::Error>> {
    // Simply monitor connection status
    let conn_check = async {
        while conn.is_connected() {
            Timer::after_secs(5).await;
        }
    };

    match select(client.task(), conn_check).await {
        Either::First(e) => e,
        Either::Second(_) => {
            info!("Connection lost");
            Ok(())
        }
    }
}

/// Discover the split service on the connected peripheral, then run its
/// [`PeripheralManager`] over the GATT link.
async fn discover_and_run_manager<C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool>(
    id: usize,
    client: &GattClient<'_, C, P, 10>,
    matrix_config: PeripheralMatrixConfig,
) -> Result<(), BleHostError<C::Error>> {
    let services = client.services_by_uuid(&Uuid::new_long(SPLIT_SERVICE_UUID)).await?;
    info!("Services found");
    if let Some(service) = services.first() {
        let message_to_central = client
            .characteristic_by_uuid::<GattSplitMessage>(
                service,
                // uuid: 0e6313e3-bd0b-45c2-8d2e-37a2e8128bc3
                &Uuid::Uuid128([
                    195u8, 139u8, 18u8, 232u8, 162u8, 55u8, 46u8, 141u8, 194u8, 69u8, 11u8, 189u8, 227u8, 19u8, 99u8,
                    14u8,
                ]),
            )
            .await?;
        info!("Message to central found");
        let message_to_peripheral = client
            .characteristic_by_uuid::<GattSplitMessage>(
                service,
                // uuid: 4b3514fb-cae4-4d38-a097-3a2a3d1c3b9c
                &Uuid::Uuid128([
                    156u8, 59u8, 28u8, 61u8, 42u8, 58u8, 151u8, 160u8, 56u8, 77u8, 228u8, 202u8, 251u8, 20u8, 53u8,
                    75u8,
                ]),
            )
            .await?;
        info!("Subscribing notifications");
        let listener = client.subscribe(&message_to_central, false).await?;
        let split_ble_driver = BleSplitCentralDriver {
            listener,
            message_to_peripheral,
            client,
        };
        let peripheral_manager = PeripheralManager::new(split_ble_driver, id, matrix_config);
        peripheral_manager.run().await;
        info!("Peripheral manager stopped");
    };
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

/// Wait until the BLE stack's runner is up (latched by `serve`), plus a 500ms
/// grace period. Polled because the one-shot latch has multiple waiters.
async fn wait_for_stack_started() {
    while !STACK_STARTED.signaled() {
        Timer::after_millis(500).await;
    }
    Timer::after_millis(500).await;
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
