//! Dongle firmware (`dongle` feature): a BLE central that relays one bonded
//! RMK keyboard to a USB host.
//!
//! Toward the keyboard it is a HID-over-GATT client; for everything else it is
//! a relay. The keyboard's host protocol (Rynk frames, or Vial reports with the
//! `vial` feature) passes through unparsed in both directions, so the dongle
//! answers no command of its own and never tracks the protocol. Keymaps and
//! storage stay on the keyboard; the dongle stores only one bond.
//!
//! Two tasks, joined by [`Dongle::run`]:
//! - `ble_task`: the trouble runner, with the scan handler below;
//! - [`DongleCentral::run`]: find a keyboard, connect, secure, relay, repeat.

#[cfg(not(feature = "vial"))]
mod router;
#[cfg(feature = "vial")]
mod vial_router;
use core::cell::Cell;

use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy, LeSetScanParams};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use bt_hci::param::{AddrKind, BdAddr, LeAdvEventKind, Status};
use embassy_futures::join::join;
use embassy_futures::select::{Either, select, select3};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, with_deadline, with_timeout};
#[cfg(not(feature = "vial"))]
use rmk_types::protocol::rynk::{RYNK_BLE_CHUNK_SIZE, RYNK_INPUT_CHAR_UUID, RYNK_OUTPUT_CHAR_UUID, RYNK_SERVICE_UUID};
pub use router::DongleRouter;
#[cfg(feature = "vial")]
use router::VialReport;
use trouble_host::prelude::*;
use usbd_hid::descriptor::{MediaKeyboardReport, MouseReport, SystemControlReport};
#[cfg(feature = "vial")]
use vial_router as router;

use crate::ble::adv::Adv;
use crate::ble::profile::{ProfileInfo, ProfileManager};
use crate::ble::scan::{DONGLE_SCAN_WINDOW, scan_config, start_scan};
use crate::ble::{update_ble_phy, update_conn_params, wait_for_stack_started};
use crate::channel::send_hid_report;
use crate::core_traits::Runnable;
use crate::event::{EventSubscriber, LedIndicatorEvent, SubscribableEvent};
use crate::hid::{KeyboardReport, Report};
use crate::{DONGLE_PAIRING_WINDOW_SECS, RawMutex};

/// The dongle relays exactly one keyboard.
const DONGLE_CONNECTIONS_MAX: usize = 1;

/// trouble's `CHANNELS` counts only dynamic L2CAP channels (fixed
/// signalling/ATT/SMP are free); the dongle only talks GATT, so zero.
const DONGLE_L2CAP_CHANNELS_MAX: usize = 0;

/// The trailing `1, 2` are `ADV_SETS` (its default) and `BONDS`: the old bond
/// is pruned only after a new pairing succeeds, so replacing the keyboard
/// briefly holds two — one slot would lose the new bond on reboot.
type DongleBleResources = HostResources<DefaultPacketPool, DONGLE_CONNECTIONS_MAX, DONGLE_L2CAP_CHANNELS_MAX, 1, 2>;

/// What the 3 covers: the 0x1812 query matches both the report service and the
/// host-protocol HID service (rynk's or vial's), plus rynk's custom service.
type Client<'a, C> = GattClient<'a, C, DefaultPacketPool, 3>;

const BOND_SLOT: u8 = 0;

/// Which keyboard a connection is to. Decides whether the link pairs or
/// encrypts, and whether a rejected key means the stored bond is dead.
#[derive(Clone, Copy, PartialEq)]
enum Peer {
    /// The keyboard this dongle already holds a bond for.
    Bonded,
    /// A keyboard from a pairing window; it replaces that bond.
    New,
}

/// What the scan handler tells [`DongleCentral`]. Inside the BLE runner it has
/// no access to bonds or profiles, so it only reports; the dongle task decides.
struct ScanHandler {
    /// The bonded keyboard's address to watch; [`DongleCentral::run`] updates it
    /// whenever it re-reads the bond.
    bonded_addr: BlockingMutex<RawMutex, Cell<Option<BdAddr>>>,
    /// The most recent keyboard seen seeking a dongle, and its RSSI.
    seeking_keyboard: Signal<RawMutex, ((AddrKind, BdAddr), i8)>,
    /// The bonded keyboard was seen advertising (anything but seeking). This
    /// blocks adopting a replacement; it never triggers a connect.
    bonded_seen: Signal<RawMutex, ()>,
    /// The bonded keyboard asked for the dongle: a directed advertisement, or
    /// seeking again after clearing its bond. Only this triggers a connect.
    bonded_asked: Signal<RawMutex, ()>,
}

impl ScanHandler {
    fn new() -> Self {
        Self {
            bonded_addr: BlockingMutex::new(Cell::new(None)),
            seeking_keyboard: Signal::new(),
            bonded_seen: Signal::new(),
            bonded_asked: Signal::new(),
        }
    }
}

impl EventHandler for ScanHandler {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = it.next() {
            let bonded = self.bonded_addr.lock(|addr| addr.get()) == Some(report.addr);
            if Adv::decode(report.data) == Some(Adv::DongleSeeking) {
                debug!("[dongle] seeking keyboard {:?} rssi {}", report.addr, report.rssi);
                self.seeking_keyboard
                    .signal(((report.addr_kind, report.addr), report.rssi));
                // Seeking again means it cleared its bond; the reconnect will re-pair.
                if bonded {
                    self.bonded_asked.signal(());
                }
            } else if bonded {
                self.bonded_seen.signal(());
                // Directed advertisements are only reported to their target: us.
                if report.event_kind == LeAdvEventKind::AdvDirectInd {
                    self.bonded_asked.signal(());
                }
            }
        }
    }
}

/// The dongle runnable. It owns and sizes its own BLE stack — the keyboard
/// side's [`crate::ble::BleTransport`] is not involved, so one build can carry
/// both kinds of binaries. The USB side is a normal [`crate::usb::UsbTransport`]
/// serving the same [`DongleRouter`].
pub struct Dongle<'a, C> {
    /// Taken by `run`, which owns the stack and its resources.
    controller: Option<C>,
    address: [u8; 6],
    router: &'a DongleRouter,
}

impl<'a, C> Dongle<'a, C> {
    /// `router` must be the one this binary's [`crate::usb::UsbTransport`] serves.
    pub fn new(controller: C, address: [u8; 6], router: &'a DongleRouter) -> Self {
        Self {
            controller: Some(controller),
            address,
            router,
        }
    }
}

impl<C> Runnable for Dongle<'_, C>
where
    C: Controller
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeSetScanParams>,
{
    async fn run(&mut self) -> ! {
        let controller = self.controller.take().expect("Dongle::run called twice");
        let mut resources = DongleBleResources::new();
        let stack = trouble_host::new(controller, &mut resources)
            .set_random_address(Address::random(self.address))
            .build();
        let stack = &stack;
        let scan = ScanHandler::new();
        let mut central = DongleCentral {
            stack,
            scan: &scan,
            router: self.router,
            profiles: ProfileManager::new(stack),
        };

        join(crate::ble::ble_task(stack.runner(), &scan), central.run()).await;
        unreachable!("Dongle sub-tasks must run forever")
    }
}

/// The dongle's BLE central: the state that outlives one connection.
/// Per-connection state — the link, its GATT client — is passed as arguments.
struct DongleCentral<'b, 's: 'b, C: Controller + ControllerCmdAsync<LeSetPhy>> {
    stack: &'b Stack<'s, C, DefaultPacketPool>,
    scan: &'b ScanHandler,
    router: &'b DongleRouter,
    profiles: ProfileManager<'b, 's, C, DefaultPacketPool, 1>,
}

impl<'b, 's: 'b, C> DongleCentral<'b, 's, C>
where
    C: Controller
        + ControllerCmdAsync<LeSetPhy>
        + ControllerCmdSync<LeReadLocalSupportedFeatures>
        + ControllerCmdSync<LeSetScanParams>,
{
    async fn run(&mut self) -> ! {
        wait_for_stack_started().await;
        self.profiles.load_bonded_devices().await;
        self.profiles.update_stack_bonds();

        let bonded = self.profiles.active_bond_info().map(|b| b.info.identity.addr);
        self.scan.bonded_addr.lock(|a| a.set(bonded.map(|addr| addr.addr)));
        // The only chance to pair while a bond exists: one window at boot, so a
        // keyboard the user already set seeking can replace the bonded one.
        if bonded.is_some()
            && let Some((kind, addr)) = self.run_pairing_window().await
            && let Some(conn) = self.connect(Address { kind, addr }).await
        {
            self.run_connection(conn, Peer::New).await;
        }

        loop {
            let bonded = self.profiles.active_bond_info().map(|b| b.info.identity.addr);
            self.scan.bonded_addr.lock(|a| a.set(bonded.map(|addr| addr.addr)));

            if let Some(addr) = bonded {
                // Connect only once the keyboard asks for the dongle; its bare address
                // would match its host advertising too. The accept list drops other traffic.
                self.scan.bonded_asked.reset();
                let session = start_scan(self.stack, DONGLE_SCAN_WINDOW, &[addr]).await;
                self.scan.bonded_asked.wait().await;
                session.stop().await;
                if let Some(conn) = self.connect(addr).await {
                    self.run_connection(conn, Peer::Bonded).await;
                }
            } else if let Some((kind, addr)) = self.run_pairing_window().await
                && let Some(conn) = self.connect(Address { kind, addr }).await
            {
                self.run_connection(conn, Peer::New).await;
            }
            Timer::after_millis(500).await;
        }
    }

    async fn connect(&self, address: Address) -> Option<Connection<'b, DefaultPacketPool>> {
        let mut central = self.stack.central();

        let config = ConnectConfig {
            // Relay interval, but zero latency and a longer supervision timeout
            // while pairing and discovery run; the relay settings come after.
            connect_params: RequestedConnParams {
                max_latency: 0,
                supervision_timeout: Duration::from_secs(30),
                ..relay_conn_params()
            },
            scan_config: ScanConfig {
                filter_accept_list: &[address],
                ..scan_config(DONGLE_SCAN_WINDOW)
            },
        };
        // If the keyboard is absent the attempt times out; the caller's loop retries.
        match with_timeout(Duration::from_secs(15), central.connect(&config)).await {
            Ok(Ok(conn)) => Some(conn),
            Ok(Err(e)) => {
                #[cfg(feature = "defmt")]
                let e = defmt::Debug2Format(&e);
                debug!("[dongle] connect error: {:?}", e);
                None
            }
            Err(_) => None,
        }
    }

    /// Scan for keyboards seeking a dongle; return the strongest seen within 2s
    /// of the first. Whichever shows up first wins: the bonded keyboard ends the
    /// window with `None`, while a seeker seen before it was set seeking by the
    /// user before the (re)plug — that beats the automatic reconnect.
    async fn run_pairing_window(&self) -> Option<(AddrKind, BdAddr)> {
        info!("[dongle] pairing window open for {}s", DONGLE_PAIRING_WINDOW_SECS);
        self.scan.seeking_keyboard.reset();
        self.scan.bonded_seen.reset();
        let deadline = Instant::now() + Duration::from_secs(DONGLE_PAIRING_WINDOW_SECS as u64);

        let pick = async {
            let (mut best_addr, mut best_rssi) = self.scan.seeking_keyboard.wait().await;
            // Don't wait out the whole window: collect for 2s after the first one.
            let gather = Instant::now() + Duration::from_secs(2);
            while let Ok((addr, rssi)) = with_deadline(gather, self.scan.seeking_keyboard.wait()).await {
                if rssi > best_rssi {
                    (best_addr, best_rssi) = (addr, rssi);
                }
            }
            (best_addr, best_rssi)
        };

        let Ok(session) = with_deadline(deadline, start_scan(self.stack, DONGLE_SCAN_WINDOW, &[])).await else {
            info!("[dongle] pairing window closed, the scanner never started");
            return None;
        };
        let found = match with_deadline(deadline, select(pick, self.scan.bonded_seen.wait())).await {
            Ok(Either::First((addr, rssi))) => {
                info!("[dongle] pairing candidate {:?} (rssi {})", addr.1, rssi);
                Some(addr)
            }
            Ok(Either::Second(())) => {
                debug!("[dongle] bonded keyboard is alive, closing the pairing window");
                None
            }
            Err(_) => {
                info!("[dongle] pairing window closed, no keyboard found");
                None
            }
        };
        // The connect that follows fails unless the controller has fully stopped scanning.
        session.stop().await;
        found
    }

    /// One connection from start to end: secure it, relay over it, and release
    /// everything still held on the host when it drops.
    async fn run_connection(&mut self, conn: Connection<'b, DefaultPacketPool>, peer: Peer) {
        if !self.secure_connection(&conn, peer).await {
            info!("[dongle] securing failed");
        } else if let Ok(client) = Client::new(self.stack, &conn).await {
            // The client task receives notifications and the watcher drains connection
            // events; the relay runs beside them and ends when either one ends.
            select3(
                client.task(),
                self.watch_connection(&conn),
                self.discover_and_relay(&conn, &client),
            )
            .await;

            self.router.link_down();
            release_held_keys().await;
            info!("[dongle] disconnected");
        }
        conn.disconnect();
    }

    /// Keep draining connection events for the connection's whole life: a full
    /// queue drops new events, including the disconnect this returns on.
    async fn watch_connection(&self, conn: &Connection<'_, DefaultPacketPool>) {
        loop {
            match conn.next().await {
                ConnectionEvent::Disconnected { .. } => return,
                ConnectionEvent::RequestConnectionParams(req) => self.accept_conn_params(req).await,
                _ => {}
            }
        }
    }

    async fn accept_conn_params(&self, req: ConnectionParamsRequest) {
        if let Err(e) = req.accept(None, self.stack).await {
            debug!("[dongle] conn param accept error: {:?}", e);
        }
    }

    /// Pair (a new keyboard) or encrypt (the bonded one), then wait for the link
    /// to report it is secure.
    async fn secure_connection(&mut self, conn: &Connection<'_, DefaultPacketPool>, peer: Peer) -> bool {
        // Both sides must be bondable, or neither side gets the bond data and the
        // keyboard would re-pair on every reconnect. Must come before `request_security`.
        if let Err(e) = conn.set_bondable(true) {
            warn!("[dongle] set_bondable error: {:?}", e);
            return false;
        }
        if let Err(e) = conn.request_security() {
            warn!("[dongle] request_security error: {:?}", e);
            return false;
        }
        loop {
            match with_timeout(Duration::from_secs(30), conn.next()).await {
                // Save the new bond — also when the bonded keyboard cleared its side
                // and re-paired, otherwise we keep an old key nothing accepts anymore.
                Ok(ConnectionEvent::PairingComplete { bond: Some(bond), .. }) => {
                    self.profiles
                        .add_profile_info(ProfileInfo {
                            slot_num: BOND_SLOT,
                            removed: false,
                            info: bond,
                            cccd_table: heapless::Vec::new(),
                        })
                        .await;
                    return true;
                }
                Ok(ConnectionEvent::Encrypted { .. } | ConnectionEvent::PairingComplete { .. }) => return true,
                Ok(ConnectionEvent::PairingFailed(e)) => {
                    warn!("[dongle] pairing failed: {:?}", e);
                    if peer == Peer::Bonded {
                        self.profiles.clear_bond(BOND_SLOT).await;
                    }
                    return false;
                }
                // A keyboard that cleared its side rejects our key at the link layer,
                // so the rejection arrives here, not as `PairingFailed`. The stored
                // key is dead either way: drop it, and the next loop opens a pairing window.
                Ok(ConnectionEvent::Disconnected { reason }) => {
                    if peer == Peer::Bonded && reason == Status::AUTHENTICATION_FAILURE {
                        warn!("[dongle] bonded keyboard refused our key, dropping the bond");
                        self.profiles.clear_bond(BOND_SLOT).await;
                    }
                    return false;
                }
                Err(_) => return false,
                // Securing is the only event reader until it returns, so answer these here.
                Ok(ConnectionEvent::RequestConnectionParams(req)) => self.accept_conn_params(req).await,
                Ok(_) => {}
            }
        }
    }

    /// Tune the link, discover and subscribe, then relay. `None` means setup
    /// failed, which ends the connection.
    async fn discover_and_relay(&self, conn: &Connection<'_, DefaultPacketPool>, client: &Client<'_, C>) -> Option<()> {
        // Same latency setup as a split link: 2M PHY + 7.5 ms interval.
        update_ble_phy(self.stack, conn, PhyKind::Le2M).await;
        update_conn_params(self.stack, conn, &relay_conn_params()).await;

        let chars = KeyboardCharacteristics::discover(client).await?;
        chars.subscribe(client).await?;
        // One catch-all listener for every subscription — one queue, routed by handle.
        let mut listener = client.listen_all().ok()?;

        self.router.link_up();
        info!("[dongle] relaying");
        self.relay(
            #[cfg(not(feature = "vial"))]
            conn,
            client,
            &mut listener,
            &chars,
        )
        .await;
        Some(())
    }

    /// Relay both directions until this future is dropped. `NOTIF_MTU` is
    /// trouble's notification buffer size, taken from the listener type;
    /// writing the number out here would tie RMK to one trouble build.
    async fn relay<const NOTIF_MTU: usize>(
        &self,
        #[cfg(not(feature = "vial"))] conn: &Connection<'_, DefaultPacketPool>,
        client: &Client<'_, C>,
        listener: &mut NotificationListener<'_, NOTIF_MTU>,
        chars: &KeyboardCharacteristics,
    ) {
        // Largest single write to the Rynk characteristic: ATT MTU minus the
        // 3-byte write header. Vial reports are 32 bytes and are sent whole.
        #[cfg(not(feature = "vial"))]
        let chunk_size = RYNK_BLE_CHUNK_SIZE
            .min((conn.att_mtu() as usize).saturating_sub(3))
            .max(1);

        let keyboard_to_host = async {
            loop {
                let notification = listener.next().await;
                let (handle, data) = (notification.handle(), notification.as_ref());
                // The config stream is the only one not parsed; it goes straight to the host.
                if handle == chars.config_input.handle {
                    // A full pipe usually means the host is a moment behind, so wait 20ms at most.
                    #[cfg(not(feature = "vial"))]
                    if with_timeout(Duration::from_millis(20), self.router.to_host.write_all(data))
                        .await
                        .is_err()
                    {
                        // Send a frame delimiter so the host drops the cut-off frame and resyncs.
                        let _ = self.router.to_host.try_write(&[0]);
                        warn!("[dongle] host config stream overflow, dropping bytes");
                    }
                    // Each report stands alone: dropping one loses that reply, nothing else.
                    #[cfg(feature = "vial")]
                    match VialReport::try_from(data) {
                        Ok(report) => {
                            if self.router.to_host.try_send(report).is_err() {
                                warn!("[dongle] host reply queue full, dropping a reply");
                            }
                        }
                        Err(_) => warn!("[dongle] non-report-size vial notify dropped"),
                    }
                } else if let Some(report) = chars.report(handle, data) {
                    send_hid_report(report).await;
                }
            }
        };

        let led_to_keyboard = async {
            let mut led_events = LedIndicatorEvent::subscriber();
            loop {
                let event = led_events.next_event().await;
                let _ = client
                    .write_characteristic_without_response(&chars.keyboard_output, &[event.0.into_bits()])
                    .await;
            }
        };

        // One characteristic write per loop turn: Rynk reads one write's worth of
        // bytes (the read does the chunking); Vial takes one unsplittable report.
        #[cfg(not(feature = "vial"))]
        let mut request = [0u8; RYNK_BLE_CHUNK_SIZE];
        let request_to_keyboard = async {
            loop {
                #[cfg(not(feature = "vial"))]
                {
                    let n = self.router.to_keyboard.read(&mut request[..chunk_size]).await;
                    let _ = client
                        .write_characteristic_without_response(&chars.config_output, &request[..n])
                        .await;
                }
                #[cfg(feature = "vial")]
                {
                    let report = self.router.to_keyboard.receive().await;
                    let _ = client
                        .write_characteristic_without_response(&chars.config_output, &report)
                        .await;
                }
            }
        };

        // Three independent loops, not one combined one: an LED update must not
        // wait behind a config write, or the other way around.
        select3(keyboard_to_host, led_to_keyboard, request_to_keyboard).await;
    }
}

/// Link parameters while relaying: 7.5 ms interval, the same latency budget as
/// a split link. The long supervision timeout favors surviving radio noise over
/// a fast reconnect after a dongle power-cycle (rare): the keyboard starts its
/// directed reconnect advertising only after this timeout runs out.
fn relay_conn_params() -> RequestedConnParams {
    RequestedConnParams {
        min_connection_interval: Duration::from_micros(7500),
        max_connection_interval: Duration::from_micros(7500),
        max_latency: 30,
        supervision_timeout: Duration::from_secs(10),
        ..Default::default()
    }
}

/// The characteristics the relay needs on the keyboard.
struct KeyboardCharacteristics {
    keyboard_input: Characteristic<[u8]>,
    keyboard_output: Characteristic<[u8]>,
    mouse: Characteristic<[u8]>,
    media: Characteristic<[u8]>,
    system: Characteristic<[u8]>,
    /// The host-protocol pair: from rynk's custom service, or the two 32-byte
    /// report characteristics of vial's own HID service.
    config_input: Characteristic<[u8]>,
    config_output: Characteristic<[u8]>,
}

impl KeyboardCharacteristics {
    /// Discover the HID and host-protocol services. Report characteristics all
    /// share UUID 0x2A4D, but both ends are RMK, so the declaration order below
    /// is fixed and identifies them.
    async fn discover<C: Controller>(client: &Client<'_, C>) -> Option<Self> {
        let mut hid_services = client
            .services_by_uuid(&Uuid::new_short(0x1812))
            .await
            .ok()?
            .into_iter();
        let hid = hid_services.next()?;
        let report_uuid = Uuid::new_short(0x2A4D);
        // The 9 must fit every characteristic `HidService` declares, or discovery fails.
        let mut reports = client
            .characteristics::<9>(&hid)
            .await
            .ok()?
            .into_iter()
            .filter(|c| c.uuid == report_uuid);
        let keyboard_input = reports.next()?;
        let keyboard_output = reports.next()?;
        let mouse = reports.next()?;
        let media = reports.next()?;
        let system = reports.next()?;

        #[cfg(not(feature = "vial"))]
        let (config_input, config_output) = {
            let rynk = client
                .services_by_uuid(&RYNK_SERVICE_UUID.into())
                .await
                .ok()?
                .into_iter()
                .next()?;
            (
                client
                    .characteristic_by_uuid::<[u8]>(&rynk, &RYNK_INPUT_CHAR_UUID.into())
                    .await
                    .ok()?,
                client
                    .characteristic_by_uuid::<[u8]>(&rynk, &RYNK_OUTPUT_CHAR_UUID.into())
                    .await
                    .ok()?,
            )
        };
        // Vial uses the second HID service: input (notify) first, then output (write).
        #[cfg(feature = "vial")]
        let (config_input, config_output) = {
            let vial = hid_services.next()?;
            let mut reports = client
                .characteristics::<9>(&vial)
                .await
                .ok()?
                .into_iter()
                .filter(|c| c.uuid == report_uuid);
            (reports.next()?, reports.next()?)
        };

        Some(Self {
            keyboard_input,
            keyboard_output,
            mouse,
            media,
            system,
            config_input,
            config_output,
        })
    }

    /// Subscribe to everything the keyboard notifies on by writing each CCCD once.
    async fn subscribe<C: Controller>(&self, client: &Client<'_, C>) -> Option<()> {
        for ch in [
            &self.keyboard_input,
            &self.mouse,
            &self.media,
            &self.system,
            &self.config_input,
        ] {
            if let Some(cccd) = ch.cccd_handle {
                client.write_handle(cccd, &[0x01, 0x00]).await.ok()?;
            }
        }
        Some(())
    }

    /// The HID report a notification carries, or `None`. BLE report bytes match
    /// the USB layout, so the handle alone tells the report type.
    fn report(&self, handle: u16, data: &[u8]) -> Option<Report> {
        if handle == self.keyboard_input.handle && data.len() >= 8 {
            Some(Report::KeyboardReport(KeyboardReport {
                modifier: data[0],
                reserved: 0,
                leds: 0,
                keycodes: data[2..8].try_into().unwrap(),
            }))
        } else if handle == self.mouse.handle && data.len() >= 5 {
            Some(Report::MouseReport(MouseReport {
                buttons: data[0],
                x: data[1] as i8,
                y: data[2] as i8,
                wheel: data[3] as i8,
                pan: data[4] as i8,
            }))
        } else if handle == self.media.handle && data.len() >= 2 {
            Some(Report::MediaKeyboardReport(MediaKeyboardReport {
                usage_id: u16::from_le_bytes([data[0], data[1]]),
            }))
        } else if handle == self.system.handle && !data.is_empty() {
            Some(Report::SystemControlReport(SystemControlReport { usage_id: data[0] }))
        } else {
            None
        }
    }
}

/// Release whatever the keyboard was holding when the link dropped, so nothing
/// stays stuck down on the host.
async fn release_held_keys() {
    for report in [
        Report::KeyboardReport(KeyboardReport::default()),
        Report::MouseReport(MouseReport {
            buttons: 0,
            x: 0,
            y: 0,
            wheel: 0,
            pan: 0,
        }),
        Report::MediaKeyboardReport(MediaKeyboardReport { usage_id: 0 }),
        Report::SystemControlReport(SystemControlReport { usage_id: 0 }),
    ] {
        send_hid_report(report).await;
    }
}

#[cfg(test)]
mod tests {
    use bt_hci::FromHciBytes;
    use bt_hci::param::LeAdvReports;

    use super::*;

    const BONDED: [u8; 6] = [1, 2, 3, 4, 5, 6];
    const OTHER: [u8; 6] = [7, 8, 9, 10, 11, 12];

    const ADV_IND: u8 = 0;
    const ADV_DIRECT_IND: u8 = 1;

    /// Flags + manufacturer-specific data naming a dongle-seeking advertisement (see `Adv::build`).
    const SEEKING_DATA: &[u8] = &[0x02, 0x01, 0x04, 0x04, 0xFF, 0x53, 0x52, 0x01];
    /// A host advertisement carries no RMK payload; flags alone stand in for it.
    const HOST_DATA: &[u8] = &[0x02, 0x01, 0x06];

    /// One legacy advertising report as the controller lays it out:
    /// count, kind, address kind, address, data length, data, rssi.
    fn report(handler: &ScanHandler, event_kind: u8, addr: [u8; 6], data: &[u8]) {
        let mut bytes: heapless::Vec<u8, 64> = heapless::Vec::new();
        bytes.extend_from_slice(&[1, event_kind, 1]).unwrap();
        bytes.extend_from_slice(&addr).unwrap();
        bytes.push(data.len() as u8).unwrap();
        bytes.extend_from_slice(data).unwrap();
        bytes.push(0xC0).unwrap(); // rssi
        let (reports, _) = LeAdvReports::from_hci_bytes(&bytes).unwrap();
        handler.on_adv_reports(reports.iter());
    }

    fn bonded_handler() -> ScanHandler {
        let handler = ScanHandler::new();
        handler.bonded_addr.lock(|a| a.set(Some(BdAddr::new(BONDED))));
        handler
    }

    #[test]
    fn a_host_advertisement_blocks_adoption_but_never_triggers_a_connect() {
        let handler = bonded_handler();
        report(&handler, ADV_IND, BONDED, HOST_DATA);
        assert!(handler.bonded_seen.signaled());
        assert!(!handler.bonded_asked.signaled());
    }

    #[test]
    fn a_directed_advertisement_triggers_the_connect() {
        let handler = bonded_handler();
        report(&handler, ADV_DIRECT_IND, BONDED, &[]);
        assert!(handler.bonded_asked.signaled());
        assert!(handler.bonded_seen.signaled());
    }

    #[test]
    fn the_bonded_keyboard_seeking_again_triggers_the_connect() {
        let handler = bonded_handler();
        report(&handler, ADV_IND, BONDED, SEEKING_DATA);
        assert!(handler.seeking_keyboard.signaled());
        assert!(handler.bonded_asked.signaled());
        // Seeking does not count as "seen": it competes in the window's RSSI pick instead.
        assert!(!handler.bonded_seen.signaled());
    }

    #[test]
    fn another_keyboard_is_a_pairing_candidate_and_nothing_more() {
        let handler = bonded_handler();
        report(&handler, ADV_IND, OTHER, SEEKING_DATA);
        report(&handler, ADV_DIRECT_IND, OTHER, &[]);
        assert!(handler.seeking_keyboard.signaled());
        assert!(!handler.bonded_seen.signaled());
        assert!(!handler.bonded_asked.signaled());
    }
}
