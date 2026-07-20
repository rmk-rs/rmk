//! Keyboard-wide sleep management.
//!
//! One manager owns the keyboard's sleep state: it watches [`SLEEP_INPUT`] and
//! latches each decision through [`crate::state::set_sleeping`], which both
//! stores the value for pollers like the battery service and publishes it as
//! [`crate::event::SleepStateEvent`] for everything else — the display, the
//! lighting engine, host readback, and on split centrals the per-link
//! connection-parameter followers in `split::ble::central`.

use embassy_futures::select::{Either, select};
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Timer};

use crate::SPLIT_CENTRAL_SLEEP_TIMEOUT_SECONDS;

/// Input to [`run_sleep_manager`], same encoding as the latched sleep state:
/// - `true`: sleep now, without waiting out the idle timeout
/// - `false`: activity — wake up, or restart the idle timeout
static SLEEP_INPUT: Signal<crate::RawMutex, bool> = Signal::new();

/// Report keyboard activity: wake the keyboard up, or restart the idle timeout
/// when it's already awake.
pub(crate) fn report_activity() {
    SLEEP_INPUT.signal(false);
}

/// Ask the keyboard to sleep now instead of waiting out the idle timeout. Sent
/// when the host suspends us or when advertising times out.
pub(crate) fn request_sleep() {
    SLEEP_INPUT.signal(true);
}

/// The keyboard's one sleep manager.
///
/// Run by [`crate::ble::BleTransport`], the single always-present BLE task, so
/// the state can never get stuck: split or not, connected or not, inputs always
/// reach it. Disabled when the configured timeout
/// (`split_central_sleep_timeout_seconds`) is 0, the default.
pub(crate) async fn run_sleep_manager() {
    if SPLIT_CENTRAL_SLEEP_TIMEOUT_SECONDS == 0 {
        info!("Sleep management disabled (timeout = 0)");
        core::future::pending::<()>().await;
        return;
    }

    info!(
        "Sleep manager started with {}s timeout",
        SPLIT_CENTRAL_SLEEP_TIMEOUT_SECONDS
    );
    manage_sleep_state(Duration::from_secs(SPLIT_CENTRAL_SLEEP_TIMEOUT_SECONDS.into())).await
}

/// The sleep state machine, separate from [`run_sleep_manager`] only so tests
/// can drive it with a short timeout (the configured one is 0 in test builds).
async fn manage_sleep_state(idle_timeout: Duration) -> ! {
    loop {
        // Awake: sleep once the keyboard has been idle for `idle_timeout`, or as
        // soon as something asks us to. The input is polled first so activity
        // racing the timeout wins the tie instead of causing a spurious sleep.
        loop {
            match select(SLEEP_INPUT.wait(), Timer::after(idle_timeout)).await {
                Either::First(true) | Either::Second(_) => break,
                Either::First(false) => debug!("Activity detected, resetting sleep timeout"),
            }
        }
        info!("Entering sleep mode");
        crate::state::set_sleeping(true);

        // Asleep: only activity wakes us; further sleep requests change nothing.
        while SLEEP_INPUT.wait().await {}

        info!("Waking up from sleep mode due to activity");
        crate::state::set_sleeping(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_block_on as block_on;

    /// Run the state machine with a 1s idle timeout until `script` finishes.
    fn with_sleep_manager(script: impl core::future::Future<Output = ()>) {
        block_on(async {
            select(manage_sleep_state(Duration::from_secs(1)), script).await;
        });
    }

    fn sleeping() -> bool {
        crate::state::current_sleeping()
    }

    #[test]
    fn sleeps_when_idle_and_wakes_on_activity() {
        with_sleep_manager(async {
            Timer::after_millis(900).await;
            assert!(!sleeping(), "still inside the idle timeout");

            Timer::after_millis(200).await;
            assert!(sleeping(), "idle timeout elapsed");

            report_activity();
            Timer::after_millis(10).await;
            assert!(!sleeping(), "activity wakes the keyboard");
        });
    }

    #[test]
    fn activity_restarts_the_idle_timeout() {
        with_sleep_manager(async {
            // Each report lands inside the timeout, so 1.8s of steady typing
            // must never reach it.
            for _ in 0..3 {
                Timer::after_millis(600).await;
                assert!(!sleeping(), "activity must restart the idle timeout");
                report_activity();
            }
        });
    }

    #[test]
    fn sleep_request_skips_the_idle_timeout() {
        with_sleep_manager(async {
            request_sleep();
            Timer::after_millis(10).await;
            assert!(sleeping(), "a sleep request doesn't wait for the timeout");

            request_sleep();
            Timer::after_millis(10).await;
            assert!(sleeping(), "a second request while asleep changes nothing");

            report_activity();
            Timer::after_millis(10).await;
            assert!(!sleeping());
        });
    }
}
