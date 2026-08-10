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

/// Hold a scan session open, retrying while the controller refuses to start
/// one. Never returns: callers race it against their own stop condition.
pub(crate) async fn hold_scan<C: Controller + ControllerCmdSync<LeSetScanParams>>(
    stack: &Stack<'_, C, DefaultPacketPool>,
    window: Duration,
) -> ! {
    loop {
        let mut central = stack.central();
        let mut scanner = Scanner::new(&mut central);
        match scanner.scan(&scan_config(window)).await {
            Ok(_session) => core::future::pending::<()>().await,
            Err(_) => Timer::after_millis(500).await,
        }
    }
}
