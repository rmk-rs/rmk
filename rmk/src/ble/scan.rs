//! Passive scanning, shared by the split central and the dongle.

#[cfg(not(feature = "ext_adv"))]
use bt_hci::cmd::le::LeSetScanParams;
#[cfg(feature = "ext_adv")]
use bt_hci::cmd::le::{LeExtCreateConn, LeSetExtScanParams};
#[cfg(feature = "ext_adv")]
use bt_hci::controller::ControllerCmdAsync;
use bt_hci::controller::ControllerCmdSync;
use embassy_time::{Duration, Timer};
use trouble_host::prelude::*;

#[cfg(feature = "ext_adv")]
pub(crate) trait CentralController:
    Controller + ControllerCmdSync<LeSetExtScanParams> + ControllerCmdAsync<LeExtCreateConn>
{
}

#[cfg(feature = "ext_adv")]
impl<C: Controller + ControllerCmdSync<LeSetExtScanParams> + ControllerCmdAsync<LeExtCreateConn>> CentralController
    for C
{
}

#[cfg(not(feature = "ext_adv"))]
pub(crate) trait CentralController: Controller + ControllerCmdSync<LeSetScanParams> {}

#[cfg(not(feature = "ext_adv"))]
impl<C: Controller + ControllerCmdSync<LeSetScanParams>> CentralController for C {}

/// Split central: the radio is shared with the host link and the peripheral
/// connections, so it can spare only 30ms of every 100ms.
#[cfg(feature = "split")]
pub(crate) const SPLIT_CENTRAL_SCAN_WINDOW: Duration = Duration::from_millis(30);

/// Dongle: USB-powered with a single link, so favor fast discovery.
#[cfg(feature = "dongle")]
pub(crate) const DONGLE_SCAN_WINDOW: Duration = Duration::from_millis(60);

/// A passive scan listening for `window` out of every 100ms.
pub(crate) fn scan_config(window: Duration) -> ScanConfig<'static> {
    ScanConfig {
        active: false,
        interval: Duration::from_millis(100),
        window,
        ..Default::default()
    }
}

/// Start a scan, retrying while the controller refuses one — it does until a
/// previous connect's initiator has stopped.
///
/// End the session with [`ScanSession::stop`], which waits for the controller to
/// confirm. Dropping it only signals the cancel, and the controller refuses an
/// initiator until the runner has issued the stop.
pub(crate) async fn start_scan<'a, C: CentralController>(
    stack: &'a Stack<'_, C, DefaultPacketPool>,
    window: Duration,
    filter_accept_list: &[Address],
) -> ScanSession<'a, { cfg!(feature = "ext_adv") }> {
    let config = ScanConfig {
        filter_accept_list,
        ..scan_config(window)
    };
    loop {
        let mut central = stack.central();
        let mut scanner = Scanner::new(&mut central);
        #[cfg(feature = "ext_adv")]
        let scan = scanner.scan_ext(&config);
        #[cfg(not(feature = "ext_adv"))]
        let scan = scanner.scan(&config);
        match scan.await {
            Ok(session) => return session,
            Err(_) => Timer::after_millis(500).await,
        }
    }
}

pub(crate) async fn connect<'a, C: CentralController>(
    central: &mut Central<'a, C, DefaultPacketPool>,
    config: &ConnectConfig<'_>,
) -> Result<Connection<'a, DefaultPacketPool>, BleHostError<C::Error>> {
    #[cfg(feature = "ext_adv")]
    let connect = central.connect_ext(config);
    #[cfg(not(feature = "ext_adv"))]
    let connect = central.connect(config);
    connect.await
}
