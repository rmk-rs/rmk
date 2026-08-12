use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use embassy_futures::join::join3;
use embassy_futures::select::{Either, Either3, select, select3};
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer, with_timeout};
use rmk_types::ble::BleState;
use rmk_types::connection::ConnectionType;
use rmk_types::led_indicator::LedIndicator;
use trouble_host::prelude::appearance::human_interface_device::KEYBOARD;
use trouble_host::prelude::service::{BATTERY, HUMAN_INTERFACE_DEVICE};
use trouble_host::prelude::*;

use crate::RawMutex;
use crate::ble::battery_service::BleBatteryServer;
use crate::ble::ble_server::{BleHidServer, Server};
use crate::ble::device_info::{PnPID, VidSource};
#[cfg(feature = "host")]
use crate::ble::host::{HOST_WRITE_BUFFER_SIZE, HostGattHandler, HostWriteOutcome};
use crate::ble::led::BleLedReader;
#[cfg(feature = "passkey_entry")]
use crate::ble::passkey::{PasskeyInputState, next_gatt_event};
use crate::ble::profile::{ProfileInfo, ProfileManager, UPDATED_CCCD_TABLE, UPDATED_PROFILE};
use crate::ble::sleep::{report_activity, request_sleep};
use crate::channel::{BLE_REPORT_CHANNEL, LED_SIGNAL};
use crate::config::{BleBatteryConfig, DeviceConfig, RmkConfig};
use crate::core_traits::Runnable;
use crate::event::SubscribableEvent;
use crate::hid::{HidWriterTrait, run_led_reader};
#[cfg(feature = "split")]
use crate::split::PeripheralMatrixConfig;
#[cfg(feature = "split")]
use crate::split::ble::central::run_ble_peripheral_manager;
use crate::state::set_ble_state;

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
pub(crate) mod sleep;

/// Max number of connections of a keyboard's BLE stack.
const CONNECTIONS_MAX: usize = crate::SPLIT_PERIPHERALS_NUM + 1;

/// Max number of L2CAP channels
const L2CAP_CHANNELS_MAX: usize = CONNECTIONS_MAX * 4; // Signal + att + smp + hid

const BLE_TRANSPORT_ENABLED: u8 = 0;
const BLE_TRANSPORT_PAUSING: u8 = 1;
const BLE_TRANSPORT_PAUSED: u8 = 2;

/// Control handle for pausing and resuming a non-split [`BleTransport`].
///
/// Allocate this separately from the transport (normally as a `static`) and
/// attach it with [`BleTransport::with_control`]. [`pause`](Self::pause) does
/// not cancel the trouble-host runner: it waits until RMK has dropped an active
/// advertiser, or requested and observed disconnection of an active host, and
/// parked the host connection loop. [`resume`](Self::resume) lets that same
/// stack advertise again.
///
/// These methods require `BleTransport::run` to be polled concurrently. A
/// single task should serialize pause/resume requests.
pub struct BleTransportControl {
    enabled: AtomicBool,
    phase: AtomicU8,
    request_changed: Signal<RawMutex, ()>,
    phase_changed: Signal<RawMutex, ()>,
}

impl BleTransportControl {
    /// Create an enabled BLE transport control.
    pub const fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            phase: AtomicU8::new(BLE_TRANSPORT_ENABLED),
            request_changed: Signal::new(),
            phase_changed: Signal::new(),
        }
    }

    /// Create a control that starts paused before the BLE runner is first polled.
    ///
    /// This is useful when the application restores another radio transport at
    /// boot and must prevent a transient BLE advertisement before it calls
    /// [`resume`](Self::resume).
    pub const fn new_paused() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            phase: AtomicU8::new(BLE_TRANSPORT_PAUSING),
            request_changed: Signal::new(),
            phase_changed: Signal::new(),
        }
    }

    /// Pause host advertising/connections without stopping the BLE stack runner.
    pub async fn pause(&self) {
        self.enabled.store(false, Ordering::Release);
        self.request_changed.signal(());
        self.wait_for_phase(BLE_TRANSPORT_PAUSED).await;
    }

    /// Resume host advertising on the existing BLE stack.
    pub async fn resume(&self) {
        self.enabled.store(true, Ordering::Release);
        self.request_changed.signal(());
        self.wait_for_phase(BLE_TRANSPORT_ENABLED).await;
    }

    async fn wait_for_phase(&self, phase: u8) {
        while self.phase.load(Ordering::Acquire) != phase {
            self.phase_changed.wait().await;
        }
    }

    async fn wait_for_pause_request(&self) {
        while self.enabled.load(Ordering::Acquire) {
            self.request_changed.wait().await;
        }
        self.set_phase(BLE_TRANSPORT_PAUSING);
    }

    fn pause_requested(&self) -> bool {
        !self.enabled.load(Ordering::Acquire)
    }

    fn acknowledge_paused(&self) {
        self.set_phase(BLE_TRANSPORT_PAUSED);
    }

    async fn wait_for_resume_request(&self) {
        while !self.enabled.load(Ordering::Acquire) {
            self.request_changed.wait().await;
        }
    }

    fn acknowledge_enabled(&self) {
        self.set_phase(BLE_TRANSPORT_ENABLED);
    }

    fn set_phase(&self, phase: u8) {
        if self.phase.swap(phase, Ordering::AcqRel) != phase {
            self.phase_changed.signal(());
        }
    }
}

impl Default for BleTransportControl {
    fn default() -> Self {
        Self::new()
    }
}

async fn wait_for_pause_request(control: Option<&BleTransportControl>) {
    match control {
        Some(control) => control.wait_for_pause_request().await,
        None => core::future::pending().await,
    }
}

/// BLE transport. Owns the whole BLE stack.
///
/// On a split build the transport is the BLE split central:
/// `run` also loads the peripherals' stored addresses and drives the
/// peripheral managers and scanner on the same stack.
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
    #[cfg(not(feature = "split"))]
    control: Option<&'a BleTransportControl>,
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
            #[cfg(not(feature = "split"))]
            control: None,
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

    /// Attach an application-owned pause/resume control handle.
    #[cfg(not(feature = "split"))]
    pub fn with_control(mut self, control: &'a BleTransportControl) -> Self {
        self.control = Some(control);
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
        serve(
            &stack,
            &self.device_config,
            &self.config,
            #[cfg(feature = "host")]
            self.host_service,
            self.control,
        )
        .await
    }
}

#[cfg(feature = "split")]
impl<'a, C> Runnable for BleTransport<'a, C>
where
    C: Controller
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<bt_hci::cmd::le::LeSetScanParams>,
{
    async fn run(&mut self) -> ! {
        // Load the preferred connection from storage
        let preferred = crate::state::load_preferred_connection().await;
        crate::state::set_preferred_connection(preferred);

        let controller = self.controller.take().expect("BleTransport::run called twice");

        // Load the peripherals' stored addresses through the storage task,
        // the same way a peripheral loads its central's address. The scanner
        // and the managers then share the slots; `Cell` is enough because a
        // slot is `Copy`.
        let mut addrs = [None; crate::SPLIT_PERIPHERALS_NUM];
        for (id, slot) in addrs.iter_mut().enumerate() {
            *slot = crate::storage::read_peer_address(id as u8)
                .await
                .filter(|peer| peer.is_valid)
                .map(|peer| peer.address);
        }
        let addrs = addrs.map(core::cell::Cell::new);

        let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> = HostResources::new();
        let stack = trouble_host::new(controller, &mut resources)
            .set_random_address(Address::random(self.address))
            .build();

        let managers =
            embassy_futures::join::join_array(core::array::from_fn::<_, { crate::SPLIT_PERIPHERALS_NUM }, _>(|i| {
                run_ble_peripheral_manager(i, &addrs[i], &stack, self.peripheral_matrices[i])
            }));
        join3(
            serve(
                &stack,
                &self.device_config,
                &self.config,
                #[cfg(feature = "host")]
                self.host_service,
            ),
            managers,
            crate::split::ble::central::scan_peripherals(&stack, &addrs),
        )
        .await;
        unreachable!("BleTransport sub-tasks must run forever")
    }
}

/// Advertise→connect→serve forever, joined with the stack runner and the
/// sleep manager. Builds and owns the GATT server.
async fn serve<#[cfg(feature = "host")] 'r, C>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    device_config: &DeviceConfig<'static>,
    config: &BleBatteryConfig<'static>,
    #[cfg(feature = "host")] host_service: Option<&'r crate::host::HostService<'r>>,
    #[cfg(not(feature = "split"))] control: Option<&BleTransportControl>,
) -> !
where
    C: Controller + ControllerCmdAsync<LeSetPhy> + ControllerCmdSync<LeReadLocalSupportedFeatures>,
{
    #[cfg(feature = "split")]
    let control: Option<&BleTransportControl> = None;

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
    let mut profile_manager = ProfileManager::new(stack);
    // Load the bonded devices from storage
    profile_manager.load_bonded_devices().await;
    profile_manager.update_stack_bonds();

    let mut peripheral = stack.peripheral();
    let runner = stack.runner();

    let profile_manager = &mut profile_manager;

    let connection_loop = async {
        loop {
            if let Some(control) = control
                && control.pause_requested()
            {
                set_ble_state(BleState::Inactive);
                control.acknowledge_paused();
                loop {
                    match select(control.wait_for_resume_request(), profile_manager.update_profile()).await {
                        Either::First(()) => break,
                        Either::Second(()) => {}
                    }
                }
                control.acknowledge_enabled();
            } else if let Some(control) = control {
                control.acknowledge_enabled();
            }

            match select3(
                advertise(product_name, &mut peripheral, server),
                profile_manager.update_profile(),
                wait_for_pause_request(control),
            )
            .await
            {
                Either3::First(Ok(conn)) => {
                    // Do NOT emit BleState::Connected here. gatt_events_task emits
                    // Connected when it sees GattConnectionEvent::Encrypted.
                    let active_bond_info = profile_manager.active_bond_info();
                    // Check the bond info after the connection is just created.
                    if let Some(bond) = &active_bond_info
                        && !bond.info.identity.match_identity(&conn.raw().peer_identity())
                    {
                        warn!("[ble] connected peer doesn't match the active profile, disconnecting");
                        conn.raw().disconnect();
                        loop {
                            if let GattConnectionEvent::Disconnected { .. } = conn.next().await {
                                break;
                            }
                        }
                        continue;
                    }
                    let should_disconnect = match select3(
                        run_ble_keyboard(
                            server,
                            &conn,
                            stack,
                            active_bond_info,
                            config,
                            #[cfg(feature = "host")]
                            host_service,
                        ),
                        profile_manager.update_profile(),
                        wait_for_pause_request(control),
                    )
                    .await
                    {
                        Either3::First(_) => false,
                        Either3::Second(_) | Either3::Third(_) => true,
                    };
                    if should_disconnect {
                        // When the profile changes, manually disconnect from the current host
                        if conn.raw().is_connected() {
                            conn.raw().disconnect();
                            loop {
                                if let GattConnectionEvent::Disconnected { .. } = conn.next().await {
                                    break;
                                }
                            }
                        }
                    }
                }
                Either3::First(Err(BleHostError::BleHost(Error::Timeout))) => {
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
                    match select3(
                        key_wake.next_message_pure(),
                        pointing_wake.next_message_pure(),
                        wait_for_pause_request(control),
                    )
                    .await
                    {
                        Either3::First(_) | Either3::Second(_) => report_activity(),
                        Either3::Third(_) => {}
                    }
                }
                Either3::First(Err(e)) => {
                    #[cfg(feature = "defmt")]
                    let e = defmt::Debug2Format(&e);
                    error!("Advertise error: {:?}", e);
                    Timer::after_millis(200).await;
                }
                Either3::Second(()) | Either3::Third(()) => {}
            };

            if control.is_some_and(|control| !control.enabled.load(Ordering::Acquire)) {
                set_ble_state(BleState::Inactive);
                continue;
            }

            // Skip the Inactive transition if we never moved off Advertising
            if crate::state::current_ble_status().state != BleState::Advertising {
                set_ble_state(BleState::Inactive);
            }
        }
    };

    #[cfg(feature = "split")]
    let event_handler = {
        // Latched before the runner starts so peripheral scanning can proceed
        // as soon as `join3` polls it.
        crate::split::ble::central::STACK_STARTED.signal(true);
        crate::split::ble::central::ScanHandler {}
    };
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

/// NoopHandler is used on the device which never scans,
/// such as a split peripheral or a normal keyboard.
pub(crate) struct NoopHandler;

impl EventHandler for NoopHandler {}

/// This is a background task that is required to run forever alongside any other BLE tasks.
pub(crate) async fn ble_task<C: Controller + ControllerCmdAsync<LeSetPhy>, P: PacketPool, E: EventHandler>(
    mut runner: Runner<'_, C, P>,
    handler: &E,
) {
    loop {
        if let Err(e) = runner.run_with_handler(handler).await {
            error!("[ble_task] runner error: {:?}", e);
            embassy_time::Timer::after_millis(100).await;
        }
    }
}

/// Stream Events until the connection closes.
///
/// This function will handle the GATT events and process them.
/// This is how we interact with read and write requests.
async fn gatt_events_task(server: &Server<'_>, conn: &GattConnection<'_, '_, DefaultPacketPool>) -> Result<(), Error> {
    let level = server.battery_service.level;
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
                            debug!("Read GATT Event to Unknown: {:?}", event.handle());
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

/// Create an advertiser to use to connect to a BLE Central, and wait for it to connect.
async fn advertise<'a, 'b, C: Controller>(
    name: &'a str,
    peripheral: &mut Peripheral<'a, C, DefaultPacketPool>,
    server: &'b Server<'_>,
) -> Result<GattConnection<'a, 'b, DefaultPacketPool>, BleHostError<C::Error>> {
    // Wait for 10ms to ensure the USB is checked
    embassy_time::Timer::after_millis(10).await;
    let mut advertiser_data = [0; 31];
    AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::CompleteServiceUuids16(&[BATTERY.to_le_bytes(), HUMAN_INTERFACE_DEVICE.to_le_bytes()]),
            AdStructure::CompleteLocalName(name.as_bytes()),
            AdStructure::Unknown {
                ty: 0x19, // Appearance
                data: &KEYBOARD.to_le_bytes(),
            },
        ],
        &mut advertiser_data[..],
    )?;
    let advertisement = Advertisement::ConnectableScannableUndirected {
        adv_data: &advertiser_data[..],
        scan_data: &[],
    };

    let advertise_config = AdvertisementParameters {
        primary_phy: PhyKind::Le2M,
        secondary_phy: PhyKind::Le2M,
        tx_power: TxPower::Plus8dBm,
        interval_min: Duration::from_millis(200),
        interval_max: Duration::from_millis(200),
        ..Default::default()
    };

    info!("[adv] advertising");
    set_ble_state(BleState::Advertising);
    let advertiser = peripheral.advertise(&advertise_config, advertisement).await?;

    // Timeout for advertising is 300s
    match with_timeout(Duration::from_secs(300), advertiser.accept()).await {
        Ok(conn_res) => {
            let conn = conn_res?.with_attribute_server(server)?;
            info!("[adv] connection established");
            if let Err(e) = conn.raw().set_bondable(true) {
                error!("Set bondable error: {:?}", e);
            };
            Ok(conn)
        }
        Err(_) => Err(BleHostError::BleHost(Error::Timeout)),
    }
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
    // Wait for 5 seconds before setting connection parameters to avoid connection drop
    embassy_time::Timer::after_secs(5).await;

    // For macOS/iOS(aka Apple devices), both interval should be set to 15ms
    // Reference: https://developer.apple.com/accessories/Accessory-Design-Guidelines.pdf
    update_conn_params(
        stack,
        conn.raw(),
        &RequestedConnParams {
            min_connection_interval: Duration::from_millis(15),
            max_connection_interval: Duration::from_millis(15),
            max_latency: 30,
            min_event_length: Duration::from_secs(0),
            max_event_length: Duration::from_secs(0),
            supervision_timeout: Duration::from_secs(10),
        },
    )
    .await;

    embassy_time::Timer::after_secs(5).await;

    // Setting the conn param the second time ensures that we have best performance on all platforms
    update_conn_params(
        stack,
        conn.raw(),
        &RequestedConnParams {
            min_connection_interval: Duration::from_micros(7500),
            max_connection_interval: Duration::from_micros(7500),
            max_latency: 30,
            min_event_length: Duration::from_secs(0),
            max_event_length: Duration::from_secs(0),
            supervision_timeout: Duration::from_secs(10),
        },
    )
    .await;

    // Wait forever. This is because we want the conn params setting can be interrupted when the connection is lost.
    // So this task shouldn't quit after setting the conn params.
    core::future::pending::<()>().await;
}

/// Run BLE keyboard for one connection.
///
/// Returns when the GATT events task ends (i.e. the connection drops).
/// `writer_task`, `led_task`, and `host_task` are all infinite, so the outer
/// `select(communication_task, inner)` cancels them as a side-effect of
/// `communication_task` returning. `inner` itself never completes.
async fn run_ble_keyboard<
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

    let host_phy = if cfg!(feature = "use_1m_phy") {
        PhyKind::Le1M
    } else {
        PhyKind::Le2M
    };
    update_ble_phy(stack, conn.raw(), host_phy).await;

    let communication_task = async {
        if let Either3::First(e) = select3(
            gatt_events_task(server, conn),
            set_conn_params(stack, conn),
            ble_battery_server.run(),
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

    let inner = embassy_futures::join::join3(writer_task, led_task, host_task);
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
                if 0x2A == error.to_status().into_inner() {
                    // Busy, retry
                    info!("[update_ble_phy] HCI busy: {:?}", error);
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
                if 0x3A == error.to_status().into_inner() {
                    // Busy, retry
                    info!("[update_conn_params] HCI busy: {:?}", error);
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
    use core::sync::atomic::Ordering;
    use std::sync::{Mutex, OnceLock};

    use embassy_futures::join::join;
    use embassy_futures::select::select;
    use embassy_time::Timer;
    use rmk_types::ble::{BleState, BleStatus};

    use crate::event::{Axis, AxisEvent, AxisValType, KeyboardEvent, PointingEvent, SubscribableEvent, publish_event};
    use crate::state::{current_ble_status, set_ble_profile, set_ble_state};
    use crate::test_support::test_block_on as block_on;

    use super::{BLE_TRANSPORT_PAUSED, BLE_TRANSPORT_PAUSING, BleTransportControl};

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
    fn transport_control_pauses_and_resumes_the_same_runner() {
        let control = BleTransportControl::new();

        block_on(async {
            join(
                async {
                    control.pause().await;
                    assert_eq!(control.phase.load(Ordering::Acquire), BLE_TRANSPORT_PAUSED);
                    // Requests are idempotent once the phase is acknowledged.
                    control.pause().await;

                    control.resume().await;
                    control.resume().await;
                },
                async {
                    control.wait_for_pause_request().await;
                    assert_eq!(control.phase.load(Ordering::Acquire), BLE_TRANSPORT_PAUSING);
                    control.acknowledge_paused();
                    control.wait_for_resume_request().await;
                    control.acknowledge_enabled();
                },
            )
            .await;
        });
    }

    #[test]
    fn initially_paused_control_does_not_require_an_advertising_cycle() {
        let control = BleTransportControl::new_paused();
        assert_eq!(control.phase.load(Ordering::Acquire), BLE_TRANSPORT_PAUSING);

        block_on(async {
            join(
                async {
                    control.pause().await;
                    assert_eq!(control.phase.load(Ordering::Acquire), BLE_TRANSPORT_PAUSED);
                    control.resume().await;
                },
                async {
                    control.wait_for_pause_request().await;
                    control.acknowledge_paused();
                    control.wait_for_resume_request().await;
                    control.acknowledge_enabled();
                },
            )
            .await;
        });

        assert_eq!(control.phase.load(Ordering::Acquire), super::BLE_TRANSPORT_ENABLED);
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
