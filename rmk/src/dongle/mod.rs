//! Dongle firmware (`dongle` feature): a BLE central that relays one bonded
//! RMK keyboard to a USB host.
//!
//! It is a HID-over-GATT client toward the keyboard and a byte relay for
//! everything else — Rynk frames pass through unparsed in both directions, so
//! the dongle answers no command of its own and never tracks the protocol.
//! Keymaps and storage stay on the keyboard; the dongle persists one bond.
//!
//! Task layout (all joined by [`Dongle::run`]):
//! - `ble_task`: trouble runner with the seeking-advertisement scan handler;
//! - `link_task`: find a keyboard, connect, secure, relay, repeat;
//! - bond removal, since only `run` holds the stack.

pub(crate) mod link;
pub(crate) mod router;

use core::cell::RefCell;

use bt_hci::cmd::le::{LeReadLocalSupportedFeatures, LeSetPhy, LeSetScanParams};
use bt_hci::controller::{ControllerCmdAsync, ControllerCmdSync};
use bt_hci::param::{AddrKind, BdAddr};
use embassy_futures::join::join3;
use embassy_futures::select::{Either3, select3};
use embassy_sync::blocking_mutex::Mutex as BlockingMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Instant, with_deadline};
pub use router::DongleRouter;
use trouble_host::prelude::*;

use crate::ble::adv::Adv;
use crate::ble::scan::{DONGLE_SCAN_WINDOW, hold_scan};
use crate::core_traits::Runnable;
use crate::{DONGLE_PAIRING_WINDOW_SECS, RawMutex};

/// One keyboard link, and no more: the dongle relays exactly one keyboard.
const DONGLE_CONNECTIONS_MAX: usize = 1;
const DONGLE_L2CAP_CHANNELS_MAX: usize = DONGLE_CONNECTIONS_MAX * 4; // Signal + att + smp + hid

/// BLE resources sized for the dongle role; owned by [`Dongle::run`].
type DongleBleResources = HostResources<DefaultPacketPool, DONGLE_CONNECTIONS_MAX, DONGLE_L2CAP_CHANNELS_MAX>;

/// The keyboard this dongle relays.
pub(crate) struct Peer {
    /// `None` until a keyboard pairs in a pairing window.
    pub(crate) bond: Option<BondInformation>,
    /// The link task has it connected and is relaying.
    pub(crate) connected: bool,
}

static PEER: BlockingMutex<RawMutex, RefCell<Peer>> = BlockingMutex::new(RefCell::new(Peer {
    bond: None,
    connected: false,
}));

pub(crate) fn read_peer<R>(f: impl FnOnce(&Peer) -> R) -> R {
    PEER.lock(|p| f(&p.borrow()))
}

pub(crate) fn update_peer<R>(f: impl FnOnce(&mut Peer) -> R) -> R {
    PEER.lock(|p| f(&mut p.borrow_mut()))
}

/// Latest matching seeking advertisement seen by the scan handler.
static SEEKER_FOUND: Signal<RawMutex, ((AddrKind, BdAddr), i8)> = Signal::new();

/// The bonded keyboard turned up, so the pairing window can stand down.
static BONDED_SEEN: Signal<RawMutex, ()> = Signal::new();

/// A bond to drop from the trouble stack, consumed by a housekeeping task in
/// [`Dongle::run`], since only it holds the stack.
pub(crate) static REMOVED_BOND: Channel<RawMutex, Identity, 1> = Channel::new();

/// Runner event handler: surface seeking keyboards, and the bonded one's return.
struct DongleScanHandler;

impl EventHandler for DongleScanHandler {
    fn on_adv_reports(&self, mut it: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = it.next() {
            if Adv::decode(report.data) == Some(Adv::DongleSeeking) {
                debug!("[dongle] seeking keyboard {:?} rssi {}", report.addr, report.rssi);
                SEEKER_FOUND.signal(((report.addr_kind, report.addr), report.rssi));
            } else if read_peer(|p| p.bond.as_ref().is_some_and(|b| b.identity.addr.addr == report.addr)) {
                BONDED_SEEN.signal(());
            }
        }
    }
}

/// Scan for keyboards seeking a dongle, returning the strongest sighted within
/// 2s of the first. Ends early with `None` when the bonded keyboard turns up,
/// so one back from sleep is not left waiting out the window.
async fn run_pairing_window<C>(stack: &Stack<'_, C, DefaultPacketPool>) -> Option<(AddrKind, BdAddr)>
where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    info!("[dongle] pairing window open for {}s", DONGLE_PAIRING_WINDOW_SECS);
    SEEKER_FOUND.reset();
    BONDED_SEEN.reset();
    let deadline = Instant::now() + Duration::from_secs(DONGLE_PAIRING_WINDOW_SECS as u64);

    let pick = async {
        let (mut best_addr, mut best_rssi) = SEEKER_FOUND.wait().await;
        // Don't sit out the whole window: gather 2s past the first sighting.
        let gather = Instant::now() + Duration::from_secs(2);
        while let Ok((addr, rssi)) = with_deadline(gather, SEEKER_FOUND.wait()).await {
            if rssi > best_rssi {
                (best_addr, best_rssi) = (addr, rssi);
            }
        }
        (best_addr, best_rssi)
    };

    match with_deadline(
        deadline,
        select3(hold_scan(stack, DONGLE_SCAN_WINDOW), pick, BONDED_SEEN.wait()),
    )
    .await
    {
        Ok(Either3::Second((addr, rssi))) => {
            info!("[dongle] pairing candidate {:?} (rssi {})", addr.1, rssi);
            Some(addr)
        }
        Ok(Either3::Third(())) => {
            debug!("[dongle] bonded keyboard is back, closing the pairing window");
            None
        }
        _ => {
            info!("[dongle] pairing window closed, no keyboard found");
            None
        }
    }
}

/// The dongle runnable. Owns and sizes its own BLE stack — the keyboard role's
/// [`crate::ble::BleTransport`] is not involved, so one build can carry both
/// kinds of binaries. The USB side is a stock [`crate::usb::UsbTransport`]
/// with [`DongleRouter`] attached.
pub struct Dongle<C> {
    /// Taken by `run`, which owns the stack and its resources.
    controller: Option<C>,
    address: [u8; 6],
}

impl<C> Dongle<C> {
    /// `bond` comes from [`crate::storage::new_storage_for_dongle`].
    pub fn new(controller: C, address: [u8; 6], bond: Option<BondInformation>) -> Self {
        if let Some(bond) = &bond {
            info!("[dongle] bonded to {:?}", bond.identity.addr);
        }
        update_peer(|p| p.bond = bond);
        Self {
            controller: Some(controller),
            address,
        }
    }

    /// The USB router, for [`crate::usb::UsbTransport::with_dongle_router`].
    pub fn router(&self) -> &'static DongleRouter {
        &DongleRouter
    }
}

impl<C> Runnable for Dongle<C>
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

        // Register the persisted bond with the freshly built stack.
        if let Some(bond) = read_peer(|p| p.bond.clone())
            && let Err(e) = stack.add_bond_information(bond)
        {
            warn!("[dongle] add bond error: {:?}", e);
        }

        let housekeeping = async {
            loop {
                let identity = REMOVED_BOND.receive().await;
                if let Err(e) = stack.remove_bond_information(identity) {
                    debug!("[dongle] remove bond error: {:?}", e);
                }
            }
        };
        join3(
            crate::ble::ble_task(stack.runner(), &DongleScanHandler),
            link::link_task(stack),
            housekeeping,
        )
        .await;
        unreachable!("Dongle sub-tasks must run forever")
    }
}
