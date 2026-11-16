use embassy_time::Duration;

use super::{WatchdogFeed, WatchdogRunner};

/// RP2040 watchdog wrapper that reloads the countdown on each feed.
/// Hardware max timeout is ~8.3s (RP2040-E1 errata).
pub struct Rp2040Watchdog<'d> {
    inner: embassy_rp::watchdog::Watchdog<'d>,
    timeout: Duration,
}

impl<'d> Rp2040Watchdog<'d> {
    pub fn new(watchdog: embassy_rp::watchdog::Watchdog<'d>, timeout: Duration) -> Self {
        Self {
            inner: watchdog,
            timeout,
        }
    }

    pub fn start(&mut self) {
        self.inner.pause_on_debug(true);
        self.inner.start(self.timeout);
    }

    pub fn default_runner(watchdog: embassy_rp::watchdog::Watchdog<'d>) -> WatchdogRunner<Self> {
        let mut wdt = Self::new(watchdog, Duration::from_secs(8));
        wdt.start();
        WatchdogRunner::new(wdt, Duration::from_secs(4))
    }
}

impl WatchdogFeed for Rp2040Watchdog<'_> {
    fn feed(&mut self) {
        self.inner.feed(self.timeout);
    }
}
