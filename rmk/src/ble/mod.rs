use core::cell::Cell;

#[cfg(feature = "subrating")]
use bt_hci::cmd::le::LeSubrateRequest;
#[cfg(feature = "_ext_adv")]
use bt_hci::cmd::le::{
    LeAddDeviceToFilterAcceptList, LeClearAdvSets, LeClearFilterAcceptList, LeExtCreateConn,
    LeReadNumberOfSupportedAdvSets, LeSetAdvSetRandomAddr, LeSetExtAdvData, LeSetExtAdvEnable, LeSetExtAdvParams,
    LeSetExtScanEnable, LeSetExtScanParams, LeSetExtScanResponseData,
};
use bt_hci::cmd::le::{LeRand, LeReadLocalSupportedFeatures, LeSetPhy};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use bt_hci::param::Error as HciError;
use embassy_futures::join::{join_array, join3, join5};
use embassy_futures::select::{Either, Either4, select, select3, select4};
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::channel::Channel;
#[cfg(feature = "rynk")]
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_sync::watch::{Receiver, Watch};
use embassy_time::{Duration, Instant, Timer};
use rmk_types::ble::BleState;
use rmk_types::connection::ConnectionType;
use rmk_types::led_indicator::LedIndicator;
use trouble_host::prelude::*;
use usbd_hid::descriptor::{MediaKeyboardReport, SystemControlReport};

#[cfg(not(feature = "_ext_adv"))]
use crate::ble::adv::advertise;
#[cfg(feature = "_ext_adv")]
use crate::ble::adv::advertise_profile;
use crate::ble::adv::advertising_request;
use crate::ble::battery_service::BleBatteryServer;
#[cfg(feature = "split")]
use crate::ble::battery_service::BlePeripheralBatteryServer;
use crate::ble::ble_server::{BleHidServer, Server};
use crate::ble::device_info::{PnPID, VidSource};
#[cfg(all(feature = "host", feature = "rynk"))]
use crate::ble::host::RynkBleSource;
#[cfg(feature = "host")]
use crate::ble::host::{HOST_WRITE_BUFFER_SIZE, HostGattHandler, HostWriteOutcome};
use crate::ble::led::BleLedReader;
#[cfg(feature = "passkey_entry")]
use crate::ble::passkey::{PasskeyInputState, next_gatt_event};
use crate::ble::profile::{BOND_SLOTS, BleProfileAction, ProfileInfo, ProfileManager, UPDATED_PROFILES};
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
use crate::state::set_ble_state;

pub(crate) mod adv;
pub(crate) mod battery_service;
pub(crate) mod ble_server;
pub(crate) mod device_info;
#[cfg(feature = "host")]
pub(crate) mod host;
pub(crate) mod led;
pub mod passkey;
pub(crate) mod profile;
pub(crate) mod rng;
#[cfg(test)]
mod runtime_tests;
#[cfg(any(feature = "split", feature = "dongle"))]
pub(crate) mod scan;
pub(crate) mod sleep;

#[cfg(all(feature = "subrating", feature = "_no_subrating"))]
compile_error!("You may not enable feature `subrating` on unsupported platforms!");

/// Controller commands for extended advertising, scanning and connecting.
#[cfg(feature = "_ext_adv")]
pub(crate) trait ExtAdvCmds:
    for<'t> ControllerCmdSync<LeSetExtAdvData<'t>>
    + ControllerCmdSync<LeClearAdvSets>
    + ControllerCmdSync<LeSetExtAdvParams>
    + ControllerCmdSync<LeSetAdvSetRandomAddr>
    + ControllerCmdSync<LeReadNumberOfSupportedAdvSets>
    + for<'t> ControllerCmdSync<LeSetExtAdvEnable<'t>>
    + for<'t> ControllerCmdSync<LeSetExtScanResponseData<'t>>
    + ControllerCmdSync<LeSetExtScanParams>
    + ControllerCmdSync<LeSetExtScanEnable>
    + ControllerCmdSync<LeClearFilterAcceptList>
    + ControllerCmdSync<LeAddDeviceToFilterAcceptList>
    + ControllerCmdAsync<LeExtCreateConn>
{
}

#[cfg(feature = "_ext_adv")]
impl<T> ExtAdvCmds for T where
    T: for<'t> ControllerCmdSync<LeSetExtAdvData<'t>>
        + ControllerCmdSync<LeClearAdvSets>
        + ControllerCmdSync<LeSetExtAdvParams>
        + ControllerCmdSync<LeSetAdvSetRandomAddr>
        + ControllerCmdSync<LeReadNumberOfSupportedAdvSets>
        + for<'t> ControllerCmdSync<LeSetExtAdvEnable<'t>>
        + for<'t> ControllerCmdSync<LeSetExtScanResponseData<'t>>
        + ControllerCmdSync<LeSetExtScanParams>
        + ControllerCmdSync<LeSetExtScanEnable>
        + ControllerCmdSync<LeClearFilterAcceptList>
        + ControllerCmdSync<LeAddDeviceToFilterAcceptList>
        + ControllerCmdAsync<LeExtCreateConn>
{
}

#[cfg(not(feature = "_ext_adv"))]
pub(crate) trait ExtAdvCmds {}

#[cfg(not(feature = "_ext_adv"))]
impl<T> ExtAdvCmds for T {}

/// Host and dongle connections: one per profile in extended mode.
#[doc(hidden)]
pub const HOST_LINKS: usize = if cfg!(feature = "_ext_adv") {
    crate::ble::profile::BOND_SLOTS
} else {
    1
};
const _: () = core::assert!(HOST_LINKS <= u8::MAX as usize, "BLE host capacity must fit in u8");

/// Max number of connections of a keyboard's BLE stack; a dongle sizes its
/// own — see [`crate::dongle::Dongle`].
#[doc(hidden)]
pub const CONNECTIONS_MAX: usize = crate::SPLIT_PERIPHERALS_NUM + HOST_LINKS;

/// Whether controller setup must enable extended advertising commands.
#[doc(hidden)]
pub const EXT_ADV_ENABLED: bool = cfg!(feature = "_ext_adv");

/// Reserve additional SDC resources using the upper bounds in nrf-sdc a22ca716's sdc.h.
/// Applies to RMK's default 31-byte advertisement and three scan buffers.
#[doc(hidden)]
pub const fn nrf_sdc_memory_size(
    legacy_size: usize,
    central_links: usize,
    packet_size: usize,
    tx_buffers: u8,
    rx_buffers: u8,
) -> usize {
    if !EXT_ADV_ENABLED {
        return legacy_size;
    }
    // SDC_MEM_PER_PERIPHERAL_LINK, including optional SDC_MEM_SUBRATING per link.
    let per_link = 923 + (tx_buffers as usize + rx_buffers as usize) * (packet_size + 15) - 84
        + if cfg!(feature = "subrating") { 60 } else { 0 };
    let per_link = (per_link + 7) & !7;
    let advertising = (5184 + 31 * 18) / 10; // SDC_MEM_PER_ADV_SET(31).
    // SDC_MEM_SCAN_EXT(3) plus SDC_MEM_INITIATOR; retain the legacy allocation too.
    let scanning = if central_links > 0 { 408 + 3 * 320 + 384 } else { 0 };
    (legacy_size + HOST_LINKS.saturating_sub(1) * per_link + advertising + scanning + 7) & !7
}

/// Max simultaneous `active` receivers: one host task, one GATT write gate and
/// one report router per connection.
pub(crate) const ACTIVE_RECEIVERS: usize = HOST_LINKS * if cfg!(feature = "rynk") { 3 } else { 1 };

/// Shared Rynk frame storage for BLE: one buffer and one RX ring for the whole
/// transport, driven only by the connection whose profile is active.
#[cfg(all(feature = "host", feature = "rynk"))]
pub(crate) struct BleConfig {
    pub(crate) buffer: Mutex<NoopRawMutex, [u8; crate::RYNK_BUFFER_SIZE]>,
    pub(crate) pipe: embassy_sync::pipe::Pipe<NoopRawMutex, 512>,
}

/// Max number of L2CAP channels
const L2CAP_CHANNELS_MAX: usize = CONNECTIONS_MAX * 4; // Signal + att + smp + hid

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
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeRand>
        + ExtAdvCmds,
{
    async fn run(&mut self) -> ! {
        // Load the preferred connection from storage
        let preferred = crate::state::load_preferred_connection().await;
        crate::state::set_preferred_connection(preferred);

        // Seeding through a reference keeps the controller out of the future across the await.
        if let Err(e) = rng::seed_from_hardware(self.controller.as_ref().expect("BleTransport::run called twice")).await
        {
            error!("[ble] failed to seed the RNG, rebooting: {:?}", e);
            Timer::after_millis(100).await;
            crate::boot::reboot_keyboard();
        }
        let controller = self.controller.take().expect("BleTransport::run called twice");
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
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>
        + ControllerCmdSync<LeRand>
        + ExtAdvCmds,
    #[cfg(feature = "subrating")] C: Controller
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>
        + ControllerCmdSync<LeRand>
        + ExtAdvCmds
        + ControllerCmdAsync<LeSubrateRequest>,
> Runnable for BleTransport<'a, C>
{
    async fn run(&mut self) -> ! {
        // Load the preferred connection from storage
        let preferred = crate::state::load_preferred_connection().await;
        crate::state::set_preferred_connection(preferred);

        // Seeding through a reference keeps the controller out of the future across the await.
        if let Err(e) = rng::seed_from_hardware(self.controller.as_ref().expect("BleTransport::run called twice")).await
        {
            error!("[ble] failed to seed the RNG, rebooting: {:?}", e);
            Timer::after_millis(100).await;
            crate::boot::reboot_keyboard();
        }
        let controller = self.controller.take().expect("BleTransport::run called twice");

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
        join3(
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
    // The active profile is broadcast to every connection; each connection
    // creates its receivers when it starts serving.
    let active: Watch<NoopRawMutex, u8, ACTIVE_RECEIVERS> = Watch::new();
    let mut profile_manager: ProfileManager<_, _, BOND_SLOTS> = ProfileManager::new(stack);
    // Load the bonded devices from storage
    profile_manager.load_bonded_devices().await;
    profile_manager.update_stack_bonds();
    active.sender().send(crate::state::current_profile());

    // One Rynk buffer and RX ring for the whole BLE transport; only the active
    // profile's connection drives them.
    #[cfg(all(feature = "host", feature = "rynk"))]
    let ble_config = host_service.map(|_| BleConfig {
        buffer: Mutex::new([0; crate::RYNK_BUFFER_SIZE]),
        pipe: embassy_sync::pipe::Pipe::new(),
    });

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    struct Link<'a, 'b> {
        conn: GattConnection<'a, 'b, DefaultPacketPool>,
        profile: u8,
        bond_info: Option<ProfileInfo>,
    }

    let incoming: Channel<NoopRawMutex, Link<'_, '_>, 1> = Channel::new();
    let completed: Channel<NoopRawMutex, u8, HOST_LINKS> = Channel::new();
    let workers = join_array(core::array::from_fn::<_, HOST_LINKS, _>(|_| async {
        loop {
            let Link {
                conn,
                profile,
                bond_info,
            } = incoming.receive().await;
            serve_keyboard_connection(
                server,
                &conn,
                stack,
                profile,
                bond_info,
                config,
                #[cfg(feature = "host")]
                host_service,
                &active,
                #[cfg(all(feature = "host", feature = "rynk"))]
                ble_config.as_ref(),
            )
            .await;
            drop(conn);
            completed.send(profile).await;
        }
    }));
    let profile_manager = &mut profile_manager;

    let connection_loop = async {
        let mut links: [Option<Connection<'_, DefaultPacketPool>>; BOND_SLOTS] = core::array::from_fn(|_| None);
        let mut deadline = Instant::now() + Duration::from_secs(300);
        let mut wake_subscribers = None;
        Timer::after_millis(10).await;
        loop {
            let now = Instant::now();
            let profile = crate::state::current_profile();
            if now < deadline {
                wake_subscribers = None;
            } else if wake_subscribers.is_none() {
                if let Some(pairing) = profile_manager.repairing {
                    disconnect_profile(pairing, &mut links, &completed).await;
                    profile_manager.set_pairing(None).await;
                }
                wake_subscribers = Some((
                    crate::event::KeyboardEvent::subscriber(),
                    crate::event::PointingEvent::subscriber(),
                ));
                if links.get(profile as usize).is_some_and(Option::is_none) {
                    set_ble_state(BleState::Inactive);
                    request_sleep();
                }
            }
            let bond = profile_manager.bond_info(profile);
            let request = advertising_request(profile, &links, now, deadline, bond.as_ref(), address, product_name)
                .filter(|_| profile_manager.background_connections || links.iter().all(Option::is_none));
            if request.is_some() {
                set_ble_state(BleState::Advertising);
            }
            let accept = async {
                if let Some(request) = &request {
                    let timeout = deadline.saturating_duration_since(now);
                    #[cfg(feature = "_ext_adv")]
                    let result = advertise_profile(&mut peripheral, &server.server, request, timeout).await;
                    #[cfg(not(feature = "_ext_adv"))]
                    let result = advertise(&mut peripheral, &server.server, request.adv, timeout).await;
                    result.map(|conn| (request.profile, conn))
                } else {
                    core::future::pending().await
                }
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
            match select4(accept, profile_manager.next_action(), completed.receive(), activity).await {
                Either4::First(Ok((profile, conn))) => {
                    if profile != crate::state::current_profile() || links[profile as usize].is_some() {
                        disconnect(&conn).await;
                        continue;
                    }
                    // A replaced advertisement can still deliver a late connection.
                    #[cfg(feature = "_ext_adv")]
                    if conn.raw().local_address()
                        != Some(BdAddr::new(crate::ble::profile::profile_address(address, profile)))
                    {
                        warn!("[ble] late connection belongs to another profile");
                        disconnect(&conn).await;
                        continue;
                    }
                    let bond_info = profile_manager.bond_info(profile);
                    if let Some(bond) = &bond_info
                        && !bond.info.identity.match_identity(&conn.raw().peer_identity())
                    {
                        warn!("[ble] peer does not match its profile");
                        disconnect(&conn).await;
                        continue;
                    }
                    #[cfg(feature = "dongle")]
                    if profile != crate::ble::profile::DONGLE_PROFILE
                        && profile_manager.is_bonded_dongle(&conn.raw().peer_identity())
                    {
                        warn!("[ble] dongle connected on a host profile");
                        disconnect(&conn).await;
                        continue;
                    }
                    if let Err(e) = conn.raw().set_bondable(bond_info.is_none()) {
                        error!("Set bondable error: {:?}", e);
                        disconnect(&conn).await;
                        continue;
                    }
                    links[profile as usize] = Some(conn.raw().clone());
                    incoming
                        .send(Link {
                            conn,
                            profile,
                            bond_info,
                        })
                        .await;
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
                            let enabled = enabled && HOST_LINKS > 1;
                            profile_manager.background_connections = enabled;
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
                        BleProfileAction::Previous => {
                            if current == 0 {
                                crate::NUM_BLE_PROFILE.saturating_sub(1) as u8
                            } else {
                                current - 1
                            }
                        }
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
                    if usize::from(target) >= BOND_SLOTS || (!clear && !pair && target == current) {
                        continue;
                    }
                    report_activity();
                    deadline = Instant::now() + Duration::from_secs(300);
                    if !clear && target != current {
                        if let Some(pairing) = profile_manager.repairing {
                            disconnect_profile(pairing, &mut links, &completed).await;
                            profile_manager.set_pairing(None).await;
                        }
                        if !profile_manager.background_connections
                            && let Some(conn) = &links[current as usize]
                        {
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
                        profile_manager.switch_profile(target, state, &active).await;
                    }
                    if clear || pair {
                        disconnect_profile(target, &mut links, &completed).await;
                        if clear {
                            profile_manager.clear_bond(target).await;
                        } else {
                            profile_manager.set_pairing(Some(target)).await;
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
        workers,
        run_led_reader(&mut ble_led_reader, ConnectionType::Ble),
        sleep::run_sleep_manager(),
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
#[allow(clippy::too_many_arguments)]
async fn gatt_events_task(
    server: &Server<'_>,
    conn: &GattConnection<'_, '_, DefaultPacketPool>,
    profile: u8,
    mut profile_info: Option<ProfileInfo>,
    led: &Cell<LedIndicator>,
    #[cfg(feature = "rynk")] active: Receiver<'_, NoopRawMutex, u8, ACTIVE_RECEIVERS>,
    #[cfg(all(feature = "host", feature = "rynk"))] ble_config: Option<&BleConfig>,
    #[cfg(all(feature = "host", feature = "rynk"))] source: &Cell<RynkBleSource>,
) {
    let level = server.battery_service.level;
    #[cfg(feature = "split")]
    let peripheral_levels = server.peripheral_battery_services.levels;
    let output_keyboard = server.hid_service.output_keyboard;
    let hid_control_point = server.hid_service.hid_control_point;
    let input_keyboard = server.hid_service.input_keyboard;
    let mouse = server.hid_service.mouse_report;
    let media = server.hid_service.media_report;
    let system_control = server.hid_service.system_report;

    #[cfg(feature = "passkey_entry")]
    let mut passkey_state = PasskeyInputState::new();

    #[cfg(feature = "host")]
    let mut host_gatt_handler = HostGattHandler::new(
        server,
        #[cfg(feature = "rynk")]
        active,
        #[cfg(feature = "rynk")]
        profile,
        #[cfg(all(feature = "host", feature = "rynk"))]
        ble_config,
        #[cfg(all(feature = "host", feature = "rynk"))]
        source,
    );

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
                    let snapshot = ProfileInfo {
                        slot_num: profile,
                        info: bond_info,
                        removed: false,
                        cccd_table,
                    };
                    UPDATED_PROFILES[profile as usize].signal(snapshot.clone());
                    profile_info = Some(snapshot);
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
                                led.set(led_indicator);
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
                            match host_gatt_handler.handle_write(event.handle(), data, encrypted).await {
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

                    if let Some(info) = &mut profile_info
                        && let Some(table) = server.get_client_att_table(conn.raw())
                        && let Ok(bytes) = heapless::Vec::from_slice(table.raw())
                    {
                        info.cccd_table = bytes;
                        UPDATED_PROFILES[profile as usize].signal(info.clone());
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
                if profile_info.is_some() {
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
    completed: &Channel<NoopRawMutex, u8, HOST_LINKS>,
) {
    if let Some(conn) = &links[profile as usize] {
        conn.disconnect();
    }
    while links[profile as usize].is_some() {
        let ended = completed.receive().await;
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
#[allow(clippy::too_many_arguments)]
async fn serve_keyboard_connection<
    'a,
    'b,
    #[cfg(feature = "host")] 'r,
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
>(
    server: &'b Server<'_>,
    conn: &GattConnection<'a, 'b, DefaultPacketPool>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    profile: u8,
    bond_info: Option<ProfileInfo>,
    config: &BleBatteryConfig,
    #[cfg(feature = "host")] host_service: Option<&'r crate::host::HostService<'r>>,
    active: &Watch<NoopRawMutex, u8, ACTIVE_RECEIVERS>,
    #[cfg(all(feature = "host", feature = "rynk"))] ble_config: Option<&BleConfig>,
) {
    let mut ble_hid_server = BleHidServer::new(server, conn);
    let led = Cell::new(LedIndicator::default());
    let mut report_active = active.receiver().expect("active receiver slots exhausted");
    #[cfg(feature = "rynk")]
    let gatt_active = active.receiver().expect("active receiver slots exhausted");
    #[cfg(feature = "rynk")]
    let host_active = active.receiver().expect("active receiver slots exhausted");
    #[cfg(all(feature = "host", feature = "rynk"))]
    let source = Cell::new(RynkBleSource::None);
    let mut ble_battery_server = config.enabled.then(|| BleBatteryServer::new(server, conn));
    #[cfg(feature = "split")]
    let mut ble_peripheral_battery_server = crate::SPLIT_BATTERY_PERIPHERAL_IDS
        .first()
        .map(|_| BlePeripheralBatteryServer::new(server, conn));

    // CCCD lookup uses cached bond info to avoid a cancellable flash read while
    // this future is racing other arms of an outer `select`.
    if let Some(bond_info) = &bond_info
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

    let communication_task = async {
        // PHY negotiation must not delay GATT disconnection or input routing.
        let connection_params = async {
            update_ble_phy(stack, conn.raw(), host_phy).await;
            set_conn_params(stack, conn).await;
        };
        select3(
            gatt_events_task(
                server,
                conn,
                profile,
                bond_info,
                &led,
                #[cfg(feature = "rynk")]
                gatt_active,
                #[cfg(all(feature = "host", feature = "rynk"))]
                ble_config,
                #[cfg(all(feature = "host", feature = "rynk"))]
                &source,
            ),
            connection_params,
            battery_task,
        )
        .await;
    };

    let writer_task = async {
        loop {
            report_active.get_and(|&p| p == profile).await;
            if conn.raw().security_level().is_ok_and(|level| level.encrypted()) {
                set_ble_state(BleState::Connected);
            }
            LED_SIGNAL.signal(led.get());
            write_active_reports(profile, &mut ble_hid_server, &mut report_active).await;
        }
    };

    #[cfg(feature = "host")]
    let host_task = async {
        if let Some(service) = host_service {
            #[cfg(feature = "rynk")]
            HostGattHandler::run(
                server,
                conn,
                service,
                host_active,
                profile,
                ble_config.expect("BleConfig exists when a host service is set"),
                &source,
            )
            .await;
            #[cfg(feature = "vial")]
            loop {
                HostGattHandler::run(server, conn, service).await;
            }
        } else {
            core::future::pending::<()>().await;
        }
    };
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

    let inner = join3(writer_task, host_task, dongle_event_task);
    select(communication_task, inner).await;
}

/// Send this profile's reports until it loses active, then release its held inputs.
async fn write_active_reports<W: HidWriterTrait<ReportType = Report>>(
    profile: u8,
    writer: &mut W,
    active: &mut Receiver<'_, NoopRawMutex, u8, ACTIVE_RECEIVERS>,
) {
    loop {
        match select(active.changed_and(|&p| p != profile), BLE_REPORT_CHANNEL.receive()).await {
            Either::First(_) => break,
            Either::Second(report) => {
                if let Err(e) = writer.write_report(&report).await {
                    error!("Failed to send report: {:?}", e);
                }
            }
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
        let _ = writer.write_report(&report).await;
    }
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

    use embassy_futures::join::join;
    use embassy_futures::select::select;
    use embassy_time::Timer;
    use rmk_types::ble::{BleState, BleStatus};

    use crate::event::{Axis, AxisEvent, AxisValType, KeyboardEvent, PointingEvent, SubscribableEvent, publish_event};
    use crate::state::{current_ble_status, set_ble_profile, set_ble_state};
    use crate::test_support::test_block_on as block_on;

    #[cfg(feature = "_ext_adv")]
    #[test]
    fn battery_subscriber_budget_covers_all_host_links_and_config_sessions() {
        use super::HOST_LINKS;
        use crate::event::BatteryStatusEvent;

        let count = HOST_LINKS + 1 + if cfg!(feature = "rynk") { 2 } else { 0 };
        let _subscribers: Vec<_> = (0..count).map(|_| BatteryStatusEvent::subscriber()).collect();
    }

    #[cfg(feature = "_ext_adv")]
    #[test]
    fn active_report_routing_releases_old_host_and_leaves_background_idle() {
        use core::future::Future;
        use core::pin::pin;
        use core::task::{Context, Waker};

        use super::{ACTIVE_RECEIVERS, NoopRawMutex, Watch, write_active_reports};
        use crate::channel::BLE_REPORT_CHANNEL;
        use crate::hid::{HidError, HidWriterTrait, KeyboardReport, Report};

        struct Writer(Vec<Report>);
        impl HidWriterTrait for Writer {
            type ReportType = Report;

            async fn write_report(&mut self, report: &Report) -> Result<usize, HidError> {
                self.0.push(report.clone());
                Ok(1)
            }
        }

        BLE_REPORT_CHANNEL.clear();
        let active = Watch::<NoopRawMutex, u8, ACTIVE_RECEIVERS>::new();
        active.sender().send(0);
        let mut a = active.receiver().unwrap();
        let mut b = active.receiver().unwrap();
        let mut old = Writer(Vec::new());
        let mut new = Writer(Vec::new());
        {
            let mut a_task = pin!(async {
                a.get_and(|&p| p == 0).await;
                write_active_reports(0, &mut old, &mut a).await;
            });
            let mut b_task = pin!(async {
                b.get_and(|&p| p == 1).await;
                write_active_reports(1, &mut new, &mut b).await;
            });
            let mut cx = Context::from_waker(Waker::noop());
            BLE_REPORT_CHANNEL
                .try_send(Report::KeyboardReport(KeyboardReport {
                    modifier: 2,
                    keycodes: [4, 0, 0, 0, 0, 0],
                    ..Default::default()
                }))
                .unwrap();
            assert!(b_task.as_mut().poll(&mut cx).is_pending());
            assert!(!BLE_REPORT_CHANNEL.is_empty());
            assert!(a_task.as_mut().poll(&mut cx).is_pending());
            assert!(BLE_REPORT_CHANNEL.is_empty());

            active.sender().send(1);
            assert!(a_task.as_mut().poll(&mut cx).is_ready());
            BLE_REPORT_CHANNEL
                .try_send(Report::KeyboardReport(KeyboardReport {
                    keycodes: [5, 0, 0, 0, 0, 0],
                    ..Default::default()
                }))
                .unwrap();
            assert!(b_task.as_mut().poll(&mut cx).is_pending());
        }
        assert_eq!(old.0.len(), 5 + usize::from(cfg!(feature = "steno")));
        assert!(matches!(&old.0[0], Report::KeyboardReport(r) if r.modifier == 2 && r.keycodes[0] == 4));
        assert!(matches!(&old.0[1], Report::KeyboardReport(r) if r.modifier == 0 && r.keycodes == [0; 6]));
        assert!(
            matches!(&old.0[2], Report::MouseReport(r) if r.buttons == 0 && r.x == 0 && r.y == 0 && r.wheel == 0 && r.pan == 0)
        );
        assert!(matches!(&old.0[3], Report::MediaKeyboardReport(r) if r.usage_id == 0));
        assert!(matches!(&old.0[4], Report::SystemControlReport(r) if r.usage_id == 0));
        #[cfg(feature = "steno")]
        assert!(matches!(&old.0[5], Report::StenoReport(r) if r.keys == [0; 8]));
        assert_eq!(new.0.len(), 1);
        assert!(matches!(&new.0[0], Report::KeyboardReport(r) if r.keycodes[0] == 5));
    }

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

        set_ble_profile(2, true, BleState::Inactive);
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

        set_ble_profile(1, false, BleState::Inactive);
        set_ble_state(BleState::Connected);
        set_ble_profile(3, true, BleState::Inactive);

        assert_eq!(
            current_ble_status(),
            BleStatus {
                profile: 3,
                state: BleState::Inactive,
                bonded: true,
            }
        );
    }

    #[test]
    fn wake_activity_includes_pointing_events() {
        let _guard = ble_status_test_lock().lock().unwrap();

        block_on(async {
            let wake = async {
                let mut key_wake = KeyboardEvent::subscriber();
                let mut pointing_wake = PointingEvent::subscriber();
                let _ = select(key_wake.next_message_pure(), pointing_wake.next_message_pure()).await;
            };
            join(wake, async {
                Timer::after_millis(1).await;
                publish_event(PointingEvent {
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
                })
            })
            .await;
        });
    }
}
