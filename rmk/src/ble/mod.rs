use core::cell::Cell;

#[cfg(not(feature = "ext_adv"))]
use bt_hci::cmd::le::LeSetScanParams;
#[cfg(feature = "subrating")]
use bt_hci::cmd::le::LeSubrateRequest;
#[cfg(feature = "ext_adv")]
use bt_hci::cmd::le::{
    LeClearAdvSets, LeExtCreateConn, LeReadNumberOfSupportedAdvSets, LeSetAdvSetRandomAddr, LeSetExtAdvData,
    LeSetExtAdvParams, LeSetExtScanParams, LeSetExtScanResponseData,
};
use bt_hci::cmd::le::{LeRand, LeReadLocalSupportedFeatures, LeSetPhy};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use bt_hci::param::Error as HciError;
use embassy_futures::join::{join_array, join5};
use embassy_futures::select::{Either, Either3, select, select3, select5};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use heapless::LinearMap;
use rmk_types::ble::BleState;
use rmk_types::connection::ConnectionType;
use rmk_types::led_indicator::LedIndicator;
use trouble_host::prelude::*;
use usbd_hid::descriptor::{MediaKeyboardReport, SystemControlReport};

use crate::BOND_SLOTS;
use crate::ble::adv::{Adv, advertise};
use crate::ble::battery_service::BleBatteryServer;
#[cfg(feature = "split")]
use crate::ble::battery_service::BlePeripheralBatteryServer;
use crate::ble::ble_server::{BleHidServer, Server};
use crate::ble::device_info::{PnPID, VidSource};
#[cfg(feature = "host")]
use crate::ble::host::{GattWriteResult, HOST_WRITE_BUFFER_SIZE, HostConnection};
use crate::ble::led::BleLedReader;
#[cfg(feature = "passkey_entry")]
use crate::ble::passkey::{PasskeyInputState, next_gatt_event};
#[cfg(feature = "dongle")]
use crate::ble::profile::DONGLE_PROFILE;
use crate::ble::profile::{ProfileInfo, ProfileManager, load_stack_bond};
use crate::channel::{BLE_PROFILE_CHANNEL, BLE_REPORT_CHANNEL, LED_SIGNAL};
use crate::config::{BleBatteryConfig, DeviceConfig, RmkConfig};
use crate::core_traits::Runnable;
use crate::event::SubscribableEvent;
use crate::hid::{HidWriterTrait, Report, run_led_reader};
use crate::sleep::{report_activity, request_sleep};
#[cfg(feature = "split")]
use crate::split::PeripheralMatrixConfig;
#[cfg(feature = "split")]
use crate::split::ble::central::{run_peripheral_session, scan_and_connect_peripherals};
use crate::state::{set_ble_state, wait_for_connection_status};

pub(crate) mod adv;
pub(crate) mod battery_service;
pub(crate) mod ble_server;
pub(crate) mod device_info;
#[cfg(test)]
mod fake_hci;
#[cfg(feature = "host")]
pub(crate) mod host;
pub(crate) mod led;
#[cfg(all(feature = "_nrf_ble", target_arch = "arm", target_os = "none"))]
mod p256;
pub mod passkey;
pub(crate) mod profile;
pub(crate) mod rng;
#[cfg(any(feature = "split", feature = "dongle"))]
pub(crate) mod scan;

#[cfg(all(feature = "subrating", feature = "_no_subrating"))]
compile_error!("You may not enable feature `subrating` on unsupported platforms!");

#[cfg(all(feature = "pico_w_ble", feature = "ext_adv"))]
compile_error!("Pico W does not support extended HCI advertising commands");

/// Additional HCI commands required for extended advertising.
#[doc(hidden)]
#[cfg(feature = "ext_adv")]
pub trait ExtAdvCmds:
    for<'t> ControllerCmdSync<LeSetExtAdvData<'t>>
    + ControllerCmdSync<LeClearAdvSets>
    + ControllerCmdSync<LeSetExtAdvParams>
    + ControllerCmdSync<LeSetAdvSetRandomAddr>
    + ControllerCmdSync<LeReadNumberOfSupportedAdvSets>
    + for<'t> ControllerCmdSync<LeSetExtScanResponseData<'t>>
{
}

#[cfg(feature = "ext_adv")]
impl<T> ExtAdvCmds for T where
    T: for<'t> ControllerCmdSync<LeSetExtAdvData<'t>>
        + ControllerCmdSync<LeClearAdvSets>
        + ControllerCmdSync<LeSetExtAdvParams>
        + ControllerCmdSync<LeSetAdvSetRandomAddr>
        + ControllerCmdSync<LeReadNumberOfSupportedAdvSets>
        + for<'t> ControllerCmdSync<LeSetExtScanResponseData<'t>>
{
}

#[cfg(not(feature = "ext_adv"))]
#[doc(hidden)]
pub trait ExtAdvCmds {}

#[cfg(not(feature = "ext_adv"))]
impl<T> ExtAdvCmds for T {}

/// Additional HCI commands required for central scanning and connecting.
#[doc(hidden)]
#[cfg(feature = "ext_adv")]
pub trait CentralCmds: ControllerCmdSync<LeSetExtScanParams> + ControllerCmdAsync<LeExtCreateConn> {}

#[cfg(feature = "ext_adv")]
impl<T> CentralCmds for T where T: ControllerCmdSync<LeSetExtScanParams> + ControllerCmdAsync<LeExtCreateConn> {}

#[cfg(not(feature = "ext_adv"))]
#[doc(hidden)]
pub trait CentralCmds: ControllerCmdSync<LeSetScanParams> {}

#[cfg(not(feature = "ext_adv"))]
impl<T: ControllerCmdSync<LeSetScanParams>> CentralCmds for T {}

/// Simultaneous host and dongle connections, set by `[ble] max_connections`.
#[doc(hidden)]
pub const NUM_HOST_LINKS: usize = crate::NUM_HOST_LINKS;

/// Max number of connections of a keyboard's BLE stack; a dongle sizes its
/// own — see [`crate::dongle::Dongle`].
#[doc(hidden)]
pub const CONNECTIONS_MAX: usize = crate::SPLIT_PERIPHERALS_NUM + NUM_HOST_LINKS;

/// Calculate ESP BLE controller capacity for central and peripheral links.
#[doc(hidden)]
#[cfg(feature = "_esp_ble")]
pub const fn calculate_esp_ble_capacity(central_links: usize, peripheral_links: usize) -> u8 {
    let mut capacity = central_links + peripheral_links;
    let limit = if cfg!(any(feature = "esp32c3_ble", feature = "esp32s3_ble")) {
        // C3/S3 share activity slots between connections, scanning, and advertising.
        capacity += (central_links > 0) as usize + (peripheral_links > 0) as usize;
        10
    } else if cfg!(feature = "esp32s31_ble") {
        10
    } else if cfg!(feature = "esp32h2_ble") {
        35
    } else {
        70
    };
    assert!(
        capacity > 0 && capacity <= limit,
        "BLE controller capacity is outside the chip's limits"
    );
    capacity as u8
}

/// Default SDC Link Layer TX/RX packet size, in bytes.
pub const SDC_PACKET_SIZE: usize = 251;
/// Default number of SDC TX packet buffers per connection.
pub const SDC_TX_BUFFERS: u8 = 3;
/// Default number of SDC RX packet buffers per connection.
pub const SDC_RX_BUFFERS: u8 = 3;

/// Calculate SDC memory using the same packet size and buffer counts as the controller.
#[doc(hidden)]
pub const fn calculate_sdc_memory_size(
    central_links: usize,
    peripheral_links: usize,
    packet_size: usize,
    tx_buffers: u8,
    rx_buffers: u8,
) -> usize {
    // SDC_MEM_PER_{CENTRAL,PERIPHERAL}_LINK plus their shared-memory bounds.
    let buffers = (tx_buffers as usize + rx_buffers as usize) * (packet_size + 15) - 84;
    let links = central_links * (803 + buffers + 21) + peripheral_links * (923 + buffers + 17);
    let advertising = (5184 + 31 * 18) / 10; // SDC_MEM_PER_ADV_SET(31).
    // SDC_MEM_SCAN[_EXT](3); scanning and initiating run sequentially.
    let scanning = if central_links > 0 {
        408 + 3 * if cfg!(feature = "ext_adv") { 320 } else { 104 }
    } else {
        0
    };
    let subrating = if cfg!(feature = "subrating") {
        12 + (central_links + peripheral_links) * 60
    } else {
        0
    };
    let filter_accept_list = 4 + 8 * 8; // SDC_MEM_FAL(8).
    (links + advertising + scanning + subrating + filter_accept_list + 7) & !7
}

// Fixed ATT/SMP/signalling channels do not consume dynamic L2CAP slots.
const L2CAP_CHANNELS_MAX: usize = 0;

// Custom messages share the GATT write dispatcher with the host protocol.
const GATT_WRITE_BUFFER_SIZE: usize = {
    #[cfg(feature = "host")]
    let size = HOST_WRITE_BUFFER_SIZE;
    #[cfg(not(feature = "host"))]
    let size = 32;
    #[cfg(all(feature = "dongle", feature = "custom_message"))]
    let size = {
        let custom =
            <crate::custom_message::CustomMessage as postcard::experimental::max_size::MaxSize>::POSTCARD_MAX_SIZE;
        if custom > size { custom } else { size }
    };
    size
};

/// BLE transport. Owns the whole BLE stack.
///
/// On a split build the transport is the BLE split central:
/// `run` also loads the peripherals' stored addresses and drives the
/// radio and per-peripheral session tasks on the same stack.
pub struct BleTransport<'a, C>
where
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    /// Taken by `run`: the stack it builds consumes the controller.
    /// The option can be removed by changing to `run(self)`, but it requires
    /// https://github.com/rust-lang/rust/issues/59087 to be fixed
    controller: Option<C>,
    address: [u8; 6],
    device_config: DeviceConfig<'static>,
    config: BleBatteryConfig,
    /// One matrix region per split peripheral.
    #[cfg(feature = "split")]
    peripheral_matrices: [PeripheralMatrixConfig; crate::SPLIT_PERIPHERALS_NUM],
    #[cfg(feature = "host")]
    host_service: Option<&'a crate::host::HostService<'a>>,
    // Keeps `'a` in the type's parameter list across all feature configurations.
    #[cfg(not(feature = "host"))]
    _phantom: core::marker::PhantomData<&'a ()>,
}

impl<'a, C> BleTransport<'a, C>
where
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    pub fn new(
        controller: C,
        address: [u8; 6],
        rmk_config: RmkConfig<'static>,
        #[cfg(feature = "split")] peripheral_matrices: [PeripheralMatrixConfig; crate::SPLIT_PERIPHERALS_NUM],
    ) -> Self {
        Self {
            controller: Some(controller),
            address,
            device_config: rmk_config.device_config,
            config: rmk_config.ble_battery_config,
            #[cfg(feature = "split")]
            peripheral_matrices,
            #[cfg(feature = "host")]
            host_service: None,
            #[cfg(not(feature = "host"))]
            _phantom: core::marker::PhantomData,
        }
    }

    /// Attach the host-protocol service (Vial or Rynk, picked at compile
    /// time by feature). See
    /// [`UsbTransport::with_host_service`](crate::usb::UsbTransport::with_host_service).
    #[cfg(feature = "host")]
    pub fn with_host_service(mut self, service: &'a crate::host::HostService<'a>) -> Self {
        self.host_service = Some(service);
        self
    }
}

#[cfg(not(feature = "split"))]
impl<'a, C> Runnable for BleTransport<'a, C>
where
    C: Controller
        + ExtAdvCmds
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeRand>,
{
    async fn run(&mut self) -> ! {
        let controller = self.controller.take().expect("BleTransport::run called twice");
        rng::init(&controller).await;

        // Load the preferred connection from storage
        let preferred = crate::state::load_preferred_connection().await;
        crate::state::set_preferred_connection(preferred);

        // Host connections share the stack resources.
        let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, 1, BOND_SLOTS> =
            HostResources::new();
        let stack = trouble_host::new(controller, &mut resources)
            .set_random_address(Address::random(self.address))
            .build();
        run_ble_keyboard(
            &stack,
            self.address,
            &self.device_config,
            &self.config,
            #[cfg(feature = "host")]
            self.host_service,
        )
        .await
    }
}

#[cfg(feature = "split")]
impl<
    'a,
    #[cfg(not(feature = "subrating"))] C: Controller
        + CentralCmds
        + ExtAdvCmds
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeRand>,
    #[cfg(feature = "subrating")] C: Controller
        + CentralCmds
        + ExtAdvCmds
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeRand>
        + ControllerCmdAsync<LeSubrateRequest>,
> Runnable for BleTransport<'a, C>
{
    async fn run(&mut self) -> ! {
        let controller = self.controller.take().expect("BleTransport::run called twice");
        rng::init(&controller).await;

        // Load the preferred connection from storage
        let preferred = crate::state::load_preferred_connection().await;
        crate::state::set_preferred_connection(preferred);

        let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, 1, BOND_SLOTS> =
            HostResources::new();
        let stack = trouble_host::new(controller, &mut resources)
            .set_random_address(Address::random(self.address))
            .build();

        // The connect peripherals task hands each established connection to its
        // session task, and a session task reports back when its session ends.
        let conn_channels: [Channel<NoopRawMutex, Connection<'_, DefaultPacketPool>, 1>; crate::SPLIT_PERIPHERALS_NUM] =
            core::array::from_fn(|_| Channel::new());
        let ended: Channel<NoopRawMutex, usize, { crate::SPLIT_PERIPHERALS_NUM }> = Channel::new();

        let sessions =
            embassy_futures::join::join_array(core::array::from_fn::<_, { crate::SPLIT_PERIPHERALS_NUM }, _>(|i| {
                run_peripheral_session(i, &conn_channels[i], &ended, &stack, self.peripheral_matrices[i])
            }));
        embassy_futures::join::join3(
            run_ble_keyboard(
                &stack,
                self.address,
                &self.device_config,
                &self.config,
                #[cfg(feature = "host")]
                self.host_service,
            ),
            sessions,
            scan_and_connect_peripherals(&stack, &conn_channels, &ended),
        )
        .await;
        unreachable!("BleTransport sub-tasks must run forever")
    }
}

/// Owns the GATT server and the profile manager, and advertises→connects→
/// serves forever, joined with the stack runner and the sleep manager.
async fn run_ble_keyboard<
    #[cfg(feature = "host")] 'r,
    C: Controller + ExtAdvCmds + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    address: [u8; 6],
    device_config: &DeviceConfig<'static>,
    config: &BleBatteryConfig,
    #[cfg(feature = "host")] host_service: Option<&'r crate::host::HostService<'r>>,
) -> ! {
    let product_name = device_config.product_name;

    info!("Starting advertising and GATT service");
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: product_name,
        appearance: &appearance::human_interface_device::KEYBOARD,
    }))
    .unwrap();

    server
        .set(
            &server.device_config_service.pnp_id,
            &PnPID {
                vid_source: VidSource::UsbIF,
                vendor_id: device_config.vid,
                product_id: device_config.pid,
                product_version: 0x0001,
            },
        )
        .unwrap();
    server
        .set(
            &server.device_config_service.serial_number,
            &device_config.serial_number,
        )
        .unwrap();
    server
        .set(
            &server.device_config_service.manufacturer_name,
            &heapless::String::try_from(device_config.manufacturer).expect("manufacturer is too long for BLE"),
        )
        .unwrap();

    let server = &server;

    if crate::ble::passkey::passkey_entry_enabled() {
        stack.set_io_capabilities(IoCapabilities::KeyboardOnly);
    }
    let mut profile_manager: ProfileManager<BOND_SLOTS> = ProfileManager::new();
    // Load the bonded devices from storage
    profile_manager.load_bonded_devices().await;

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    let accepted_connections: Channel<NoopRawMutex, (GattConnection<'_, '_, DefaultPacketPool>, u8), 1> =
        Channel::new();
    let completed: Channel<NoopRawMutex, u8, NUM_HOST_LINKS> = Channel::new();
    let workers = join_array(core::array::from_fn::<_, NUM_HOST_LINKS, _>(|_| async {
        loop {
            let (conn, profile_id) = accepted_connections.receive().await;
            serve_keyboard_connection(
                server,
                &conn,
                stack,
                profile_id,
                &profile_manager,
                config,
                #[cfg(feature = "host")]
                host_service,
            )
            .await;
            drop(conn);
            completed.send(profile_id).await;
        }
    }));

    let connection_loop = async {
        // Keep host and dongle handles until their GATT workers finish.
        let mut host_connections = LinearMap::<u8, Connection<'_, DefaultPacketPool>, NUM_HOST_LINKS>::new();
        const ADV_TIMEOUT: Duration = Duration::from_secs(300);
        // A bonded host usually reconnects soon, so its advertising starts fast and then slows.
        const FAST_ADV_DURATION: Duration = Duration::from_secs(30);
        let mut adv_deadline = Some(Instant::now() + ADV_TIMEOUT);
        // Let USB detection run before the first advertisement.
        Timer::after_millis(10).await;

        loop {
            let profile_id = crate::state::current_profile();
            let local_address = cfg!(feature = "ext_adv")
                .then(|| BdAddr::new(crate::ble::profile::profile_address(address, profile_id)));
            let bond = profile_manager.bond_info(profile_id);

            let advertise_and_connect = async {
                // Advertise only while this profile has no link and a worker is free.
                if host_connections.contains_key(&profile_id) || host_connections.len() == NUM_HOST_LINKS {
                    return core::future::pending().await;
                }

                let deadline = match adv_deadline {
                    Some(deadline) => deadline,
                    None => {
                        let mut keys = crate::event::KeyboardEvent::subscriber();
                        let mut pointing = crate::event::PointingEvent::subscriber();
                        let _ = select(keys.next_message_pure(), pointing.next_message_pure()).await;
                        report_activity();
                        *adv_deadline.insert(Instant::now() + ADV_TIMEOUT)
                    }
                };

                let fast_until = deadline - (ADV_TIMEOUT - FAST_ADV_DURATION);
                let adv = match &bond {
                    #[cfg(feature = "dongle")]
                    Some(bond) if profile_id == DONGLE_PROFILE => Adv::Directed(bond.info.identity.addr),
                    #[cfg(feature = "dongle")]
                    None if profile_id == DONGLE_PROFILE => Adv::DongleSeeking,
                    bond => Adv::Host {
                        name: product_name,
                        fast: bond.is_some() && Instant::now() < fast_until,
                    },
                };
                // Fast advertising stops early, so it restarts at the slow interval.
                let stop = match adv {
                    Adv::Host { fast: true, .. } => fast_until,
                    _ => deadline,
                };
                load_stack_bond(stack, bond.as_ref().map(|bond| bond.info.clone()));
                set_ble_state(profile_id, BleState::Advertising);
                advertise(
                    &mut peripheral,
                    &server.server,
                    adv,
                    local_address,
                    stop.saturating_duration_since(Instant::now()),
                )
                .await
            };

            // Consume queued wake input before a profile action can cancel the connection flow.
            let gatt_conn = match select3(
                advertise_and_connect,
                BLE_PROFILE_CHANNEL.receive(),
                completed.receive(),
            )
            .await
            {
                Either3::First(Ok(gatt_conn)) => gatt_conn,
                // Only fast advertising ended; the advertising period continues.
                Either3::First(Err(BleHostError::BleHost(Error::Timeout)))
                    if adv_deadline.is_some_and(|deadline| Instant::now() < deadline) =>
                {
                    continue;
                }
                Either3::First(Err(BleHostError::BleHost(Error::Timeout))) => {
                    profile_manager.set_pairing(None);
                    adv_deadline = None;
                    set_ble_state(profile_id, BleState::Inactive);
                    request_sleep();
                    continue;
                }
                Either3::First(Err(e)) => {
                    set_ble_state(profile_id, BleState::Inactive);
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Advertise error: {:?}", e);
                    Timer::after_millis(200).await;
                    continue;
                }
                Either3::Second(action) => {
                    use crate::ble::profile::BleProfileAction::{ClearBond, Disconnect, Next, Previous, Switch};

                    let last_host = crate::NUM_BLE_PROFILE.saturating_sub(1) as u8;
                    let slot = match action {
                        Switch(slot) | Disconnect(slot) => slot,
                        ClearBond(slot) => slot.unwrap_or(profile_id),
                        Next if profile_id < last_host => profile_id + 1,
                        Next => 0,
                        Previous => profile_id.checked_sub(1).unwrap_or(last_host),
                    };
                    if matches!(action, Disconnect(_) | ClearBond(_)) {
                        if slot == profile_id {
                            // The selected profile waits for a new host.
                            profile_manager.set_pairing(Some(slot));
                            set_ble_state(slot, BleState::Inactive);
                            report_activity();
                            adv_deadline = Some(Instant::now() + ADV_TIMEOUT);
                        }
                        if let Some(conn) = host_connections.get(&slot) {
                            conn.disconnect();
                        }
                        if matches!(action, ClearBond(_)) {
                            profile_manager.clear_bond(slot);
                        }
                        continue;
                    }
                    if slot == profile_id {
                        continue;
                    }
                    // End re-pairing before a switch leaves this profile's status behind.
                    profile_manager.set_pairing(None);
                    report_activity();
                    adv_deadline = Some(Instant::now() + ADV_TIMEOUT);
                    // Keep a paired link unless the target profile needs its slot.
                    let keep_connected = (host_connections.contains_key(&slot)
                        || host_connections.len() < NUM_HOST_LINKS)
                        && host_connections.get(&profile_id).is_some_and(|conn| {
                            conn.security_level().is_ok_and(|level| level.encrypted())
                                && profile_manager
                                    .bond_info(profile_id)
                                    .is_some_and(|bond| bond.info.identity.match_identity(&conn.peer_identity()))
                        });
                    if !keep_connected {
                        set_ble_state(profile_id, BleState::Inactive);
                        if let Some(conn) = host_connections.get(&profile_id) {
                            conn.disconnect();
                        }
                    }
                    profile_manager.switch_profile(slot);
                    continue;
                }
                Either3::Third(ended) => {
                    host_connections.remove(&ended);
                    if ended == profile_id && crate::state::current_ble_status().state == BleState::Connected {
                        adv_deadline = Some(Instant::now() + ADV_TIMEOUT);
                    }
                    set_ble_state(ended, BleState::Inactive);
                    continue;
                }
            };

            let conn = gatt_conn.raw();
            let peer = conn.peer_identity();
            // A replaced advertisement can still deliver a late connection.
            #[cfg(feature = "ext_adv")]
            if conn.local_address() != local_address {
                warn!("[ble] late connection belongs to another profile");
                disconnect(&gatt_conn).await;
                continue;
            }
            #[cfg(feature = "dongle")]
            if (profile_id == DONGLE_PROFILE
                && bond
                    .as_ref()
                    .is_some_and(|bond| !bond.info.identity.match_identity(&peer)))
                || (profile_id != DONGLE_PROFILE && profile_manager.is_bonded_dongle(&peer))
            {
                warn!("[ble] peer does not match its dongle profile");
                disconnect(&gatt_conn).await;
                continue;
            }
            if let Err(e) = conn.set_bondable(true) {
                error!("Set bondable error: {:?}", e);
                continue;
            }
            if let Some(bond) = bond {
                if bond.info.identity.match_identity(&peer) {
                    match ClientAttTableView::try_from_raw(&bond.cccd_table) {
                        Ok(view) => server.set_client_att_table(conn, &view),
                        Err(e) => warn!("Invalid stored CCCD table: {:?}", e),
                    }
                } else if let Err(e) = stack.remove_bond_information(bond.info.identity) {
                    warn!("Remove previous peer from BLE stack: {:?}", e);
                }
            }
            assert!(host_connections.insert(profile_id, conn.clone()).is_ok());
            accepted_connections.send((gatt_conn, profile_id)).await;
        }
    };

    #[cfg(feature = "split")]
    let event_handler = crate::split::ble::central::ScanHandler;
    #[cfg(not(feature = "split"))]
    let event_handler = NoopHandler;

    let mut ble_led_reader = BleLedReader;
    join5(
        ble_task(runner, &event_handler),
        connection_loop,
        workers,
        run_led_reader(&mut ble_led_reader, ConnectionType::Ble),
        profile_manager.run(),
    )
    .await;
    unreachable!("BleTransport sub-tasks must run forever")
}

/// NoopHandler is used on the device which never scans,
/// such as a split peripheral or a normal keyboard.
pub(crate) struct NoopHandler;

impl EventHandler for NoopHandler {}

/// Latched by [`ble_task`] on its first poll, so the roles that drive the same
/// stack know the runner is about to pump it.
static STACK_STARTED: Signal<crate::RawMutex, ()> = Signal::new();

/// Wait until [`ble_task`] is up, plus a grace period. Polled because the
/// one-shot latch has multiple waiters.
pub(crate) async fn wait_for_stack_started() {
    while !STACK_STARTED.signaled() {
        Timer::after_millis(500).await;
    }
    Timer::after_millis(500).await;
}

/// This is a background task that is required to run forever alongside any other BLE tasks.
pub(crate) async fn ble_task<C: Controller, P: PacketPool, E: EventHandler>(mut runner: Runner<'_, C, P>, handler: &E) {
    STACK_STARTED.signal(());
    if let Err(e) = runner.run_with_handler(handler).await {
        error!("[ble_task] runner stopped, rebooting: {:?}", e);
        Timer::after_millis(100).await;
        crate::boot::reboot_keyboard();
    }
}

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
async fn gatt_events_task(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    profile_id: u8,
    profile_manager: &ProfileManager<BOND_SLOTS>,
    led: &Cell<LedIndicator>,
    #[cfg(feature = "host")] host: Option<&HostConnection<'_>>,
) {
    let level = &server.battery_service.level;
    #[cfg(feature = "split")]
    let peripheral_levels = &server.peripheral_battery_services.levels;
    let output_keyboard = &server.hid_service.output_keyboard;
    let hid_control_point = &server.hid_service.hid_control_point;
    let input_keyboard = &server.hid_service.input_keyboard;
    let mouse = &server.hid_service.mouse_report;
    let media = &server.hid_service.media_report;
    let system_control = &server.hid_service.system_report;

    #[cfg(feature = "passkey_entry")]
    let mut passkey_state = PasskeyInputState::new();

    loop {
        #[cfg(feature = "passkey_entry")]
        let Some(event) = next_gatt_event(conn, &mut passkey_state).await else {
            continue;
        };
        #[cfg(not(feature = "passkey_entry"))]
        let event = conn.next().await;

        match event {
            GattConnectionEvent::Disconnected { reason } => {
                #[cfg(feature = "passkey_entry")]
                passkey_state.clear();
                info!("[gatt] disconnected: {:?}", reason);
                break;
            }
            GattConnectionEvent::PairingComplete { bond: Some(_), .. }
            | GattConnectionEvent::PassKeyInput
            | GattConnectionEvent::BondLost
                if profile_id != crate::state::current_profile() =>
            {
                conn.raw().disconnect();
            }
            GattConnectionEvent::PairingComplete { security_level, bond } => {
                #[cfg(feature = "passkey_entry")]
                passkey_state.clear();
                info!("[gatt] pairing complete: {:?}", security_level);
                if let Some(bond_info) = bond {
                    let cccd_table = server
                        .get_client_att_table(conn.raw())
                        .and_then(|t| heapless::Vec::from_slice(t.raw()).ok())
                        .unwrap_or_default();
                    profile_manager.set_bond(ProfileInfo {
                        slot_num: profile_id,
                        info: bond_info,
                        removed: false,
                        cccd_table,
                    });
                }
            }
            GattConnectionEvent::PairingFailed(err) => {
                #[cfg(feature = "passkey_entry")]
                passkey_state.clear();
                error!("[gatt] pairing error: {:?}", err);
                conn.raw().disconnect();
            }
            GattConnectionEvent::Encrypted { security_level, .. } => {
                info!("[gatt] encrypted: {:?}", security_level);
                set_ble_state(profile_id, BleState::Connected);
            }
            GattConnectionEvent::PhyUpdateFailed { .. } | GattConnectionEvent::ConnectionParamsUpdateFailed { .. } => {
                warn!("[gatt] link parameter update failed");
            }
            GattConnectionEvent::Gatt { event: gatt_event } => {
                let mut cccd_updated = false;
                let result = match &gatt_event {
                    GattEvent::Read(event) => {
                        debug!("Read GATT Event: {:?}", event.handle());
                        let Ok(security_level) = conn.raw().security_level() else {
                            continue;
                        };
                        if security_level.encrypted() {
                            server.refresh_battery_level(event.handle());
                            None
                        } else {
                            Some(AttErrorCode::INSUFFICIENT_ENCRYPTION)
                        }
                    }
                    GattEvent::Write(event) => {
                        #[cfg(all(feature = "dongle", feature = "custom_message"))]
                        let is_custom_message = event.handle() == server.dongle_event_service.custom_to_keyboard.handle;
                        #[cfg(not(all(feature = "dongle", feature = "custom_message")))]
                        let is_custom_message = false;
                        let Ok(security_level) = conn.raw().security_level() else {
                            continue;
                        };
                        let encrypted = security_level.encrypted();

                        // trouble-host 0.7 exposes written bytes via a closure; copy them out
                        // once so the dispatch below (which awaits) can use them freely.
                        let mut data_buf = [0u8; GATT_WRITE_BUFFER_SIZE];
                        let data_len = event.with_data(|_, data| {
                            let n = data.len().min(data_buf.len());
                            data_buf[..n].copy_from_slice(&data[..n]);
                            data.len()
                        });
                        let data = &data_buf[..data_len.min(data_buf.len())];
                        let mut control_point_write = false;

                        if event.handle() == output_keyboard.handle {
                            if data_len == 1 {
                                let led_indicator = LedIndicator::from_bits(data[0]);
                                debug!("Got keyboard state: {:?}", led_indicator);
                                led.set(led_indicator);
                                if profile_id == crate::state::current_profile() {
                                    LED_SIGNAL.signal(led_indicator);
                                }
                            } else {
                                warn!("Wrong keyboard state data: {:?}", data);
                            }
                        } else if event.handle() == input_keyboard.cccd_handle.expect("No CCCD for input keyboard")
                            || event.handle() == mouse.cccd_handle.expect("No CCCD for mouse report")
                            || event.handle() == media.cccd_handle.expect("No CCCD for media report")
                            || event.handle() == system_control.cccd_handle.expect("No CCCD for system report")
                            || event.handle() == level.cccd_handle.expect("No CCCD for battery level")
                            || {
                                #[cfg(feature = "split")]
                                {
                                    peripheral_levels.iter().any(|level| {
                                        event.handle()
                                            == level.cccd_handle.expect("No CCCD for peripheral battery level")
                                    })
                                }
                                #[cfg(not(feature = "split"))]
                                {
                                    false
                                }
                            }
                        {
                            cccd_updated = true;
                        } else if event.handle() == hid_control_point.handle {
                            control_point_write = true;
                        } else if is_custom_message {
                            #[cfg(all(feature = "dongle", feature = "custom_message"))]
                            match postcard::from_bytes::<crate::custom_message::CustomMessage>(data) {
                                Ok(message) => match message.target {
                                    #[cfg(feature = "split")]
                                    crate::custom_message::CustomMessageTarget::Peripherals => {
                                        crate::custom_message::send(message)
                                    }
                                    crate::custom_message::CustomMessageTarget::Central => {
                                        crate::event::publish_event(message)
                                    }
                                    _ => (),
                                },
                                Err(_) => warn!("[ble] undecodable custom message dropped"),
                            }
                        } else {
                            #[cfg(feature = "host")]
                            if let Some(host) = host {
                                match host.handle_write(event.handle(), data, encrypted).await {
                                    GattWriteResult::Handled => {}
                                    GattWriteResult::CccdWrite => cccd_updated = true,
                                    GattWriteResult::HidControlPoint => control_point_write = true,
                                    GattWriteResult::Unhandled => {
                                        debug!("Write GATT Event to Unknown: {:?}", event.handle())
                                    }
                                }
                            }
                            #[cfg(not(feature = "host"))]
                            debug!("Write GATT Event to Unknown: {:?}", event.handle());
                        }

                        if control_point_write && profile_id == crate::state::current_profile() {
                            info!("Write GATT Event to Control Point: {:?}", event.handle());
                            // Forward an HID Control Point write to sleep management.
                            // HID Class spec opcodes for the HID Control Point characteristic:
                            //   - 0: HID_CTRL_SUSPEND
                            //   - 1: HID_CTRL_EXIT_SUSPEND
                            if data_len == 1 {
                                match data[0] {
                                    0 => request_sleep(),
                                    1 => report_activity(),
                                    _ => {}
                                }
                            }
                        }

                        if encrypted {
                            None
                        } else {
                            Some(AttErrorCode::INSUFFICIENT_ENCRYPTION)
                        }
                    }
                    GattEvent::Other(_) => None,
                    GattEvent::NotAllowed(_) => None,
                };

                // This step is also performed at drop(), but writing it explicitly is necessary
                // in order to ensure reply is sent.
                let result = if let Some(code) = result {
                    gatt_event.reject(code)
                } else {
                    gatt_event.accept()
                };
                // Update CCCD table after processing the event
                if cccd_updated {
                    // When macOS wakes up from sleep mode, it won't send EXIT SUSPEND command
                    // So we need to monitor the sleep state by using CCCD write event
                    report_activity();

                    if let Some(table) = server.get_client_att_table(conn.raw())
                        && let Ok(bytes) = heapless::Vec::from_slice(table.raw())
                    {
                        profile_manager.update_profile_cccd_table(profile_id, &conn.raw().peer_identity(), bytes);
                    }
                }
                match result {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("[gatt] error sending response: {:?}", e),
                }
            }
            GattConnectionEvent::PhyUpdated { tx_phy, rx_phy } => {
                info!("[gatt] PhyUpdated: {:?}, {:?}", tx_phy, rx_phy)
            }
            GattConnectionEvent::ConnectionParamsUpdated {
                conn_interval,
                peripheral_latency,
                supervision_timeout,
            } => {
                info!(
                    "[gatt] ConnectionParamsUpdated: {:?}ms, {:?}, {:?}ms",
                    conn_interval.as_millis(),
                    peripheral_latency,
                    supervision_timeout.as_millis()
                );
            }
            GattConnectionEvent::RequestConnectionParams(req) => info!(
                "[gatt] RequestConnectionParams: interval: ({:?}, {:?})ms, {:?}, {:?}ms",
                req.params().min_connection_interval.as_millis(),
                req.params().max_connection_interval.as_millis(),
                req.params().max_latency,
                req.params().supervision_timeout.as_millis(),
            ),
            GattConnectionEvent::DataLengthUpdated {
                max_tx_octets,
                max_tx_time,
                max_rx_octets,
                max_rx_time,
            } => {
                info!(
                    "[gatt] DataLengthUpdated: tx/rx octets: ({:?}, {:?}), tx/rx time: ({:?}, {:?})",
                    max_tx_octets, max_rx_octets, max_tx_time, max_rx_time
                );
            }
            GattConnectionEvent::FrameSpaceUpdated {
                frame_space,
                initiator,
                phys,
                spacing_types,
            } => {
                info!(
                    "[gatt] FrameSpaceUpdated: {:?}, {:?}, {:?}, {:?}",
                    frame_space, initiator, phys, spacing_types
                );
            }
            GattConnectionEvent::ConnectionRateChanged {
                conn_interval,
                subrate_factor,
                peripheral_latency,
                continuation_number,
                supervision_timeout,
            } => {
                info!(
                    "[gatt] ConnectionRateChanged: {:?}ms, {:?}, {:?}, {:?}, {:?}ms",
                    conn_interval.as_millis(),
                    subrate_factor,
                    peripheral_latency,
                    continuation_number,
                    supervision_timeout.as_millis()
                );
            }
            GattConnectionEvent::SubratingParamsUpdated {
                subrate_factor,
                peripheral_latency,
                continuation_number,
                supervision_timeout,
            } => {
                info!(
                    "[gatt] SubratingParamsUpdated: {:?}, {:?}, {:?}, {:?}ms",
                    subrate_factor,
                    peripheral_latency,
                    continuation_number,
                    supervision_timeout.as_millis()
                );
            }
            GattConnectionEvent::PassKeyDisplay(pass_key) => info!("[gatt] PassKeyDisplay: {:?}", pass_key),
            GattConnectionEvent::PassKeyConfirm(pass_key) => info!("[gatt] PassKeyConfirm: {:?}", pass_key),
            GattConnectionEvent::PassKeyInput => {
                #[cfg(feature = "passkey_entry")]
                if crate::PASSKEY_ENTRY_ENABLED {
                    info!("[gatt] PassKeyInput: entering passkey entry mode");
                    passkey_state.begin();
                } else {
                    warn!("[gatt] PassKeyInput: disabled in config, cancelling pairing, this shouldn't happen");
                    if let Err(e) = conn.raw().pass_key_cancel() {
                        error!("[gatt] pass_key_cancel error: {:?}", e);
                    }
                }
                #[cfg(not(feature = "passkey_entry"))]
                warn!("[gatt] PassKeyInput event, should not happen")
            }
            GattConnectionEvent::BondLost => warn!("[gatt] BondLost"),
            GattConnectionEvent::OobRequest => warn!("[gatt] OobRequest"),
        }
    }
    info!("[gatt] task finished");
}

/// Drop the link and wait for it to actually go down.
async fn disconnect(conn: &GattConnection<'_, '_, DefaultPacketPool>) {
    if !conn.raw().is_connected() {
        return;
    }
    conn.raw().disconnect();
    while !matches!(conn.next().await, GattConnectionEvent::Disconnected { .. }) {}
}

/// Set keyboard <-> host connection parameters
pub(crate) async fn set_conn_params<
    'a,
    'b,
    C: Controller + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    P: PacketPool,
>(
    stack: &Stack<'_, C, P>,
    conn: &GattConnection<'a, 'b, P>,
) {
    let requests = [
        // The first request is what Apple devices accept:
        // https://developer.apple.com/accessories/Accessory-Design-Guidelines.pdf
        (Duration::from_millis(15), 30, Duration::from_secs(6)),
        // The second request is for best performance
        (Duration::from_micros(7500), 60, Duration::from_secs(6)),
    ];

    for (interval, max_latency, supervision_timeout) in requests {
        // Wait 5 seconds before each request to avoid connection drop
        embassy_time::Timer::after_secs(5).await;

        update_conn_params(
            stack,
            conn.raw(),
            &RequestedConnParams {
                min_connection_interval: interval,
                max_connection_interval: interval,
                max_latency,
                supervision_timeout,
                ..Default::default()
            },
        )
        .await;
    }

    // Wait forever. This is because we want the conn params setting can be interrupted when the connection is lost.
    // So this task shouldn't quit after setting the conn params.
    core::future::pending::<()>().await;
}

/// Serve one host connection until its GATT Disconnected event.
async fn serve_keyboard_connection<
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    profile_id: u8,
    profile_manager: &ProfileManager<BOND_SLOTS>,
    config: &BleBatteryConfig,
    #[cfg(feature = "host")] host_service: Option<&crate::host::HostService<'_>>,
) {
    let mut ble_hid_server = BleHidServer::new(server, conn);
    let mut ble_battery_server = config.enabled.then(|| BleBatteryServer::new(server, conn));
    #[cfg(feature = "split")]
    let mut ble_peripheral_battery_server = crate::SPLIT_BATTERY_PERIPHERAL_IDS
        .first()
        .map(|_| BlePeripheralBatteryServer::new(server, conn));

    // `use_1m_phy` exists for legacy host adapters that cannot do 2M.
    // Always use 2M for the dongle link.
    #[cfg(feature = "dongle")]
    let dongle_link = profile_id == crate::ble::profile::DONGLE_PROFILE;
    #[cfg(not(feature = "dongle"))]
    let dongle_link = false;
    let host_phy = if !dongle_link && (!crate::BLE_USE_2M_PHY || cfg!(feature = "use_1m_phy")) {
        PhyKind::Le1M
    } else {
        PhyKind::Le2M
    };
    #[cfg(not(feature = "split"))]
    let battery_task = ble_battery_server.run();
    #[cfg(feature = "split")]
    let battery_task = embassy_futures::join::join(ble_battery_server.run(), ble_peripheral_battery_server.run());

    // PHY negotiation must not delay GATT disconnection or input routing.
    let connection_params = async {
        update_ble_phy(stack, conn.raw(), host_phy).await;
        set_conn_params(stack, conn).await;
    };

    // The host only reports its lock LEDs when they change, so a switch back to this link replays them.
    let led = Cell::new(LedIndicator::default());
    let writer_task = async {
        loop {
            wait_for_connection_status(|s| s.selected == profile_id).await;
            LED_SIGNAL.signal(led.get());
            while let Either::Second(report) = select(
                wait_for_connection_status(|s| s.selected != profile_id),
                BLE_REPORT_CHANNEL.receive(),
            )
            .await
            {
                if let Err(e) = ble_hid_server.write_report(&report).await {
                    error!("Failed to send report: {:?}", e);
                }
            }
            for report in [
                Report::KeyboardReport(Default::default()),
                Report::MouseReport(Default::default()),
                Report::MediaKeyboardReport(MediaKeyboardReport { usage_id: 0 }),
                Report::SystemControlReport(SystemControlReport { usage_id: 0 }),
                #[cfg(feature = "steno")]
                Report::StenoReport(Default::default()),
            ] {
                let _ = ble_hid_server.write_report(&report).await;
            }
        }
    };

    #[cfg(feature = "host")]
    let gatt_task = async {
        match host_service {
            Some(service) => {
                let host = HostConnection::new(server);
                let events = gatt_events_task(server, conn, profile_id, profile_manager, &led, Some(&host));
                select(events, host.run(conn, service)).await;
            }
            None => gatt_events_task(server, conn, profile_id, profile_manager, &led, None).await,
        }
    };
    #[cfg(not(feature = "host"))]
    let gatt_task = gatt_events_task(server, conn, profile_id, profile_manager, &led);

    // When dongle feature is enabled, send `DongleEvent` to the dongle.
    #[cfg(feature = "dongle")]
    let dongle_event_task = async {
        if !dongle_link {
            core::future::pending::<()>().await;
        }
        crate::dongle::event::run(server, conn).await;
    };
    #[cfg(not(feature = "dongle"))]
    let dongle_event_task = core::future::pending::<()>();

    select5(
        gatt_task,
        connection_params,
        battery_task,
        writer_task,
        dongle_event_task,
    )
    .await;
    disconnect(conn).await;
}

// Set the connection PHY.
pub(crate) async fn update_ble_phy<P: PacketPool>(
    stack: &Stack<'_, impl Controller + ControllerCmdAsync<LeSetPhy>, P>,
    conn: &Connection<'_, P>,
    phy: PhyKind,
) {
    // Retry 10 times
    for _ in 0..10 {
        match conn.set_phy(stack, phy).await {
            Err(BleHostError::BleHost(Error::Hci(error))) => {
                // A connection runs one link-layer control procedure at a time, and
                // a fresh one is still running its own.
                if error == HciError::CONTROLLER_BUSY || error == HciError::DIFFERENT_TRANSACTION_COLLISION {
                    info!("[update_ble_phy] controller busy, retrying: {:?}", error);
                    embassy_time::Timer::after_millis(100).await;
                    continue;
                }
                error!("[update_ble_phy] HCI error: {:?}", error);
            }
            Err(e) => {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                error!("[update_ble_phy] error: {:?}", e);
            }
            Ok(_) => {
                info!("[update_ble_phy] PHY updated");
            }
        }
        return;
    }
    warn!("[update_ble_phy] controller stayed busy, giving up");
}

/// Update the connection parameters.
///
/// Returns whether the request reached the controller, so callers that mirror
/// the parameters in their own state don't record params that never landed.
pub(crate) async fn update_conn_params<
    'a,
    'b,
    C: Controller + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    P: PacketPool,
>(
    stack: &Stack<'a, C, P>,
    conn: &Connection<'b, P>,
    params: &RequestedConnParams,
) -> bool {
    // Retry 10 times
    for _ in 0..10 {
        match conn.update_connection_params(stack, params).await {
            Err(BleHostError::BleHost(Error::Hci(error))) => {
                // A connection runs one link-layer control procedure at a time, and
                // a fresh one is still running its own.
                if error == HciError::CONTROLLER_BUSY || error == HciError::DIFFERENT_TRANSACTION_COLLISION {
                    info!("[update_conn_params] controller busy, retrying: {:?}", error);
                    embassy_time::Timer::after_millis(100).await;
                    continue;
                }
                error!("[update_conn_params] HCI error: {:?}", error);
                return false;
            }
            Err(e) => {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                error!("[update_conn_params] BLE host error: {:?}", e);
                return false;
            }
            Ok(_) => return true,
        }
    }
    warn!("[update_conn_params] controller stayed busy, giving up");
    false
}

#[cfg(test)]
mod tests {
    use std::sync::{Mutex, OnceLock};

    use embassy_futures::select::select3;
    use embassy_time::{Duration, MockDriver};
    use rmk_types::ble::{BleState, BleStatus};

    use super::fake_hci::FakeController;
    use crate::event::{Axis, AxisEvent, AxisValType, KeyboardEvent, PointingEvent, publish_event};
    use crate::state::current_ble_status;
    use crate::test_support::test_block_on as block_on;

    #[cfg(all(feature = "dongle", feature = "custom_message"))]
    #[test]
    fn custom_messages_survive_gatt_write_staging() {
        use postcard::experimental::max_size::MaxSize;

        use crate::custom_message::{CustomMessage, CustomMessageTarget};

        for target in [CustomMessageTarget::Central, CustomMessageTarget::Peripherals] {
            for len in [0, 30, 31, crate::CUSTOM_MESSAGE_MAX_SIZE]
                .into_iter()
                .filter(|len| *len <= crate::CUSTOM_MESSAGE_MAX_SIZE)
            {
                let payload = vec![0xA5; len];
                let message = CustomMessage::new(&payload, target).unwrap();
                let mut wire = [0; CustomMessage::POSTCARD_MAX_SIZE];
                let encoded = postcard::to_slice(&message, &mut wire).unwrap();
                let mut staged = [0; super::GATT_WRITE_BUFFER_SIZE];
                let copied = encoded.len().min(staged.len());
                staged[..copied].copy_from_slice(&encoded[..copied]);
                let decoded = postcard::from_bytes::<CustomMessage>(&staged[..copied])
                    .expect("a valid custom message must survive GATT write staging");
                assert_eq!(decoded.data.as_slice(), payload);
                assert_eq!(decoded.target, target);
            }
        }
    }

    fn ble_status_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Keep HCI setup pending to exercise the keyboard's outer deadline.
    fn with_stalled_keyboard(bonds: &[super::ProfileInfo], test: impl core::future::Future<Output = ()>) {
        with_keyboard(FakeController::stalled(), bonds, test);
    }

    fn with_keyboard(hci: FakeController, bonds: &[super::ProfileInfo], test: impl core::future::Future<Output = ()>) {
        use super::{BOND_SLOTS, CONNECTIONS_MAX, DefaultPacketPool, HostResources, L2CAP_CHANNELS_MAX};
        use crate::config::{BleBatteryConfig, DeviceConfig};
        use crate::core_traits::Runnable;
        use crate::storage::{Storage, StorageKey, StorageValue, async_flash_wrapper};
        use crate::test_support::InMemoryFlash;

        let _guard = ble_status_test_lock().lock().unwrap();
        crate::ble::rng::seed([7; 32]);
        crate::test_support::clear_flash_channel();
        crate::test_support::reset_connection_status();
        crate::channel::BLE_PROFILE_CHANNEL.clear();
        let mut resources =
            HostResources::<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, 1, BOND_SLOTS>::new();
        let stack = trouble_host::new(hci, &mut resources).build();
        block_on(select3(
            super::run_ble_keyboard(
                &stack,
                [1, 2, 3, 4, 5, 0xC6],
                &DeviceConfig::default(),
                &BleBatteryConfig::disabled(),
                #[cfg(feature = "host")]
                None,
            ),
            async {
                let flash = InMemoryFlash::<16_384, 4_096, 1>::new();
                let mut storage = Storage::new(async_flash_wrapper(flash), &Default::default()).await;
                for bond in bonds {
                    storage
                        .flash
                        .store_item(
                            &mut storage.buffer,
                            &StorageKey::BondInfo(bond.slot_num),
                            &StorageValue::BondInfo(bond.clone()),
                        )
                        .await
                        .unwrap();
                }
                storage.run().await;
            },
            test,
        ));
    }

    #[test]
    fn pairing_current_profile_survives_key_release_and_restores_bond_on_timeout() {
        use crate::ble::profile::{BleProfileAction, ProfileInfo};
        use crate::channel::BLE_PROFILE_CHANNEL;
        use crate::state::wait_for_connection_status;
        use crate::storage::{StorageKey, StorageValue, read};

        let bond = ProfileInfo {
            cccd_table: heapless::Vec::from_slice(&[1, 2]).unwrap(),
            ..Default::default()
        };
        with_stalled_keyboard(core::slice::from_ref(&bond), async {
            wait_for_connection_status(|s| s.selected_ble().state == BleState::Advertising).await;
            assert!(current_ble_status().bonded);

            // Holding the profile key of the selected profile.
            BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(0)).await;
            BLE_PROFILE_CHANNEL.send(BleProfileAction::Disconnect(0)).await;
            embassy_time::with_timeout(
                Duration::from_millis(20),
                wait_for_connection_status(|s| s.selected == 0 && !s.selected_ble().bonded),
            )
            .await
            .expect("holding the current profile key must start re-pairing");

            // Releasing the profile key sends a switch to the same slot.
            BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(0)).await;
            embassy_time::Timer::after_millis(1).await;
            assert!(!current_ble_status().bonded);

            MockDriver::get().advance(Duration::from_secs(300));
            let expired = wait_for_connection_status(|s| s.selected_ble().state == BleState::Inactive).await;
            assert!(expired.selected_ble().bonded);
            let Some(StorageValue::BondInfo(saved)) = read(StorageKey::BondInfo(0)).await.unwrap() else {
                panic!("re-pairing must preserve the stored bond");
            };
            assert_eq!(saved, bond);
        });
    }

    #[test]
    fn cycling_uses_available_host_profiles() {
        use crate::ble::profile::BleProfileAction::{ClearBond, Next, Previous};
        use crate::channel::BLE_PROFILE_CHANNEL;
        use crate::state::{current_profile, wait_for_connection_status};

        with_stalled_keyboard(&[], async {
            wait_for_connection_status(|s| s.selected_ble().state == BleState::Advertising).await;
            for (action, expected) in [(Previous, crate::NUM_BLE_PROFILE.saturating_sub(1) as u8), (Next, 0)] {
                BLE_PROFILE_CHANNEL.send(action).await;
                // The one-slot queue makes this wait until the preceding action was consumed.
                BLE_PROFILE_CHANNEL.send(ClearBond(Some(0))).await;
                assert_eq!(current_profile(), expected);
            }
        });
    }

    #[test]
    fn forgetting_the_selected_profile_advertises_for_a_new_host() {
        use crate::ble::profile::{BleProfileAction, ProfileManager};
        use crate::channel::BLE_PROFILE_CHANNEL;
        use crate::storage::{StorageKey, StorageValue, read};

        let hci = FakeController::default();
        with_keyboard(hci.clone(), &[bond(0, HOST_A)], async {
            advertising(&hci, 0).await;
            BLE_PROFILE_CHANNEL.send(BleProfileAction::ClearBond(None)).await;
            advertising(&hci, 1).await;
            assert!(!current_ble_status().bonded);
            embassy_time::Timer::after_millis(10).await;
            let Some(StorageValue::BondInfo(saved)) = read(StorageKey::BondInfo(0)).await.unwrap() else {
                panic!("the slot must keep a removal record");
            };
            assert!(saved.removed);
            let mut restored = ProfileManager::<{ super::BOND_SLOTS }>::new();
            restored.load_bonded_devices().await;
            assert!(restored.bond_info(0).is_none());
        });
    }

    #[test]
    fn advertising_follows_slot_capacity_and_wakes_on_input() {
        use super::BOND_SLOTS;
        use crate::ble::profile::BleProfileAction;
        use crate::channel::BLE_PROFILE_CHANNEL;
        use crate::state::wait_for_connection_status;

        with_stalled_keyboard(&[], async {
            wait_for_connection_status(|s| s.selected_ble().state == BleState::Advertising).await;
            for wake in 0..3 {
                MockDriver::get().advance(Duration::from_secs(300));
                wait_for_connection_status(|s| s.selected_ble().state == BleState::Inactive).await;
                match wake {
                    0 => publish_event(KeyboardEvent::key(0, 0, true)),
                    1 => publish_event(PointingEvent {
                        device_id: 0,
                        axes: [
                            AxisEvent {
                                typ: AxisValType::Rel,
                                axis: Axis::X,
                                value: 1,
                            },
                            AxisEvent {
                                typ: AxisValType::Rel,
                                axis: Axis::Y,
                                value: 0,
                            },
                            AxisEvent {
                                typ: AxisValType::Rel,
                                axis: Axis::Z,
                                value: 0,
                            },
                        ],
                    }),
                    _ if BOND_SLOTS > 1 => BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(1)).await,
                    _ => {
                        BLE_PROFILE_CHANNEL.send(BleProfileAction::ClearBond(Some(0))).await;
                        BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(0)).await;
                    }
                }
                let resumed = wait_for_connection_status(|s| s.selected_ble().state == BleState::Advertising).await;
                assert_eq!(resumed.selected, u8::from(wake == 2 && BOND_SLOTS > 1));
            }
        });
    }

    #[test]
    fn input_wakes_advertising_when_same_profile_switch_is_queued() {
        use crate::ble::profile::BleProfileAction;
        use crate::channel::BLE_PROFILE_CHANNEL;
        use crate::state::{current_profile, wait_for_connection_status};

        with_stalled_keyboard(&[], async {
            wait_for_connection_status(|s| s.selected_ble().state == BleState::Advertising).await;
            MockDriver::get().advance(Duration::from_secs(300));
            wait_for_connection_status(|s| s.selected_ble().state == BleState::Inactive).await;
            publish_event(KeyboardEvent::key(0, 0, true));
            BLE_PROFILE_CHANNEL
                .try_send(BleProfileAction::Switch(current_profile()))
                .unwrap();
            embassy_time::with_timeout(
                Duration::from_millis(10),
                wait_for_connection_status(|s| s.selected_ble().state == BleState::Advertising),
            )
            .await
            .expect("queued profile action must not discard the input that wakes advertising");
        });
    }

    #[test]
    fn restarted_advertising_keeps_its_window() {
        use crate::ble::profile::BleProfileAction;
        use crate::channel::BLE_PROFILE_CHANNEL;
        use crate::state::{current_profile, wait_for_connection_status};

        with_stalled_keyboard(&[], async {
            wait_for_connection_status(|s| s.selected_ble().state == BleState::Advertising).await;
            MockDriver::get().advance(Duration::from_secs(150));
            // A same-profile switch restarts advertising, as a rejected connection does.
            BLE_PROFILE_CHANNEL
                .send(BleProfileAction::Switch(current_profile()))
                .await;
            embassy_time::Timer::after_millis(1).await;
            MockDriver::get().advance(Duration::from_secs(150));
            embassy_time::with_timeout(
                Duration::from_millis(10),
                wait_for_connection_status(|s| s.selected_ble().state == BleState::Inactive),
            )
            .await
            .expect("restarting advertising must not extend its window");
        });
    }

    const HOST_A: [u8; 6] = [0xA1, 0, 0, 0, 0, 0xC1];
    const HOST_B: [u8; 6] = [0xB2, 0, 0, 0, 0, 0xC2];

    /// A stored bond with the host at random address `peer`.
    fn bond(slot: u8, peer: [u8; 6]) -> super::ProfileInfo {
        let mut bond = super::ProfileInfo {
            slot_num: slot,
            ..Default::default()
        };
        bond.info.identity.addr = trouble_host::prelude::Address::random(peer);
        bond.info.security_level = trouble_host::prelude::SecurityLevel::Encrypted;
        bond
    }

    /// Wait until the connection status satisfies `done`.
    async fn until(done: impl Fn(&rmk_types::connection::ConnectionStatus) -> bool) {
        while !done(&crate::state::current_connection_status()) {
            embassy_time::Timer::after_millis(1).await;
        }
    }

    /// Wait until advertising has started more than `starts` times.
    async fn advertising(hci: &FakeController, starts: usize) {
        while hci.advertising_starts() <= starts {
            embassy_time::Timer::after_millis(1).await;
        }
    }

    /// Wait until the keyboard serves a link; serving starts with a PHY update.
    async fn served(hci: &FakeController) {
        while hci.sent::<bt_hci::cmd::le::LeSetPhy>().is_empty() {
            embassy_time::Timer::after_millis(1).await;
        }
    }

    /// Connect the bonded `peer` on `handle` and wait until it carries input.
    async fn reconnect(hci: &FakeController, handle: u16, peer: [u8; 6]) {
        advertising(hci, 0).await;
        hci.connect(handle, peer);
        hci.encrypt(handle);
        until(|s| s.selected_ble().state == BleState::Connected).await;
    }

    #[test]
    fn a_bonded_host_reconnects_with_its_stored_bond() {
        let hci = FakeController::default();
        with_keyboard(hci.clone(), &[bond(0, HOST_A)], reconnect(&hci, 1, HOST_A));
    }

    #[test]
    fn switching_away_keeps_an_encrypted_link_only_with_a_spare_link() {
        use crate::ble::profile::BleProfileAction;
        use crate::channel::BLE_PROFILE_CHANNEL;

        let hci = FakeController::default();
        with_keyboard(hci.clone(), &[bond(0, HOST_A), bond(1, HOST_B)], async {
            reconnect(&hci, 1, HOST_A).await;
            BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(1)).await;
            // Profile 1 advertises once profile 0's link is kept or gone.
            advertising(&hci, 1).await;
            let kept = crate::NUM_HOST_LINKS > 1;
            let status = crate::state::current_connection_status();
            assert_eq!(status.ble[0].state == BleState::Connected, kept);
            assert_eq!(hci.sent::<bt_hci::cmd::link_control::Disconnect>().is_empty(), kept);
        });
    }

    #[test]
    fn a_switch_drops_the_link_it_leaves_when_every_link_is_taken() {
        use crate::ble::profile::BleProfileAction;
        use crate::channel::BLE_PROFILE_CHANNEL;

        const LINKS: usize = crate::NUM_HOST_LINKS;
        // Needs a profile without a link once every link is taken.
        if LINKS < 2 || super::BOND_SLOTS <= LINKS {
            return;
        }
        let peer = |slot: u8| [0xA0 + slot, 0, 0, 0, 0, 0xC0];
        let bonds: Vec<_> = (0..LINKS as u8).map(|slot| bond(slot, peer(slot))).collect();
        let hci = FakeController::default();
        with_keyboard(hci.clone(), &bonds, async {
            for slot in 0..LINKS as u8 {
                if slot > 0 {
                    BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(slot)).await;
                }
                advertising(&hci, slot.into()).await;
                hci.connect(u16::from(slot) + 1, peer(slot));
                hci.encrypt(u16::from(slot) + 1);
                until(|s| s.ble[usize::from(slot)].state == BleState::Connected).await;
            }
            BLE_PROFILE_CHANNEL.send(BleProfileAction::Switch(LINKS as u8)).await;
            embassy_time::with_timeout(Duration::from_millis(100), advertising(&hci, LINKS))
                .await
                .expect("the new profile must get the slot of the link it replaces");
            assert_eq!(hci.sent::<bt_hci::cmd::link_control::Disconnect>().len(), 1);
            let status = crate::state::current_connection_status();
            assert_eq!(status.ble[LINKS - 1].state, BleState::Inactive);
            assert!(
                status.ble[..LINKS - 1]
                    .iter()
                    .all(|ble| ble.state == BleState::Connected)
            );
        });
    }

    #[test]
    fn disconnecting_the_selected_link_waits_for_a_new_host() {
        use crate::ble::profile::BleProfileAction;
        use crate::channel::BLE_PROFILE_CHANNEL;

        let hci = FakeController::default();
        with_keyboard(hci.clone(), &[bond(0, HOST_A)], async {
            reconnect(&hci, 1, HOST_A).await;
            // Past the first advertising period, so disconnecting must open a new one.
            MockDriver::get().advance(Duration::from_secs(400));
            BLE_PROFILE_CHANNEL.send(BleProfileAction::Disconnect(0)).await;
            advertising(&hci, 1).await;
            assert_eq!(hci.sent::<bt_hci::cmd::link_control::Disconnect>().len(), 1);
            embassy_time::Timer::after_millis(10).await;
            // The old bond is hidden, so the old host cannot take the profile back.
            assert_eq!(
                current_ble_status(),
                BleStatus {
                    state: BleState::Advertising,
                    bonded: false
                }
            );
        });
    }

    #[test]
    fn a_new_host_can_connect_to_a_bonded_profile_while_its_host_is_away() {
        let hci = FakeController::default();
        with_keyboard(hci.clone(), &[bond(0, HOST_A)], async {
            advertising(&hci, 0).await;
            hci.connect(1, HOST_B);
            served(&hci).await;
            assert!(hci.sent::<bt_hci::cmd::link_control::Disconnect>().is_empty());
            // The old bond stays until the new host pairs.
            assert!(current_ble_status().bonded);
        });
    }

    #[test]
    fn losing_an_encrypted_link_opens_a_new_advertising_period() {
        let hci = FakeController::default();
        with_keyboard(hci.clone(), &[bond(0, HOST_A)], async {
            reconnect(&hci, 1, HOST_A).await;
            MockDriver::get().advance(Duration::from_secs(200));
            hci.drop_link(1);
            advertising(&hci, 1).await;
            // Past the end of the first period, the new one still advertises.
            MockDriver::get().advance(Duration::from_secs(150));
            embassy_time::Timer::after_millis(1).await;
            assert_eq!(current_ble_status().state, BleState::Advertising);
        });
    }

    #[test]
    fn a_link_that_never_encrypts_keeps_the_advertising_period() {
        let hci = FakeController::default();
        with_keyboard(hci.clone(), &[], async {
            advertising(&hci, 0).await;
            hci.connect(1, HOST_A);
            served(&hci).await;
            MockDriver::get().advance(Duration::from_secs(200));
            hci.drop_link(1);
            advertising(&hci, 1).await;
            MockDriver::get().advance(Duration::from_secs(110));
            embassy_time::with_timeout(
                Duration::from_millis(10),
                until(|s| s.selected_ble().state == BleState::Inactive),
            )
            .await
            .expect("an unencrypted link must not extend advertising");
        });
    }

    #[test]
    fn bonded_advertising_starts_fast_then_slows_within_its_period() {
        let hci = FakeController::default();
        with_keyboard(hci.clone(), &[bond(0, HOST_A)], async {
            advertising(&hci, 0).await;
            MockDriver::get().advance(Duration::from_secs(31));
            advertising(&hci, 1).await;
            // In 0.625 ms units: 50 ms, then 200 ms.
            assert_eq!(hci.advertising_intervals(), [80, 320]);
            MockDriver::get().advance(Duration::from_secs(270));
            embassy_time::with_timeout(
                Duration::from_millis(10),
                until(|s| s.selected_ble().state == BleState::Inactive),
            )
            .await
            .expect("slow advertising must end with its period");
        });
    }
}
