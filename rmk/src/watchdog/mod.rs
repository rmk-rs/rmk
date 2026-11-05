use embassy_time::{Duration, Timer};

use crate::core_traits::Runnable;

#[cfg(feature = "_esp_ble")]
mod esp32;
#[cfg(feature = "_nrf")]
#[cfg(not(any(feature = "nrf54l15", feature = "nrf54lm20")))]
mod nrf52;
#[cfg(feature = "rp2040")]
mod rp2040;

#[cfg(feature = "_esp_ble")]
pub use esp32::Esp32Watchdog;
#[cfg(feature = "_nrf")]
#[cfg(not(any(feature = "nrf54l15", feature = "nrf54lm20")))]
pub use nrf52::Nrf52Watchdog;
#[cfg(feature = "rp2040")]
pub use rp2040::Rp2040Watchdog;

/// Chip-agnostic watchdog feeding. Implement this for your platform's
/// watchdog peripheral, then pass it to [`WatchdogRunner`].
pub trait WatchdogFeed: Send {
    fn feed(&mut self);
}

/// A [`Runnable`] that periodically feeds a hardware watchdog.
///
/// Pass this to `run_all!` alongside your keyboard and matrix. Because
/// all runnables are joined cooperatively, a tight-loop stall in any
/// sibling task will block this runner too, letting the hardware
/// watchdog fire a reset.
pub struct WatchdogRunner<W: WatchdogFeed> {
    watchdog: W,
    interval: Duration,
}

impl<W: WatchdogFeed> WatchdogRunner<W> {
    pub fn new(watchdog: W, interval: Duration) -> Self {
        Self { watchdog, interval }
    }
}

impl<W: WatchdogFeed> Runnable for WatchdogRunner<W> {
    async fn run(&mut self) -> ! {
        loop {
            self.watchdog.feed();
            Timer::after(self.interval).await;
        }
    }
}
