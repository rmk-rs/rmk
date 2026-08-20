use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy};
#[cfg(feature = "subrating")]
use bt_hci::cmd::le::{LeSetHostFeature, LeSubrateRequest, LeSubrateRequestParams};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use bt_hci::param::Error as HciError;
use embassy_futures::join::join3;
use embassy_futures::select::{Either, Either3, select, select3};
#[cfg(feature = "split")]
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
#[cfg(feature = "split")]
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};
use rmk_types::ble::BleState;
use rmk_types::connection::ConnectionType;
use rmk_types::led_indicator::LedIndicator;
use trouble_host::prelude::*;

use crate::ble::adv::{Adv, advertise};
use crate::ble::battery_service::BleBatteryServer;
#[cfg(feature = "split")]
use crate::ble::battery_service::BlePeripheralBatteryServer;
use crate::ble::ble_server::{BleHidServer, Server};
use crate::ble::device_info::{PnPID, VidSource};
#[cfg(feature = "host")]
use crate::ble::host::{HOST_WRITE_BUFFER_SIZE, HostGattHandler, HostWriteOutcome};
use crate::ble::led::BleLedReader;
#[cfg(feature = "passkey_entry")]
use crate::ble::passkey::{PasskeyInputState, next_gatt_event};
use crate::ble::profile::{BOND_SLOTS, ProfileInfo, ProfileManager, UPDATED_CCCD_TABLE, UPDATED_PROFILE};
use crate::ble::sleep::{report_activity, request_sleep};
use crate::channel::{BLE_REPORT_CHANNEL, LED_SIGNAL};
use crate::config::{BleBatteryConfig, DeviceConfig, RmkConfig};
use crate::core_traits::Runnable;
use crate::event::SubscribableEvent;
use crate::hid::{HidWriterTrait, run_led_reader};
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
#[cfg(feature = "_nrf_ble")]
pub(crate) mod nrf;
pub mod passkey;
pub(crate) mod profile;
#[cfg(any(feature = "split", feature = "dongle"))]
pub(crate) mod scan;
pub(crate) mod sleep;

#[cfg(all(feature = "subrating", feature = "_no_subrating"))]
compile_error!("You may not enable feature `subrating` on unsupported platforms!");

/// Max number of connections of a keyboard's BLE stack; a dongle sizes its
/// own — see [`crate::dongle::Dongle`].
const CONNECTIONS_MAX: usize = crate::SPLIT_PERIPHERALS_NUM + 1;

/// Max number of L2CAP channels
const L2CAP_CHANNELS_MAX: usize = CONNECTIONS_MAX * 4; // Signal + att + smp + hid

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
    config: BleBatteryConfig<'static>,
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
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    async fn run(&mut self) -> ! {
        // Load the preferred connection from storage
        let preferred = crate::state::load_preferred_connection().await;
        crate::state::set_preferred_connection(preferred);

        let controller = self.controller.take().expect("BleTransport::run called twice");
        // Exactly one link — the host.
        let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> = HostResources::new();
        let stack = trouble_host::new(controller, &mut resources)
            .set_random_address(Address::random(self.address))
            .build();
        run_ble_keyboard(
            &stack,
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
        + ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>,
    #[cfg(feature = "subrating")] C: Controller
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>
        + ControllerCmdSync<LeSetHostFeature>
        + ControllerCmdAsync<LeSubrateRequest>,
> Runnable for BleTransport<'a, C>
{
    async fn run(&mut self) -> ! {
        // Load the preferred connection from storage
        let preferred = crate::state::load_preferred_connection().await;
        crate::state::set_preferred_connection(preferred);

        let controller = self.controller.take().expect("BleTransport::run called twice");

        let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> = HostResources::new();
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
    #[cfg(not(feature = "subrating"))] C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    #[cfg(feature = "subrating")] C: Controller
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeSetHostFeature>,
>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    device_config: &DeviceConfig<'static>,
    config: &BleBatteryConfig<'static>,
    #[cfg(feature = "host")] host_service: Option<&'r crate::host::HostService<'r>>,
) -> ! {
    let product_name = device_config.product_name;
    #[cfg(feature = "_nrf_ble")]
    let serial_number = crate::ble::nrf::get_serial_number();
    #[cfg(not(feature = "_nrf_ble"))]
    let serial_number = device_config.serial_number;

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
    // The serial number characteristic is length limited, so truncate at a char
    // boundary instead of panicking when the configured serial is too long.
    let mut serial_number_trimmed = heapless::String::new();
    for c in serial_number.chars() {
        if serial_number_trimmed.push(c).is_err() {
            break;
        }
    }
    server
        .set(&server.device_config_service.serial_number, &serial_number_trimmed)
        .unwrap();
    server
        .set(
            &server.device_config_service.manufacturer_name,
            &heapless::String::try_from(device_config.manufacturer).unwrap(),
        )
        .unwrap();
    let server = &server;

    if crate::ble::passkey::passkey_entry_enabled() {
        stack.set_io_capabilities(IoCapabilities::KeyboardOnly);
    }
    let mut profile_manager: ProfileManager<_, _, BOND_SLOTS> = ProfileManager::new(stack);
    // Load the bonded devices from storage
    profile_manager.load_bonded_devices().await;
    profile_manager.update_stack_bonds();

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    let profile_manager = &mut profile_manager;

    let connection_loop = async {
        // Subrating: set host feature flag BEFORE ANY CONNECTIONS
        // This must run concurrently with the ble_task runner (which processes HCI commands).
        #[cfg(feature = "subrating")]
        init_subrating_host_feature(stack).await;
        STACK_STARTED.signal(());

        loop {
            // On the dongle slot, advertise directed to the bonded dongle or
            // as a seeking broadcast; on the normal profiles, plain HID.
            #[cfg(feature = "dongle")]
            let adv = if crate::state::current_profile() == crate::ble::profile::DONGLE_PROFILE {
                match profile_manager.active_bond_info() {
                    Some(info) => Adv::Directed(info.info.identity.addr),
                    None => Adv::DongleSeeking,
                }
            } else {
                Adv::Host { name: product_name }
            };
            #[cfg(not(feature = "dongle"))]
            let adv = Adv::Host { name: product_name };

            // Wait for 10ms to ensure the USB is checked
            Timer::after_millis(10).await;
            info!("[adv] advertising");
            set_ble_state(BleState::Advertising);

            match select(
                advertise(&mut peripheral, &server.server, adv, Duration::from_secs(300)),
                profile_manager.update_profile(),
            )
            .await
            {
                Either::First(Ok(conn)) => {
                    info!("[adv] connection established");
                    if let Err(e) = conn.raw().set_bondable(true) {
                        error!("Set bondable error: {:?}", e);
                    }
                    // Do NOT emit BleState::Connected here. gatt_events_task emits
                    // Connected when it sees GattConnectionEvent::Encrypted.
                    let active_bond_info = profile_manager.active_bond_info();
                    // Check the bond info after the connection is just created.
                    if let Some(bond) = &active_bond_info
                        && !bond.info.identity.match_identity(&conn.raw().peer_identity())
                    {
                        warn!("[ble] connected peer doesn't match the active profile, disconnecting");
                        disconnect(&conn).await;
                        continue;
                    }
                    // When connecting to BLE host, check the connected peer is not a dongle.
                    #[cfg(feature = "dongle")]
                    if crate::state::current_profile() != crate::ble::profile::DONGLE_PROFILE
                        && profile_manager.is_bonded_dongle(&conn.raw().peer_identity())
                    {
                        warn!("[ble] the bonded dongle connected on a host BLE profile, disconnecting");
                        disconnect(&conn).await;
                        continue;
                    }
                    if let Either::Second(_) = select(
                        serve_keyboard_connection(
                            server,
                            &conn,
                            stack,
                            active_bond_info,
                            config,
                            #[cfg(feature = "host")]
                            host_service,
                        ),
                        profile_manager.update_profile(),
                    )
                    .await
                    {
                        // When the profile changes, manually disconnect from the current host
                        disconnect(&conn).await;
                    }
                }
                Either::First(Err(BleHostError::BleHost(Error::Timeout))) => {
                    warn!("Advertising timeout, sleep and wait for any key");
                    set_ble_state(BleState::Inactive);

                    request_sleep();

                    // Wake on key or pointing activity after the advertising
                    // timeout. Subscribed here, not up front: a permanently
                    // idle subscriber stalls `publish_event_async` once the
                    // channel fills, and its backlog would satisfy this wait
                    // instantly with a stale event.
                    let mut key_wake = crate::event::KeyboardEvent::subscriber();
                    let mut pointing_wake = crate::event::PointingEvent::subscriber();
                    let _ = select(key_wake.next_message_pure(), pointing_wake.next_message_pure()).await;

                    report_activity();
                }
                Either::First(Err(e)) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Advertise error: {:?}", e);
                    Timer::after_millis(200).await;
                }
                Either::Second(()) => {}
            };

            // Skip the Inactive transition if we never moved off Advertising
            if crate::state::current_ble_status().state != BleState::Advertising {
                set_ble_state(BleState::Inactive);
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
    join3(
        ble_task(runner, &event_handler),
        connection_loop,
        sleep::run_sleep_manager(),
    )
    .await;
    unreachable!("BleTransport sub-tasks must run forever")
}

/// Initialize subrating host support.
///
/// **Must** be called concurrently with `ble_task()` (the runner processes the
/// HCI command) and **before** any advertising, scanning or connecting, because
/// the controller only applies the feature flag to connections established after
/// it is set.
#[cfg(feature = "subrating")]
pub(crate) async fn init_subrating_host_feature<C: Controller + ControllerCmdSync<LeSetHostFeature>>(
    stack: &Stack<'_, C, impl PacketPool>,
) {
    const CONN_SUBRATING_HOST_BIT: u8 = 38;
    let cmd = LeSetHostFeature::new(CONN_SUBRATING_HOST_BIT, 1);
    if let Err(e) = stack.command(cmd).await {
        error!("[Host] error setting subrating host feature flag: {:?}", e);
    }
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
    loop {
        if let Err(e) = runner.run_with_handler(handler).await {
            error!("[ble_task] runner error: {:?}", e);
            Timer::after_millis(100).await;
        }
    }
}

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
async fn gatt_events_task(server: &Server<'_>, conn: &GattConnection<'_, '_, DefaultPacketPool>) -> Result<(), Error> {
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
    let mut host_gatt_handler = HostGattHandler::new(server);

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
                let profile = crate::state::current_profile();
                if let Some(bond_info) = bond {
                    let cccd_table = server
                        .get_client_att_table(conn.raw())
                        .and_then(|t| heapless::Vec::from_slice(t.raw()).ok())
                        .unwrap_or_default();
                    let profile_info = ProfileInfo {
                        slot_num: profile,
                        info: bond_info,
                        removed: false,
                        cccd_table,
                    };
                    UPDATED_PROFILE.signal(profile_info);
                }
            }
            GattConnectionEvent::PairingFailed(err) => {
                #[cfg(feature = "passkey_entry")]
                passkey_state.clear();
                error!("[gatt] pairing error: {:?}", err);
            }
            GattConnectionEvent::Encrypted { security_level, .. } => {
                info!("[gatt] encrypted: {:?}", security_level);
                set_ble_state(BleState::Connected);
            }
            GattConnectionEvent::Gatt { event: gatt_event } => {
                let mut cccd_updated = false;
                let result = match &gatt_event {
                    GattEvent::Read(event) => {
                        if event.handle() == level.handle {
                            let value = server.get(&level);
                            debug!("Read GATT Event to Level: {:?}", value);
                        } else {
                            #[cfg(feature = "split")]
                            let peripheral_level =
                                peripheral_levels.iter().find(|level| event.handle() == level.handle);
                            #[cfg(not(feature = "split"))]
                            let peripheral_level: Option<&Characteristic<u8>> = None;
                            if let Some(peripheral_level) = peripheral_level {
                                let value = server.get(peripheral_level);
                                debug!("Read GATT Event to Peripheral Level: {:?}", value);
                            } else {
                                debug!("Read GATT Event to Unknown: {:?}", event.handle());
                            }
                        }

                        if conn.raw().security_level()?.encrypted() {
                            None
                        } else {
                            Some(AttErrorCode::INSUFFICIENT_ENCRYPTION)
                        }
                    }
                    GattEvent::Write(event) => {
                        let encrypted = conn.raw().security_level()?.encrypted();

                        // trouble-host 0.7 exposes written bytes via a closure; copy them out
                        // once so the dispatch below (which awaits) can use them freely.
                        // Sized for the active host protocol's largest BLE write.
                        #[cfg(feature = "host")]
                        let mut data_buf = [0u8; HOST_WRITE_BUFFER_SIZE];
                        #[cfg(not(feature = "host"))]
                        let mut data_buf = [0u8; 32];
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
                                LED_SIGNAL.signal(led_indicator);
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

                        if control_point_write {
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
                match result {
                    Ok(reply) => reply.send().await,
                    Err(e) => warn!("[gatt] error sending response: {:?}", e),
                }

                // Update CCCD table after processing the event
                if cccd_updated {
                    // When macOS wakes up from sleep mode, it won't send EXIT SUSPEND command
                    // So we need to monitor the sleep state by using CCCD write event
                    report_activity();

                    if let Some(table) = server.get_client_att_table(conn.raw())
                        && let Ok(bytes) = heapless::Vec::from_slice(table.raw())
                    {
                        UPDATED_CCCD_TABLE.signal(bytes);
                    }
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
    Ok(())
}

/// Drop the link and wait for it to actually go down.
async fn disconnect(conn: &GattConnection<'_, '_, DefaultPacketPool>) {
    if !conn.raw().is_connected() {
        return;
    }
    conn.raw().disconnect();
    while !matches!(conn.next().await, GattConnectionEvent::Disconnected { .. }) {}
}

pub(crate) async fn set_conn_params<
    'a,
    'b,
    C: Controller + ControllerCmdSync<LeReadLocalSupportedFeatures>,
    P: PacketPool,
>(
    stack: &Stack<'_, C, P>,
    conn: &GattConnection<'a, 'b, P>,
) {
    // On the dongle slot the dongle (our own central) owns the link
    // parameters; the Apple-tuned requests below would push supervision back
    // to 10 s and slow down reconnect after a dongle power-cycle.
    #[cfg(feature = "dongle")]
    if crate::state::current_profile() == crate::ble::profile::DONGLE_PROFILE {
        core::future::pending::<()>().await;
    }

    // Apple rejects 7.5ms outright, so ask for the 15ms its guidelines allow
    // first and only then for 7.5ms: platforms that take it end up at the best
    // interval, and Apple keeps the 15ms it already granted.
    // Reference: https://developer.apple.com/accessories/Accessory-Design-Guidelines.pdf
    for interval in [Duration::from_millis(15), Duration::from_micros(7500)] {
        // Wait 5 seconds before each request to avoid connection drop
        embassy_time::Timer::after_secs(5).await;
        let max_latency = if cfg!(feature = "subrating") {
            // Set Central sleep for up to 2.25s to save power when the keyboard is asleep
            (Duration::from_millis(2250).as_micros() / interval.as_micros()) as u16
        } else {
            30
        };
        update_conn_params(
            stack,
            conn.raw(),
            &RequestedConnParams {
                min_connection_interval: interval,
                max_connection_interval: interval,
                max_latency,
                supervision_timeout: Duration::from_secs(10),
                ..Default::default()
            },
        )
        .await;
    }

    // Wait forever. This is because we want the conn params setting can be interrupted when the connection is lost.
    // So this task shouldn't quit after setting the conn params.
    core::future::pending::<()>().await;
}

/// Serve one host keyboard connection.
///
/// Returns when the GATT events task ends (i.e. the connection drops).
/// `writer_task`, `led_task`, and `host_task` are all infinite, so the outer
/// `select(communication_task, inner)` cancels them as a side-effect of
/// `communication_task` returning. `inner` itself never completes.
async fn serve_keyboard_connection<
    'a,
    'b,
    #[cfg(feature = "host")] 'r,
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
>(
    server: &'b Server<'_>,
    conn: &GattConnection<'a, 'b, DefaultPacketPool>,
    stack: &Stack<'_, C, DefaultPacketPool>,
    active_bond_info: Option<crate::ble::profile::ProfileInfo>,
    config: &BleBatteryConfig<'a>,
    #[cfg(feature = "host")] host_service: Option<&'r crate::host::HostService<'r>>,
) {
    let mut ble_hid_server = BleHidServer::new(server, conn);
    let mut ble_led_reader = BleLedReader;
    let mut ble_battery_server = config.enabled.then(|| BleBatteryServer::new(server, conn));
    #[cfg(feature = "split")]
    let mut ble_peripheral_battery_server = crate::SPLIT_BATTERY_PERIPHERAL_IDS
        .first()
        .map(|_| BlePeripheralBatteryServer::new(server, conn));

    // CCCD lookup uses cached bond info to avoid a cancellable flash read while
    // this future is racing other arms of an outer `select`.
    if let Some(bond_info) = active_bond_info
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
    let dongle_link = crate::state::current_profile() == crate::ble::profile::DONGLE_PROFILE;
    #[cfg(not(feature = "dongle"))]
    let dongle_link = false;
    let host_phy = if cfg!(feature = "use_1m_phy") && !dongle_link {
        PhyKind::Le1M
    } else {
        PhyKind::Le2M
    };
    update_ble_phy(stack, conn.raw(), host_phy).await;

    #[cfg(not(feature = "split"))]
    let battery_task = ble_battery_server.run();
    #[cfg(feature = "split")]
    let battery_task = embassy_futures::join::join(ble_battery_server.run(), ble_peripheral_battery_server.run());

    let communication_task = async {
        if let Either3::First(e) = select3(
            gatt_events_task(server, conn),
            set_conn_params(stack, conn),
            battery_task,
        )
        .await
        {
            error!("[gatt_events_task] end: {:?}", e)
        }
    };

    let writer_task = async {
        loop {
            let report = BLE_REPORT_CHANNEL.receive().await;
            if let Err(e) = ble_hid_server.write_report(&report).await {
                error!("Failed to send report: {:?}", e);
            }
        }
    };

    let led_task = run_led_reader(&mut ble_led_reader, ConnectionType::Ble);

    #[cfg(feature = "host")]
    let host_task = async {
        if let Some(service) = host_service {
            // Restart after a session-fatal TX error so Rynk survives the rest
            // of the connection.
            loop {
                HostGattHandler::run(server, conn, service).await;
            }
        } else {
            core::future::pending::<()>().await;
        }
    };
    #[cfg(not(feature = "host"))]
    let host_task = core::future::pending::<()>();

    let inner = join3(writer_task, led_task, host_task);
    select(communication_task, inner).await;
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

/// Update the subrate factor.
///
/// Returns whether the request reached the controller, so callers that mirror
/// the parameters in their own state don't record params that never landed.
#[cfg(feature = "subrating")]
pub(crate) async fn update_subrate_factor<C: Controller + ControllerCmdAsync<LeSubrateRequest>, P: PacketPool>(
    stack: &Stack<'_, C, P>,
    params: LeSubrateRequestParams,
) -> bool {
    for _ in 0..10 {
        let subrate_request = LeSubrateRequest::from(params);
        match stack.async_command(subrate_request).await {
            Ok(_) => return true,
            Err(BleHostError::BleHost(Error::Hci(error))) => {
                if 0x3A == error.to_status().into_inner() {
                    info!("[update_subrate_factor] HCI busy: {:?}", error);
                    embassy_time::Timer::after_millis(100).await;
                    continue;
                }
                error!("[update_subrate_factor] HCI error: {:?}", error);
                return false;
            }
            Err(e) => {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                error!("[update_subrate_factor] BLE host error: {:?}", e);
                return false;
            }
        }
    }
    warn!("[update_subrate_factor] controller stayed busy, giving up");
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

    fn ble_status_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn set_ble_state_preserves_current_profile() {
        let _guard = ble_status_test_lock().lock().unwrap();

        set_ble_profile(2);
        set_ble_state(BleState::Advertising);

        assert_eq!(
            current_ble_status(),
            BleStatus {
                profile: 2,
                state: BleState::Advertising,
            }
        );
    }

    #[test]
    fn set_ble_profile_resets_state_when_profile_changes() {
        let _guard = ble_status_test_lock().lock().unwrap();

        set_ble_profile(1);
        set_ble_state(BleState::Connected);
        set_ble_profile(3);

        assert_eq!(
            current_ble_status(),
            BleStatus {
                profile: 3,
                state: BleState::Inactive,
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
