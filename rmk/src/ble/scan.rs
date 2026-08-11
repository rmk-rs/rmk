//! Passive scanning, shared by the split central and the dongle.

use bt_hci::cmd::le::LeSetScanParams;
use bt_hci::controller::ControllerCmdSync;
use embassy_time::{Duration, Timer};
use trouble_host::prelude::*;

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
pub(crate) async fn start_scan<'a, C: Controller + ControllerCmdSync<LeSetScanParams>>(
    stack: &'a Stack<'_, C, DefaultPacketPool>,
    window: Duration,
) -> ScanSession<'a, false> {
    loop {
        let mut central = stack.central();
        match Scanner::new(&mut central).scan(&scan_config(window)).await {
            Ok(session) => return session,
            Err(_) => Timer::after_millis(500).await,
        }
    }
}
