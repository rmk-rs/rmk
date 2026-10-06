//! Process keyboard events and schedule periodic work or state-dependent timeouts.
//!
//! Use `#[processor]` to generate event handling and the run loop.
//! Implement [`DeadlineProcessor`] to supply deadline behavior.
//! Implement [`Runnable`] yourself when you need a different run loop.

pub mod builtin;

use embassy_futures::select::{Either, Either3, select, select3};
use embassy_time::{Instant, Timer};

use crate::core_traits::Runnable;
use crate::event::EventSubscriber;

/// Receives events and handles them one at a time.
///
/// Use `#[processor(subscribe = [...])]` to generate this implementation and
/// provide an async `on_<event_name>_event` handler for each subscribed event.
/// A timer-only processor uses [`core::convert::Infallible`] as its event type.
///
/// [`process_loop`](Self::process_loop) runs event handling without polling or deadlines.
pub trait Processor: Runnable {
    /// Type of the received events, or [`core::convert::Infallible`] without subscriptions.
    type Event;

    /// Create an event subscriber, or a permanently pending source without subscriptions.
    fn subscriber() -> impl EventSubscriber<Event = Self::Event>;

    /// Process the received event.
    async fn process(&mut self, event: Self::Event);

    /// Default processing loop that continuously receives and processes events.
    async fn process_loop(&mut self) -> ! {
        let mut sub = Self::subscriber();
        loop {
            let event = sub.next_event().await;
            self.process(event).await;
        }
    }
}

/// Adds periodic work to a processor's event handling.
///
/// Use `#[processor(poll_interval = N)]` and provide an async `poll()` method.
/// `N` is a positive interval in milliseconds. Subscriptions are optional.
///
/// [`polling_loop`](Self::polling_loop) preserves the polling schedule across events.
/// When a tick and an event are both ready, it handles the tick first.
pub trait PollingProcessor: Processor {
    /// Returns a positive interval between `update` calls.
    fn interval(&self) -> embassy_time::Duration;

    /// Update periodically, will be called according to [`Self::interval()`]
    async fn update(&mut self);

    /// Polling loop that processes events and calls `update()` at the specified interval.
    async fn polling_loop(&mut self) -> ! {
        let mut sub = Self::subscriber();
        let mut ticker = embassy_time::Ticker::every(self.interval());

        loop {
            match select(ticker.next(), sub.next_event()).await {
                Either::First(_) => self.update().await,
                Either::Second(event) => self.process(event).await,
            }
        }
    }
}

/// Schedules a timeout from the processor's current state.
///
/// Implement [`deadline`](Self::deadline) and [`on_deadline`](Self::on_deadline)
/// directly on this trait. Add `deadline` to `#[processor]` to generate a run loop
/// that calls the trait methods. TOML and Rust projects use the same implementation.
///
/// Return `None` to disable the timeout. After it fires, clear or advance it.
/// To also poll, add `poll_interval` and provide `poll()`.
pub trait DeadlineProcessor: Processor {
    /// The next moment at which [`on_deadline`](Self::on_deadline) should
    /// fire, or `None` when no timeout is currently armed.
    fn deadline(&self) -> Option<Instant>;

    /// Called when the deadline returned by [`deadline`](Self::deadline)
    /// elapses. Clear or advance the deadline before returning to avoid firing again immediately.
    async fn on_deadline(&mut self);

    /// Loop that interleaves event processing with a dynamic deadline timer.
    async fn deadline_loop(&mut self) -> ! {
        let mut sub = Self::subscriber();
        loop {
            match select(wait_for_deadline(self.deadline()), sub.next_event()).await {
                Either::First(_) => self.on_deadline().await,
                Either::Second(event) => self.process(event).await,
            }
        }
    }

    /// Process events, fixed polling ticks, and dynamic deadlines in one loop.
    ///
    /// Simultaneously ready sources run in deadline, polling, event order. Callbacks are
    /// serial; each may change the next deadline without resetting the polling cadence.
    async fn polling_deadline_loop(&mut self) -> !
    where
        Self: PollingProcessor,
    {
        let mut sub = Self::subscriber();
        let mut ticker = embassy_time::Ticker::every(self.interval());
        loop {
            match select3(wait_for_deadline(self.deadline()), ticker.next(), sub.next_event()).await {
                Either3::First(_) => self.on_deadline().await,
                Either3::Second(_) => self.update().await,
                Either3::Third(event) => self.process(event).await,
            }
        }
    }
}

async fn wait_for_deadline(deadline: Option<Instant>) {
    match deadline {
        // A new Timer yields once even when overdue; a ready deadline must win immediately.
        Some(at) if at <= Instant::now() => {}
        Some(at) => Timer::at(at).await,
        None => core::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use core::cell::Cell;
    use core::pin::pin;

    use embassy_time::{Duration, Instant, MockDriver};
    use futures::poll;
    use rmk_macro::processor;

    use super::DeadlineProcessor;
    use crate::core_traits::Runnable;
    use crate::test_support::test_block_on;

    #[processor(poll_interval = 100, deadline)]
    struct Timed<'a> {
        due: Option<Instant>,
        deadlines: &'a Cell<u8>,
        polls: &'a Cell<u8>,
    }

    impl DeadlineProcessor for Timed<'_> {
        fn deadline(&self) -> Option<Instant> {
            self.due
        }

        async fn on_deadline(&mut self) {
            self.deadlines.set(self.deadlines.get() + 1);
            self.due = None;
        }
    }

    impl Timed<'_> {
        async fn poll(&mut self) {
            self.polls.set(self.polls.get() + 1);
            self.due = Some(Instant::now() + Duration::from_millis(20));
        }
    }

    #[test]
    fn polling_rearms_deadlines_and_due_deadlines_precede_ticks() {
        test_block_on(async {
            let deadlines = Cell::new(0);
            let polls = Cell::new(0);
            let mut processor = Timed {
                due: Some(Instant::from_millis(100)),
                deadlines: &deadlines,
                polls: &polls,
            };
            let mut run = pin!(processor.run());
            assert!(poll!(run.as_mut()).is_pending());
            for (at, expected) in [(50, (0, 0)), (100, (1, 1)), (120, (2, 1)), (200, (2, 2)), (220, (3, 2))] {
                MockDriver::get().advance(Duration::from_millis(at - Instant::now().as_millis()));
                assert!(poll!(run.as_mut()).is_pending());
                assert_eq!((deadlines.get(), polls.get()), expected, "at {at} ms");
            }
        });
    }
}
