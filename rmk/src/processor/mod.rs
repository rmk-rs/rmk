//! Runtime traits for event consumers in RMK.
//!
//! In RMK's event system, `crate::event` defines how event types are published
//! and subscribed, while `Processor` defines how a task consumes those events.
//! `Processor` provides the core consume loop (`subscriber` -> `next_event` ->
//! `process`). `PollingProcessor` adds fixed polling ticks and `DeadlineProcessor`
//! adds dynamic deadlines; both can run alongside event handling in one loop.

pub mod builtin;

use embassy_futures::select::{Either, Either3, select, select3};
use embassy_time::{Instant, Timer};

use crate::core_traits::Runnable;
use crate::event::EventSubscriber;

/// Unified trait for event and timer processors.
///
/// This trait provides the interface for all event-driven processors in RMK.
/// Use the `#[processor]` macro to automatically implement this trait.
///
/// ```rust,ignore
/// use rmk_macro::processor;
///
/// // Single event subscription
/// #[processor(subscribe = [LedIndicatorEvent])]
/// struct MyProcessor { /* ... */ }
///
/// impl MyProcessor {
///     // You MUST implement on_{event_name}_event handler method
///     // for each event type in `subscribe = [..]`
///     async fn on_led_indicator_event(&mut self, event: LedIndicatorEvent) {
///         // handle event
///     }
/// }
///
/// // Multiple event subscription
/// #[processor(subscribe = [EventA, EventB])]
/// struct MyMultiProcessor { /* ... */ }
///
/// impl MyMultiProcessor {
///     async fn on_event_a_event(&mut self, event: EventA) { /* ... */ }
///     async fn on_event_b_event(&mut self, event: EventB) { /* ... */ }
/// }
/// ```
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

/// Trait for processors with periodic updates.
///
/// This trait extends `Processor` with periodic update capability.
/// The polling loop alternates between waiting for events and calling `update()`
/// at the specified interval.
///
/// ```rust,ignore
/// use rmk_macro::processor;
///
/// #[processor(subscribe = [BatteryStatusEvent], poll_interval = 1000)]
/// struct BatteryLedProcessor {
///     led_on: bool,
/// }
///
/// impl BatteryLedProcessor {
///     async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
///         // Update internal state based on battery event
///     }
///
///     // Called every 1000ms (poll_interval)
///     async fn poll(&mut self) {
///         // Toggle LED based on battery status
///         self.led_on = !self.led_on;
///     }
/// }
/// ```
pub trait PollingProcessor: Processor {
    /// Returns the interval between `update` calls.
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

/// Trait for processors driven by a dynamic timeout in addition to events.
///
/// Unlike [`PollingProcessor`], whose tick fires at a fixed interval, a
/// deadline processor produces the next deadline on demand from its own
/// state -- motion may extend it, external state may clear it. When
/// [`next_deadline`](Self::next_deadline) returns `None`, the deadline timer is inactive;
/// events and any configured polling continue.
///
/// Add `deadline` to `#[processor]` and provide inherent `deadline()` and
/// `on_deadline()` methods. The macro implements this trait and [`Runnable`],
/// including polling when `poll_interval` is also set.
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
