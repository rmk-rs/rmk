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
use embassy_futures::select::{Either, Either4, select, select4, select6};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer};
use rmk_types::ble::BleState;
use rmk_types::connection::ConnectionType;
use rmk_types::led_indicator::LedIndicator;
use trouble_host::prelude::*;
use usbd_hid::descriptor::{MediaKeyboardReport, SystemControlReport};

use crate::ble::adv::{Adv, advertise};
use crate::ble::battery_service::BleBatteryServer;
#[cfg(feature = "split")]
use crate::ble::battery_service::BlePeripheralBatteryServer;
use crate::ble::ble_server::{BleHidServer, Server};
use crate::ble::device_info::{PnPID, VidSource};
#[cfg(feature = "host")]
use crate::ble::host::{BleConfig, HOST_WRITE_BUFFER_SIZE, HostGattHandler, HostWriteOutcome};
use crate::ble::led::BleLedReader;
#[cfg(feature = "passkey_entry")]
use crate::ble::passkey::{PasskeyInputState, next_gatt_event};
#[cfg(feature = "dongle")]
use crate::ble::profile::DONGLE_PROFILE;
use crate::ble::profile::{BOND_SLOTS, BleProfileAction, ProfileInfo, ProfileManager};
use crate::ble::sleep::{report_activity, request_sleep};
use crate::channel::{BLE_REPORT_CHANNEL, LED_SIGNAL};
use crate::config::{BleBatteryConfig, DeviceConfig, RmkConfig};
use crate::core_traits::Runnable;
use crate::event::SubscribableEvent;
use crate::hid::{HidWriterTrait, Report, run_led_reader};
#[cfg(feature = "split")]
use crate::split::PeripheralMatrixConfig;
#[cfg(feature = "split")]
use crate::split::ble::central::{run_peripheral_session, scan_and_connect_peripherals};
use crate::state::{CONNECTION_STATUS, set_ble_state};

pub(crate) mod adv;
pub(crate) mod battery_service;
pub(crate) mod ble_server;
pub(crate) mod device_info;
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
pub(crate) mod sleep;

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

/// Host and dongle connections: one per profile on extended-HCI controllers.
#[doc(hidden)]
pub const NUM_HOST_LINKS: usize = if cfg!(feature = "ext_adv") {
    crate::ble::profile::BOND_SLOTS
} else {
    1
};
const _: () = core::assert!(
    BOND_SLOTS > 0,
    "BLE requires at least one bond slot; set ble_profiles_num > 0 or enable dongle"
);
const _: () = core::assert!(NUM_HOST_LINKS <= u8::MAX as usize, "BLE host capacity must fit in u8");

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

        let peripheral_sessions =
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
            peripheral_sessions,
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

    if crate::ble::passkey::passkey_entry_enabled() {
        stack.set_io_capabilities(IoCapabilities::KeyboardOnly);
    }
    let mut profile_manager: ProfileManager<_, _, BOND_SLOTS> = ProfileManager::new(stack);
    // Load the bonded devices from storage
    profile_manager.load_bonded_devices().await;
    profile_manager.update_stack_bonds();

    #[cfg(feature = "host")]
    let host_config = host_service.map(BleConfig::new);

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    // Hand each accepted connection and its profile to an available worker.
    let incoming_conn: Channel<NoopRawMutex, (GattConnection<'_, '_, DefaultPacketPool>, u8), 1> = Channel::new();
    let conn_completed: Channel<NoopRawMutex, u8, NUM_HOST_LINKS> = Channel::new();
    let conn_workers = join_array(core::array::from_fn::<_, NUM_HOST_LINKS, _>(|_| async {
        loop {
            let (conn, profile) = incoming_conn.receive().await;
            serve_keyboard_connection(
                &server,
                &conn,
                stack,
                profile,
                &profile_manager,
                config,
                #[cfg(feature = "host")]
                host_config.as_ref(),
            )
            .await;
            drop(conn);
            conn_completed.send(profile).await;
        }
    }));

    let connection_loop = async {
        let mut background_connections = NUM_HOST_LINKS > 1
            && !matches!(
                crate::storage::read(crate::storage::StorageKey::BleBackgroundConnections).await,
                Ok(Some(crate::storage::StorageValue::BleBackgroundConnections(false)))
            );
        let mut links: [Option<Connection<'_, DefaultPacketPool>>; BOND_SLOTS] = core::array::from_fn(|_| None);
        let mut deadline = Instant::now() + Duration::from_secs(300);
        let mut wake_subscribers = None;
        Timer::after_millis(10).await;
        loop {
            let now = Instant::now();
            let profile = crate::state::current_profile();
            let local_address =
                cfg!(feature = "ext_adv").then(|| BdAddr::new(crate::ble::profile::profile_address(address, profile)));
            if now < deadline {
                wake_subscribers = None;
            } else if wake_subscribers.is_none() {
                if let Some(pairing) = profile_manager.repairing.get() {
                    disconnect_profile(pairing, &mut links, &conn_completed).await;
                    profile_manager.set_pairing(None);
                }
                wake_subscribers = Some((
                    crate::event::KeyboardEvent::subscriber(),
                    crate::event::PointingEvent::subscriber(),
                ));
                if links[profile as usize].is_none() {
                    set_ble_state(BleState::Inactive);
                    request_sleep();
                }
            }
            let can_advertise = now < deadline
                && links[profile as usize].is_none()
                && (background_connections || links.iter().all(Option::is_none));
            if can_advertise {
                set_ble_state(BleState::Advertising);
            }
            let accept = async {
                if !can_advertise {
                    return core::future::pending().await;
                }
                let adv = match profile_manager.bond_info(profile) {
                    #[cfg(feature = "dongle")]
                    Some(bond) if profile == DONGLE_PROFILE => Adv::Directed(bond.info.identity.addr),
                    #[cfg(feature = "dongle")]
                    None if profile == DONGLE_PROFILE => Adv::DongleSeeking,
                    Some(_) => Adv::HostReconnect,
                    None => Adv::Host { name: product_name },
                };
                let timeout = deadline.saturating_duration_since(now);
                advertise(&mut peripheral, &server.server, adv, local_address, timeout).await
            };
            let activity = async {
                if let Some((keys, pointing)) = wake_subscribers.as_mut() {
                    let _ = select(keys.next_message_pure(), pointing.next_message_pure()).await;
                    true
                } else {
                    Timer::at(deadline).await;
                    false
                }
            };
            match select4(
                accept,
                crate::channel::BLE_PROFILE_CHANNEL.receive(),
                conn_completed.receive(),
                activity,
            )
            .await
            {
                Either4::First(Ok(gatt_conn)) => {
                    // A replaced advertisement can still deliver a late connection.
                    #[cfg(feature = "ext_adv")]
                    if gatt_conn.raw().local_address() != local_address {
                        warn!("[ble] late connection belongs to another profile");
                        disconnect(&gatt_conn).await;
                        continue;
                    }
                    let bond_info = profile_manager.bond_info(profile);
                    if let Some(bond) = &bond_info
                        && !bond.info.identity.match_identity(&gatt_conn.raw().peer_identity())
                    {
                        warn!("[ble] peer does not match its profile");
                        disconnect(&gatt_conn).await;
                        continue;
                    }
                    #[cfg(feature = "dongle")]
                    if profile != DONGLE_PROFILE && profile_manager.is_bonded_dongle(&gatt_conn.raw().peer_identity()) {
                        warn!("[ble] dongle connected on a host profile");
                        disconnect(&gatt_conn).await;
                        continue;
                    }
                    if let Err(e) = gatt_conn.raw().set_bondable(bond_info.is_none()) {
                        error!("Set bondable error: {:?}", e);
                        disconnect(&gatt_conn).await;
                        continue;
                    }
                    links[profile as usize] = Some(gatt_conn.raw().clone());
                    incoming_conn.send((gatt_conn, profile)).await;
                }
                Either4::First(Err(BleHostError::BleHost(Error::Timeout))) => {
                    deadline = Instant::now();
                }
                Either4::First(Err(e)) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Advertise error: {:?}", e);
                    Timer::after_millis(200).await;
                }
                Either4::Second(action) => {
                    let current = crate::state::current_profile();
                    let clear = matches!(action, BleProfileAction::ClearBond | BleProfileAction::ClearSlot(_));
                    let target = match action {
                        BleProfileAction::SetBackgroundConnections(enabled) => {
                            background_connections = enabled;
                            if !enabled {
                                for (profile, conn) in links.iter().enumerate() {
                                    if profile != current as usize
                                        && let Some(conn) = conn
                                    {
                                        conn.disconnect();
                                    }
                                }
                            }
                            if crate::storage::store(crate::storage::StorageItem::BleBackgroundConnections(enabled))
                                .await
                                .is_err()
                            {
                                error!("Failed to save background connection setting");
                            }
                            continue;
                        }
                        BleProfileAction::Switch(n) | BleProfileAction::Pair(n) | BleProfileAction::ClearSlot(n) => n,
                        BleProfileAction::ClearBond => current,
                        BleProfileAction::Previous | BleProfileAction::Next if crate::NUM_BLE_PROFILE == 0 => continue,
                        BleProfileAction::Previous if current == 0 => crate::NUM_BLE_PROFILE.saturating_sub(1) as u8,
                        BleProfileAction::Previous => current - 1,
                        BleProfileAction::Next => {
                            let next = current.saturating_add(1);
                            if usize::from(next) >= crate::NUM_BLE_PROFILE {
                                0
                            } else {
                                next
                            }
                        }
                    };
                    let pair =
                        matches!(action, BleProfileAction::Pair(_)) && profile_manager.bond_info(target).is_some();
                    if !clear && !pair && target == current {
                        continue;
                    }
                    report_activity();
                    deadline = Instant::now() + Duration::from_secs(300);
                    if !clear && target != current {
                        if let Some(pairing) = profile_manager.repairing.get() {
                            disconnect_profile(pairing, &mut links, &conn_completed).await;
                            profile_manager.set_pairing(None);
                        }
                        if !background_connections && let Some(conn) = &links[current as usize] {
                            conn.disconnect();
                        }
                        let state = if links[target as usize]
                            .as_ref()
                            .is_some_and(|conn| conn.security_level().is_ok_and(|level| level.encrypted()))
                        {
                            BleState::Connected
                        } else {
                            BleState::Inactive
                        };
                        profile_manager.switch_profile(target, state);
                    }
                    if clear || pair {
                        disconnect_profile(target, &mut links, &conn_completed).await;
                        if clear {
                            profile_manager.clear_bond(target);
                        } else {
                            profile_manager.set_pairing(Some(target));
                        }
                    }
                }
                Either4::Third(profile) => {
                    links[profile as usize].take();
                    if profile == crate::state::current_profile() {
                        set_ble_state(BleState::Inactive);
                        deadline = Instant::now() + Duration::from_secs(300);
                    }
                }
                Either4::Fourth(true) => {
                    report_activity();
                    deadline = Instant::now() + Duration::from_secs(300);
                }
                Either4::Fourth(false) => {}
            }
        }
    };

    #[cfg(feature = "split")]
    let event_handler = crate::split::ble::central::ScanHandler;
    #[cfg(not(feature = "split"))]
    let event_handler = NoopHandler;

    // The sleep manager lives here because this is the single always-present
    // BLE task: split or not, connected or not, it keeps running, so the
    // sleep state can never get stuck.
    let mut ble_led_reader = BleLedReader;
    join5(
        ble_task(runner, &event_handler),
        connection_loop,
        conn_workers,
        run_led_reader(&mut ble_led_reader, ConnectionType::Ble),
        embassy_futures::join::join(sleep::run_sleep_manager(), profile_manager.run()),
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
async fn gatt_events_task<C: Controller + ControllerCmdAsync<LeSetPhy>>(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    profile: u8,
    profiles: &ProfileManager<'_, '_, C, DefaultPacketPool, BOND_SLOTS>,
    #[cfg(feature = "host")] host: &HostGattHandler<'_, '_>,
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
            GattConnectionEvent::PairingComplete { security_level, bond } => {
                #[cfg(feature = "passkey_entry")]
                passkey_state.clear();
                info!("[gatt] pairing complete: {:?}", security_level);
                if let Some(bond_info) = bond {
                    let cccd_table = server
                        .get_client_att_table(conn.raw())
                        .and_then(|t| heapless::Vec::from_slice(t.raw()).ok())
                        .unwrap_or_default();
                    profiles.set_bond(ProfileInfo {
                        slot_num: profile,
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
            }
            GattConnectionEvent::Encrypted { security_level, .. } => {
                info!("[gatt] encrypted: {:?}", security_level);
                if profile == crate::state::current_profile() {
                    set_ble_state(BleState::Connected);
                }
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
                                if profile == crate::state::current_profile() {
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
                            match host.handle_write(event.handle(), data, encrypted).await {
                                HostWriteOutcome::Handled => {}
                                HostWriteOutcome::CccdUpdated => cccd_updated = true,
                                HostWriteOutcome::ControlPoint => control_point_write = true,
                                HostWriteOutcome::Unhandled => {
                                    debug!("Write GATT Event to Unknown: {:?}", event.handle())
                                }
                            }
                            #[cfg(not(feature = "host"))]
                            debug!("Write GATT Event to Unknown: {:?}", event.handle());
                        }

                        if control_point_write && profile == crate::state::current_profile() {
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
                        profiles.update_profile_cccd_table(profile, bytes);
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
            GattConnectionEvent::BondLost => {
                warn!("[gatt] BondLost");
                if profiles.bond_info(profile).is_some() {
                    conn.raw().disconnect();
                }
            }
            GattConnectionEvent::OobRequest => warn!("[gatt] OobRequest"),
        }
    }
    info!("[gatt] task finished");
}

/// Reclaim the slot only after its worker has consumed Disconnected.
async fn disconnect_profile(
    profile: u8,
    links: &mut [Option<Connection<'_, DefaultPacketPool>>; BOND_SLOTS],
    conn_completed: &Channel<NoopRawMutex, u8, NUM_HOST_LINKS>,
) {
    if let Some(conn) = &links[profile as usize] {
        conn.disconnect();
    }
    while links[profile as usize].is_some() {
        let ended = conn_completed.receive().await;
        links[ended as usize].take();
        if ended == crate::state::current_profile() {
            set_ble_state(BleState::Inactive);
        }
    }
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
    profile: u8,
    profiles: &ProfileManager<'_, '_, C, DefaultPacketPool, BOND_SLOTS>,
    config: &BleBatteryConfig,
    #[cfg(feature = "host")] host_config: Option<&BleConfig<'_>>,
) {
    let mut ble_hid_server = BleHidServer::new(server, conn);
    let mut status = CONNECTION_STATUS
        .receiver()
        .expect("connection status subscriber capacity exceeded");
    #[cfg(feature = "host")]
    let host = HostGattHandler::new(server, profile, host_config);
    let mut ble_battery_server = config.enabled.then(|| BleBatteryServer::new(server, conn));
    #[cfg(feature = "split")]
    let mut ble_peripheral_battery_server = crate::SPLIT_BATTERY_PERIPHERAL_IDS
        .first()
        .map(|_| BlePeripheralBatteryServer::new(server, conn));

    // CCCD lookup uses cached bond info to avoid a cancellable flash read while
    // this future is racing other arms of an outer `select`.
    if let Some(bond_info) = profiles.bond_info(profile)
        && bond_info.info.identity.match_identity(&conn.raw().peer_identity())
    {
        info!("Loading CCCD table: {:?}", bond_info.cccd_table);
        match ClientAttTableView::try_from_raw(&bond_info.cccd_table) {
            Ok(view) => server.set_client_att_table(conn.raw(), &view),
            Err(e) => warn!("Invalid stored CCCD table: {:?}", e),
        }
    }

    // `use_1m_phy` exists for legacy host adapters that cannot do 2M.
    // Always use 2M for the dongle link.
    #[cfg(feature = "dongle")]
    let dongle_link = profile == crate::ble::profile::DONGLE_PROFILE;
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

    let writer_task = async {
        loop {
            status.get_and(|s| s.ble.profile == profile).await;
            while let Either::Second(report) = select(
                status.get_and(|s| s.ble.profile != profile),
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
    let host_task = host.run(conn);
    #[cfg(not(feature = "host"))]
    let host_task = core::future::pending::<()>();

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

    select6(
        gatt_events_task(
            server,
            conn,
            profile,
            profiles,
            #[cfg(feature = "host")]
            &host,
        ),
        connection_params,
        battery_task,
        writer_task,
        host_task,
        dongle_event_task,
    )
    .await;
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

    use crate::event::{Axis, AxisEvent, AxisValType, KeyboardEvent, PointingEvent, publish_event};
    use crate::state::{current_ble_status, set_ble_state, set_ble_status};
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

    #[test]
    fn set_ble_state_preserves_current_profile() {
        let _guard = ble_status_test_lock().lock().unwrap();

        set_ble_status(BleStatus {
            profile: 2,
            state: BleState::Inactive,
            bonded: true,
        });
        set_ble_state(BleState::Advertising);

        assert_eq!(
            current_ble_status(),
            BleStatus {
                profile: 2,
                state: BleState::Advertising,
                bonded: true,
            }
        );
    }

    #[test]
    fn selecting_an_unconnected_profile_sets_inactive() {
        let _guard = ble_status_test_lock().lock().unwrap();

        set_ble_status(BleStatus {
            profile: 1,
            state: BleState::Connected,
            bonded: false,
        });
        set_ble_status(BleStatus {
            profile: 3,
            state: BleState::Inactive,
            bonded: true,
        });

        assert_eq!(
            current_ble_status(),
            BleStatus {
                profile: 3,
                state: BleState::Inactive,
                bonded: true,
            }
        );
    }

    fn with_stalled_keyboard(test: impl core::future::Future<Output = ()>) {
        use bt_hci::controller::ExternalController;
        use bt_hci::transport::{PacketToController, PacketToHost, Transport};

        use super::{BOND_SLOTS, CONNECTIONS_MAX, DefaultPacketPool, HostResources, L2CAP_CHANNELS_MAX};
        use crate::config::{BleBatteryConfig, DeviceConfig};

        // Keep HCI setup pending to exercise the keyboard's outer deadline.
        struct StalledTransport;
        impl embedded_io_async::ErrorType for StalledTransport {
            type Error = bt_hci::ReadHciError<core::convert::Infallible>;
        }
        impl Transport for StalledTransport {
            async fn read<'a, P: PacketToHost<'a>>(&self, _: &'a mut [u8]) -> Result<P, Self::Error> {
                core::future::pending().await
            }

            async fn write<P: PacketToController>(&self, _: &P) -> Result<(), Self::Error> {
                core::future::pending().await
            }
        }

        let _guard = ble_status_test_lock().lock().unwrap();
        crate::ble::rng::seed([7; 32]);
        crate::test_support::clear_flash_channel();
        crate::test_support::set_connection_status(Default::default());
        crate::channel::BLE_PROFILE_CHANNEL.clear();
        let mut resources =
            HostResources::<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX, 1, BOND_SLOTS>::new();
        let stack = trouble_host::new(ExternalController::<_, 1>::new(StalledTransport), &mut resources).build();
        block_on(select3(
            super::run_ble_keyboard(
                &stack,
                [1, 2, 3, 4, 5, 0xC6],
                &DeviceConfig::default(),
                &BleBatteryConfig::disabled(),
                #[cfg(feature = "host")]
                None,
            ),
            crate::test_support::drain_flash_channel(),
            test,
        ));
    }

    #[test]
    fn cycling_uses_available_host_profiles() {
        use crate::ble::profile::BleProfileAction::{ClearSlot, Next, Previous};
        use crate::channel::BLE_PROFILE_CHANNEL;
        use crate::state::{CONNECTION_STATUS, current_profile};

        with_stalled_keyboard(async {
            let mut status = CONNECTION_STATUS.receiver().unwrap();
            status.get_and(|s| s.ble.state == BleState::Advertising).await;
            for (action, expected) in [(Previous, crate::NUM_BLE_PROFILE.saturating_sub(1) as u8), (Next, 0)] {
                BLE_PROFILE_CHANNEL.send(action).await;
                // The one-slot queue makes this wait until the preceding action was consumed.
                BLE_PROFILE_CHANNEL.send(ClearSlot(0)).await;
                assert_eq!(current_profile(), expected);
            }
        });
    }

    #[test]
    fn advertising_follows_slot_capacity_and_wakes_on_input() {
        use super::BOND_SLOTS;
        use crate::ble::profile::BleProfileAction;
        use crate::channel::BLE_PROFILE_CHANNEL;
        use crate::state::CONNECTION_STATUS;

        with_stalled_keyboard(async {
            let mut status = CONNECTION_STATUS.receiver().unwrap();
            status.get_and(|s| s.ble.state == BleState::Advertising).await;
            for wake in 0..3 {
                MockDriver::get().advance(Duration::from_secs(300));
                status.get_and(|s| s.ble.state == BleState::Inactive).await;
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
                    _ => BLE_PROFILE_CHANNEL.send(BleProfileAction::ClearSlot(0)).await,
                }
                let resumed = status.get_and(|s| s.ble.state == BleState::Advertising).await;
                assert_eq!(resumed.ble.profile, u8::from(wake == 2 && BOND_SLOTS > 1));
            }
        });
    }
}
