use embassy_nrf::Peri;
use embassy_nrf::peripherals::WDT;
use embassy_nrf::wdt::{self, WatchdogHandle};
use embassy_time::Duration;

use super::{WatchdogFeed, WatchdogRunner};

/// nRF52 watchdog wrapper around a single [`WatchdogHandle`].
///
/// Create via [`embassy_nrf::wdt::Watchdog::try_new`] with `N = 1`,
/// then pass the returned handle here.
pub struct Nrf52Watchdog {
    handle: WatchdogHandle,
}

impl Nrf52Watchdog {
    pub fn new(handle: WatchdogHandle) -> Self {
        Self { handle }
    }

    pub fn default_runner(wdt: Peri<'static, WDT>) -> WatchdogRunner<Self> {
        let mut config = wdt::Config::default();
        config.timeout_ticks = 327680; // 10s at 32768 Hz
        config.action_during_debug_halt = wdt::HaltConfig::Pause;
        config.action_during_sleep = wdt::SleepConfig::Run;
        let (_driver, [handle]) = wdt::Watchdog::try_new(wdt, config).expect("WDT already active");
        WatchdogRunner::new(Self::new(handle), Duration::from_secs(5))
    }
}

impl WatchdogFeed for Nrf52Watchdog {
    fn feed(&mut self) {
        self.handle.feed();
    }
}
