//! Dongle firmware (`dongle` feature): a BLE central that relays one bonded
//! RMK keyboard to a USB host.
//!
//! It is a HID-over-GATT client toward the keyboard and a byte relay for
//! everything else — Rynk frames pass through unparsed in both directions, so
//! the dongle answers no command of its own and never tracks the protocol.
//! Keymaps and storage stay on the keyboard; the dongle persists one bond.
//!
//! Task layout (both joined by [`Dongle::run`]):
//! - `ble_task`: trouble runner with the seeking-advertisement scan handler;
//! - [`DongleCentral::run`]: find a keyboard, connect, secure, relay, repeat.

mod router;

use core::cell::Cell;

use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy, LeSetScanParams};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use bt_hci::param::{AddrKind, BdAddr, Status};
use embassy_futures::join::join;
use embassy_futures::select::{Either, select, select3};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, Timer, with_deadline, with_timeout};
use rmk_types::protocol::rynk::{RYNK_BLE_CHUNK_SIZE, RYNK_INPUT_CHAR_UUID, RYNK_OUTPUT_CHAR_UUID, RYNK_SERVICE_UUID};
pub use router::DongleRouter;
use trouble_host::prelude::*;
use usbd_hid::descriptor::{MediaKeyboardReport, MouseReport, SystemControlReport};

use crate::ble::adv::Adv;
use crate::ble::profile::{ProfileInfo, ProfileManager};
use crate::ble::scan::{DONGLE_SCAN_WINDOW, scan_config, start_scan};
use crate::ble::{update_ble_phy, update_conn_params, wait_for_stack_started};
use crate::channel::send_hid_report;
use crate::core_traits::Runnable;
use crate::event::{EventSubscriber, LedIndicatorEvent, SubscribableEvent};
use crate::hid::{KeyboardReport, Report};
use crate::{DONGLE_PAIRING_WINDOW_SECS, RawMutex};

/// One keyboard link, and no more: the dongle relays exactly one keyboard.
const DONGLE_CONNECTIONS_MAX: usize = 1;
const DONGLE_L2CAP_CHANNELS_MAX: usize = DONGLE_CONNECTIONS_MAX * 4; // Signal + att + smp + hid

/// BLE resources sized for the dongle role; owned by [`Dongle::run`].
type DongleBleResources = HostResources<DefaultPacketPool, DONGLE_CONNECTIONS_MAX, DONGLE_L2CAP_CHANNELS_MAX>;

/// The services discovery keeps: 0x1812 matches both the report service and the
/// rynk HID service, plus the rynk service itself.
type Client<'a, C> = GattClient<'a, C, DefaultPacketPool, 3>;

const BOND_SLOT: u8 = 0;

/// Which keyboard a connection is to, which decides whether it pairs or
/// encrypts, and whether a refused key means the stored bond is dead.
#[derive(Clone, Copy, PartialEq)]
enum Peer {
    /// The keyboard this dongle already holds a bond for.
    Bonded,
    /// A keyboard from a pairing window; it replaces that bond.
    New,
}

/// What the scan handler tells [`DongleCentral`]. It runs inside the BLE runner,
/// where it can reach neither the profile manager nor the stack's bond list, so
/// it only reports what it sees; the dongle task acts on it.
struct ScanHandler {
    /// Whose return to report, mirrored by [`DongleCentral::run`] every time it
    /// re-reads the bond — the one thing the task tells the handler.
    bonded_addr: BlockingMutex<RawMutex, Cell<Option<BdAddr>>>,
    /// The latest keyboard sighted seeking a dongle, and how strong its signal was.
    seeking_keyboard: Signal<RawMutex, ((AddrKind, BdAddr), i8)>,
    /// The bonded keyboard turned up, so the pairing window can stand down.
    bonded_seen: Signal<RawMutex, ()>,
}

impl ScanHandler {
    fn new() -> Self {
        Self {
            bonded_addr: BlockingMutex::new(Cell::new(None)),
            seeking_keyboard: Signal::new(),
            bonded_seen: Signal::new(),
        }
    }
}

/// Runner event handler: surface seeking keyboards, and the bonded one's return.
impl EventHandler for ScanHandler {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = it.next() {
            if Adv::decode(report.data) == Some(Adv::DongleSeeking) {
                debug!("[dongle] seeking keyboard {:?} rssi {}", report.addr, report.rssi);
                self.seeking_keyboard
                    .signal(((report.addr_kind, report.addr), report.rssi));
            } else if self.bonded_addr.lock(|addr| addr.get()) == Some(report.addr) {
                self.bonded_seen.signal(());
            }
        }
    }
}

/// The dongle runnable. Owns and sizes its own BLE stack — the keyboard role's
/// [`crate::ble::BleTransport`] is not involved, so one build can carry both
/// kinds of binaries. The USB side is a stock [`crate::usb::UsbTransport`]
/// serving the same [`DongleRouter`].
pub struct Dongle<'a, C> {
    /// Taken by `run`, which owns the stack and its resources.
    controller: Option<C>,
    address: [u8; 6],
    router: &'a DongleRouter,
}

impl<'a, C> Dongle<'a, C> {
    /// `router` is the one this binary's [`crate::usb::UsbTransport`] serves:
    /// the dongle task relays through it, the USB sessions fill it.
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

/// The dongle's BLE central: what it holds for longer than one connection. Per-connection
/// state — the link, its GATT client — stays in the signatures.
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
                // If there is bonded keyboard, repeatly connect to that keyboard.
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
            // The relaying interval from the start, but no latency and a longer
            // supervision timeout: pairing and discovery run on this link before
            // it is tuned down.
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
        // An absent keyboard times out the attempt; the caller's loop retries.
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

    /// Scan for keyboards seeking a dongle, returning the strongest sighted
    /// within 2s of the first. Ends early with `None` when the bonded keyboard
    /// turns up, so the power-on window with the keyboard present skips
    /// straight to reconnecting.
    async fn run_pairing_window(&self) -> Option<(AddrKind, BdAddr)> {
        info!("[dongle] pairing window open for {}s", DONGLE_PAIRING_WINDOW_SECS);
        self.scan.seeking_keyboard.reset();
        self.scan.bonded_seen.reset();
        let deadline = Instant::now() + Duration::from_secs(DONGLE_PAIRING_WINDOW_SECS as u64);

        let pick = async {
            let (mut best_addr, mut best_rssi) = self.scan.seeking_keyboard.wait().await;
            // Don't sit out the whole window: gather 2s past the first sighting.
            let gather = Instant::now() + Duration::from_secs(2);
            while let Ok((addr, rssi)) = with_deadline(gather, self.scan.seeking_keyboard.wait()).await {
                if rssi > best_rssi {
                    (best_addr, best_rssi) = (addr, rssi);
                }
            }
            (best_addr, best_rssi)
        };

        let Ok(session) = with_deadline(deadline, start_scan(self.stack, DONGLE_SCAN_WINDOW)).await else {
            info!("[dongle] pairing window closed, the scanner never started");
            return None;
        };
        let found = match with_deadline(deadline, select(pick, self.scan.bonded_seen.wait())).await {
            Ok(Either::First((addr, rssi))) => {
                info!("[dongle] pairing candidate {:?} (rssi {})", addr.1, rssi);
                Some(addr)
            }
            Ok(Either::Second(())) => {
                debug!("[dongle] bonded keyboard is back, closing the pairing window");
                None
            }
            Err(_) => {
                info!("[dongle] pairing window closed, no keyboard found");
                None
            }
        };
        // The connect that follows is refused an initiator until the controller has
        // actually stopped scanning.
        session.stop().await;
        found
    }

    /// One connection's whole life: secure it, relay over it, and leave the host
    /// holding nothing once it drops.
    async fn run_connection(&mut self, conn: Connection<'b, DefaultPacketPool>, peer: Peer) {
        if !self.secure_connection(&conn, peer).await {
            info!("[dongle] securing failed");
        } else if let Ok(client) = Client::new(self.stack, &conn).await {
            // The client task pumps notifications and the watcher pumps connection
            // events; the relay runs beside them and ends when either one does.
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

    /// Consume connection events for the connection's whole life: a full queue
    /// drops what is posted next, including the disconnect this returns on.
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
        // Both sides must be bondable or neither is handed bond information, and the
        // keyboard would re-pair on every reconnect. Must precede `request_security`.
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
                // Persist what the pairing produced. This covers the fresh pairing,
                // and equally a keyboard that dropped its side and re-paired over a
                // reconnect, which would otherwise leave the stored key one that no
                // keyboard will ever accept again.
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
                // A keyboard that has cleared its side refuses our key at the link
                // layer, so the refusal arrives here and not as `PairingFailed`.
                // Both mean the stored key can never work again (design §2.5): drop
                // it, and the next loop reopens for a fresh pairing.
                Ok(ConnectionEvent::Disconnected { reason }) => {
                    if peer == Peer::Bonded && reason == Status::AUTHENTICATION_FAILURE {
                        warn!("[dongle] bonded keyboard refused our key, dropping the bond");
                        self.profiles.clear_bond(BOND_SLOT).await;
                    }
                    return false;
                }
                Err(_) => return false,
                // Securing owns the event queue until it returns, so it answers these itself.
                Ok(ConnectionEvent::RequestConnectionParams(req)) => self.accept_conn_params(req).await,
                Ok(_) => {}
            }
        }
    }

    /// Everything that runs beside the GATT client task: tune the link, discover
    /// and subscribe, then relay. `None` if the setup failed, which ends the
    /// connection.
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
        self.relay(conn, client, &mut listener, &chars).await;
        Some(())
    }

    /// Relay both directions for as long as this future is polled: notifications
    /// out to USB/router, LED state and router frames back to the keyboard.
    async fn relay(
        &self,
        conn: &Connection<'_, DefaultPacketPool>,
        client: &Client<'_, C>,
        listener: &mut NotificationListener<'_, 512>,
        chars: &KeyboardCharacteristics,
    ) {
        // Largest single write chunk on the Rynk characteristic: ATT MTU minus the
        // 3-byte write header.
        let chunk_size = RYNK_BLE_CHUNK_SIZE
            .min((conn.att_mtu() as usize).saturating_sub(3))
            .max(1);

        let keyboard_to_host = async {
            loop {
                let notification = listener.next().await;
                let (handle, data) = (notification.handle(), notification.as_ref());
                // Only the Rynk stream is opaque, and it goes straight to the host.
                if handle == chars.rynk_input.handle {
                    // Never block: the typing path shares this notification queue.
                    if !matches!(self.router.to_host.try_write(data), Ok(n) if n == data.len()) {
                        // Terminate what got through, so the truncated frame fails on
                        // its own rather than gluing to the next notify — which would
                        // decode as one bogus frame and cost the host that reply too.
                        let _ = self.router.to_host.try_write(&[0]);
                        warn!("[dongle] host config stream overflow, dropping bytes");
                    }
                } else if let Some(report) = chars.report(handle, data) {
                    send_hid_report(report).await;
                }
            }
        };

        let host_to_keyboard = async {
            let mut led_events = LedIndicatorEvent::subscriber();
            loop {
                match select(led_events.next_event(), self.router.to_keyboard.receive()).await {
                    Either::First(event) => {
                        let _ = client
                            .write_characteristic_without_response(&chars.keyboard_output, &[event.0.into_bits()])
                            .await;
                    }
                    Either::Second(frame) => {
                        for part in frame.chunks(chunk_size) {
                            if client
                                .write_characteristic_without_response(&chars.rynk_output, part)
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            }
        };

        select(keyboard_to_host, host_to_keyboard).await;
    }
}

/// Parameters for the link once it relays: 7.5 ms interval — the same latency
/// budget as a split link. The generous supervision timeout trades reconnect
/// latency after a dongle power-cycle (rare) for radio-interference tolerance
/// during normal use: the keyboard only starts its directed reconnect
/// advertising once this timer expires.
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
    rynk_input: Characteristic<[u8]>,
    rynk_output: Characteristic<[u8]>,
}

impl KeyboardCharacteristics {
    /// Discover the HID and Rynk services. The five HID report characteristics
    /// share UUID 0x2A4D; both ends are RMK, so their declaration order is
    /// fixed: keyboard input, keyboard output, mouse, media, system.
    async fn discover<C: Controller>(client: &Client<'_, C>) -> Option<Self> {
        let hid = client
            .services_by_uuid(&Uuid::new_short(0x1812))
            .await
            .ok()?
            .into_iter()
            .next()?;
        let report_uuid = Uuid::new_short(0x2A4D);
        // Discovery fails unless every characteristic `HidService` declares fits.
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

        let rynk = client
            .services_by_uuid(&RYNK_SERVICE_UUID.into())
            .await
            .ok()?
            .into_iter()
            .next()?;
        let rynk_input = client
            .characteristic_by_uuid::<[u8]>(&rynk, &RYNK_INPUT_CHAR_UUID.into())
            .await
            .ok()?;
        let rynk_output = client
            .characteristic_by_uuid::<[u8]>(&rynk, &RYNK_OUTPUT_CHAR_UUID.into())
            .await
            .ok()?;

        Some(Self {
            keyboard_input,
            keyboard_output,
            mouse,
            media,
            system,
            rynk_input,
            rynk_output,
        })
    }

    /// Subscribe to everything the keyboard notifies on, by writing each CCCD
    /// once. The notifications themselves arrive on one catch-all listener.
    async fn subscribe<C: Controller>(&self, client: &Client<'_, C>) -> Option<()> {
        for ch in [
            &self.keyboard_input,
            &self.mouse,
            &self.media,
            &self.system,
            &self.rynk_input,
        ] {
            if let Some(cccd) = ch.cccd_handle {
                client.write_handle(cccd, &[0x01, 0x00]).await.ok()?;
            }
        }
        Some(())
    }

    /// The HID report a notification carries, or `None` when the handle is not
    /// a report characteristic or the payload is short. The BLE boot report and
    /// the USB one carry the same bytes, so the handle alone identifies it.
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
