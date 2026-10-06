//! Mock rmk crate for macro expansion tests.
//!
//! Provides minimal types and traits that mirror the real rmk crate
//! for testing macro expansions without needing embassy or hardware dependencies.

#![no_std]
#![allow(dead_code)]
#![allow(async_fn_in_trait)]

// Re-export futures for select_biased! macro usage
pub use futures;

/// Re-export select_biased! macro from futures crate
#[macro_export]
macro_rules! select_biased {
    ($($tokens:tt)*) => {
        $crate::futures::select_biased!($($tokens)*)
    };
}

/// Mock RawMutex type
pub struct RawMutex;

/// Mock embassy_time module
pub mod embassy_time {
    /// Mock Duration type
    #[derive(Clone, Copy, Debug)]
    pub struct Duration(pub u64);

    impl Duration {
        pub const fn from_millis(ms: u64) -> Self {
            Duration(ms)
        }

        pub const MIN: Duration = Duration(0);

        pub fn checked_sub(self, other: Duration) -> Option<Duration> {
            self.0.checked_sub(other.0).map(Duration)
        }
    }

    /// Mock Instant type
    #[derive(Clone, Copy, Debug)]
    pub struct Instant(pub u64);

    impl Instant {
        pub fn now() -> Self {
            Instant(0)
        }

        pub fn elapsed(&self) -> Duration {
            Duration(0)
        }
    }

    /// Mock Timer type
    pub struct Timer;

    impl Timer {
        pub fn after(_duration: Duration) -> impl core::future::Future<Output = ()> {
            async {}
        }
    }
}

/// Event system module - mirrors rmk::event
pub mod event {
    /// Trait for event publishers (synchronous, non-blocking)
    pub trait EventPublisher {
        type Event;
        fn publish(&self, message: Self::Event);
    }

    /// Async version of event publisher trait
    pub trait AsyncEventPublisher {
        type Event;
        fn publish_async(&self, message: Self::Event) -> impl core::future::Future<Output = ()>;
    }

    /// Trait for event subscribers (always async)
    pub trait EventSubscriber {
        type Event;
        fn next_event(&mut self) -> impl core::future::Future<Output = Self::Event>;
    }

    impl EventSubscriber for core::future::Pending<core::convert::Infallible> {
        type Event = core::convert::Infallible;

        async fn next_event(&mut self) -> Self::Event {
            self.await
        }
    }

    /// Trait for events that can be published
    pub trait PublishableEvent: Clone {
        type Publisher: EventPublisher<Event = Self>;
        fn publisher() -> Self::Publisher;
    }

    /// Async version of publishable event trait
    pub trait AsyncPublishableEvent: PublishableEvent {
        type AsyncPublisher: AsyncEventPublisher<Event = Self>;
        fn publisher_async() -> Self::AsyncPublisher;
    }

    /// Trait for events that can be subscribed to
    pub trait SubscribableEvent: Clone {
        type Subscriber: EventSubscriber<Event = Self>;
        fn subscriber() -> Self::Subscriber;
    }

    /// Combined trait for events that support both publish and subscribe
    pub trait Event: PublishableEvent + SubscribableEvent {}

    // Auto-implement Event for types that implement both
    impl<T: PublishableEvent + SubscribableEvent> Event for T {}

    /// Async version of event trait
    pub trait AsyncEvent: Event + AsyncPublishableEvent {}

    impl<T: Event + AsyncPublishableEvent> AsyncEvent for T {}

    /// Publish an event (non-blocking, may drop if buffer full)
    pub fn publish_event<E: PublishableEvent>(e: E) {
        E::publisher().publish(e);
    }

    /// Publish an event with backpressure (waits if buffer full)
    pub async fn publish_event_async<E: AsyncPublishableEvent>(e: E) {
        E::publisher_async().publish_async(e).await;
    }
}

/// Core traits module - mirrors rmk::core_traits
pub mod core_traits {
    /// Trait for runnable input devices and processors
    pub trait Runnable {
        async fn run(&mut self) -> !;
    }
}

/// Input device module - mirrors rmk::input_device
pub mod input_device {
    use crate::core_traits::Runnable;

    /// Trait for input devices
    pub trait InputDevice: Runnable {
        /// The event type produced by this input device
        type Event;

        /// Read the raw input event
        fn read_event(&mut self) -> impl core::future::Future<Output = Self::Event>;
    }
}

/// Processor module - mirrors rmk::processor
pub mod processor {
    use crate::core_traits::Runnable;
    use crate::event::EventSubscriber;

    /// Unified trait for event processors
    pub trait Processor: Runnable {
        /// Type of the received events
        type Event;

        /// Create a new event subscriber
        fn subscriber() -> impl EventSubscriber<Event = Self::Event>;

        /// Process the received event
        fn process(&mut self, event: Self::Event) -> impl core::future::Future<Output = ()>;

        /// Default processing loop that continuously receives and processes events
        async fn process_loop(&mut self) -> ! {
            let mut sub = Self::subscriber();
            loop {
                let event = sub.next_event().await;
                self.process(event).await;
            }
        }
    }

    /// Trait for processors with periodic updates
    pub trait PollingProcessor: Processor {
        /// Returns the interval between update calls
        fn interval(&self) -> crate::embassy_time::Duration;

        /// Update periodically
        fn update(&mut self) -> impl core::future::Future<Output = ()>;

        /// Polling loop that processes events and calls update at the specified interval
        async fn polling_loop(&mut self) -> ! {
            loop {
                // Mock implementation - just loop forever
            }
        }
    }

    /// Trait for processors driven by a dynamic deadline in addition to events
    pub trait DeadlineProcessor: Processor {
        /// The next moment `on_deadline` should fire, or `None` when nothing is armed
        fn deadline(&self) -> Option<crate::embassy_time::Instant>;

        /// Called when the deadline elapses without an intervening event
        fn on_deadline(&mut self) -> impl core::future::Future<Output = ()>;

        /// Loop that interleaves event processing with the deadline
        async fn deadline_loop(&mut self) -> ! {
            loop {
                // Mock implementation - just loop forever
            }
        }

        async fn polling_deadline_loop(&mut self) -> !
        where
            Self: PollingProcessor,
        {
            loop {}
        }
    }
}

/// Mock macros module for marker attributes
pub mod macros {}

/// Mock KeyMap struct for keyboard configuration
pub struct KeyMap<
    'a,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize,
> {
    _phantom: core::marker::PhantomData<&'a ()>,
}
