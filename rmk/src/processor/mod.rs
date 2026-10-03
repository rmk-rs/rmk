//! Process keyboard events and schedule periodic work or state-dependent timeouts.
//!
//! Use `#[processor]` to generate the traits and run loop for a processor.
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
/// Use `#[processor(deadline)]` and provide inherent `deadline()` and
/// `on_deadline()` methods. The macro maps them to [`next_deadline`](Self::next_deadline)
/// and [`handle_deadline`](Self::handle_deadline), then generates the run loop.
///
/// Return `None` to disable the timeout. After it fires, clear or advance it.
/// To also poll, add `poll_interval` and provide `poll()`.
pub trait DeadlineProcessor: Processor {
    /// The next moment at which [`handle_deadline`](Self::handle_deadline) should
    /// fire, or `None` when no timeout is currently armed.
    fn next_deadline(&self) -> Option<Instant>;

    /// Called when the deadline returned by [`next_deadline`](Self::next_deadline)
    /// elapses. Clear or advance the deadline before returning to avoid firing again immediately.
    async fn handle_deadline(&mut self);

    /// Loop that interleaves event processing with a dynamic deadline timer.
    async fn deadline_loop(&mut self) -> ! {
        let mut sub = Self::subscriber();
        loop {
            match select(wait_for_deadline(self.next_deadline()), sub.next_event()).await {
                Either::First(_) => self.handle_deadline().await,
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
            match select3(wait_for_deadline(self.next_deadline()), ticker.next(), sub.next_event()).await {
                Either3::First(_) => self.handle_deadline().await,
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
