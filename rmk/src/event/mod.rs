//! Event system for RMK
//!
//! This module provides:
//! - Event infrastructure (traits, publish/subscribe patterns, implementations)
//! - Built-in events (battery, connection, input, state, split, etc.)
//!
//! All events use PubSubChannel for unified publish/subscribe semantics,
//! supporting multiple subscribers per event type.
//!
//! ## Module organization
//!
//! - `input`: Input events (keyboard, modifier, pointing device)
//! - `state`: Keyboard state events (layer, WPM, LED indicator, sleep)
//! - `battery`: Battery events (ADC, charging, battery status)
//! - `connection`: Connection events (USB/BLE, BLE status)
//! - `split`: Split keyboard events (peripheral/central connection)

use embassy_sync::blocking_mutex::raw::RawMutex;
use embassy_sync::pubsub::{ImmediatePublisher, Publisher, Subscriber};
use embassy_sync::{channel, watch};

/// Generates `Deref`, `From<Event> for Payload`, and `From<Payload> for Event`
/// for a newtype event struct wrapping a payload.
macro_rules! impl_payload_wrapper {
    ($event:ty, $payload:ty) => {
        impl core::ops::Deref for $event {
            type Target = $payload;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl From<$event> for $payload {
            fn from(event: $event) -> Self {
                event.0
            }
        }

        impl From<$payload> for $event {
            fn from(payload: $payload) -> Self {
                Self(payload)
            }
        }
    };
}

mod action;
mod battery;
mod connection;
#[cfg(feature = "dfu")]
mod dfu;
mod input;
#[cfg(feature = "split")]
mod split;
mod state;

pub use action::ActionEvent;
pub use battery::{BatteryAdcEvent, BatteryStatusEvent, ChargingStateEvent};
pub use connection::{ConnectionStatus, ConnectionStatusChangeEvent, ConnectionType};
#[cfg(feature = "dfu")]
pub use dfu::DfuStatusEvent;
pub use input::{
    Axis, AxisEvent, AxisValType, KeyPos, KeyboardEvent, KeyboardEventPos, ModifierEvent, PointingEvent,
    PointingProcessorEvent, PointingSetCpiEvent, RotaryEncoderPos,
};
#[cfg(all(feature = "split", feature = "_ble"))]
pub use split::ClearPeerEvent;
#[cfg(feature = "split")]
pub use split::{CentralConnectedEvent, PeripheralBatteryEvent, PeripheralConnectedEvent};
pub use state::{LayerChangeEvent, LedIndicatorEvent, LightingChangedEvent, SleepStateEvent, WpmUpdateEvent};

/// Trait for event publishers
pub trait EventPublisher {
    type Event;
    fn publish(&self, message: Self::Event);
}

/// Async version of event publisher trait
pub trait AsyncEventPublisher {
    type Event;
    async fn publish_async(&self, message: Self::Event);
}

/// Trait for event subscribers, event subscribers are always async
pub trait EventSubscriber {
    type Event;
    async fn next_event(&mut self) -> Self::Event;
}

/// Trait for events that can be published.
pub trait PublishableEvent: Clone + Send {
    type Publisher: EventPublisher<Event = Self>;
    /// If this is true, `publish_event` will not do anything.
    /// This is used for events that are not subscribed to by anything, to exclude the publishing from compiled code.
    const PUBLISH_IS_NOOP: bool;

    fn publisher() -> Self::Publisher;
}

/// Async version of publishable event trait.
pub trait AsyncPublishableEvent: PublishableEvent {
    type AsyncPublisher: AsyncEventPublisher<Event = Self>;

    fn publisher_async() -> Self::AsyncPublisher;
}

/// Trait for events that can be subscribed to.
pub trait SubscribableEvent: Clone + Send {
    type Subscriber: EventSubscriber<Event = Self>;

    fn subscriber() -> Self::Subscriber;
}

/// Combined trait for events that support both publish and subscribe.
///
/// Most concrete event types implement this trait.
pub trait Event: PublishableEvent + SubscribableEvent {}

// Auto-implement Event for types that implement both publish and subscribe
impl<T: PublishableEvent + SubscribableEvent> Event for T {}

/// Async version of event trait
pub trait AsyncEvent: Event + AsyncPublishableEvent {}

impl<T: Event + AsyncPublishableEvent> AsyncEvent for T {}

// Implementations for embassy-sync PubSubChannel
impl<'a, M: RawMutex, T: Clone, const CAP: usize, const SUBS: usize, const PUBS: usize> EventPublisher
    for ImmediatePublisher<'a, M, T, CAP, SUBS, PUBS>
{
    type Event = T;
    fn publish(&self, message: T) {
        self.publish_immediate(message);
    }
}

impl<'a, M: RawMutex, T: Clone, const CAP: usize, const SUBS: usize, const PUBS: usize> AsyncEventPublisher
    for Publisher<'a, M, T, CAP, SUBS, PUBS>
{
    type Event = T;
    async fn publish_async(&self, message: T) {
        self.publish(message).await
    }
}

impl<'a, M: RawMutex, T: Clone, const CAP: usize, const SUBS: usize, const PUBS: usize> EventSubscriber
    for Subscriber<'a, M, T, CAP, SUBS, PUBS>
{
    type Event = T;
    async fn next_event(&mut self) -> Self::Event {
        self.next_message_pure().await
    }
}

// Implementations for embassy-sync Watch
impl<'a, M: RawMutex, T: Clone, const N: usize> EventPublisher for watch::Sender<'a, M, T, N> {
    type Event = T;
    fn publish(&self, message: T) {
        self.send(message);
    }
}

impl<'a, M: RawMutex, T: Clone, const N: usize> EventSubscriber for watch::Receiver<'a, M, T, N> {
    type Event = T;
    // WARNING: it won't work when using `XEvent::subscriber().next_event().await`,
    // because `subscriber()` creates a new subscriber, which will immediately return when `changed()` is called.
    // A possible solution is to call `changed()` twice in `next_event()`, but it looks ugly.
    async fn next_event(&mut self) -> Self::Event {
        self.changed().await
    }
}

// Implementation for embassy-sync Channel
impl<'a, M: RawMutex, T: Clone, const N: usize> EventPublisher for channel::Sender<'a, M, T, N> {
    type Event = T;
    fn publish(&self, message: T) {
        if self.try_send(message).is_err() {
            error!("Send event to Channel error, channel is full");
        }
    }
}

impl<'a, M: RawMutex, T: Clone, const N: usize> AsyncEventPublisher for channel::Sender<'a, M, T, N> {
    type Event = T;
    async fn publish_async(&self, message: T) {
        self.send(message).await
    }
}

impl<'a, M: RawMutex, T: Clone, const N: usize> EventSubscriber for channel::Receiver<'a, M, T, N> {
    type Event = T;
    async fn next_event(&mut self) -> Self::Event {
        self.receive().await
    }
}

/// Publish an event (non-blocking, may drop if buffer full)
///
/// Example: `publish_event(KeyboardEvent::key(0, 0, true))`
pub fn publish_event<E: PublishableEvent>(e: E) {
    if !E::PUBLISH_IS_NOOP {
        E::publisher().publish(e);
    }
}

/// Publish an event with backpressure (waits if buffer full)
///
/// Example: `publish_event_async(KeyboardEvent::key(0, 0, true)).await`
pub async fn publish_event_async<E: AsyncPublishableEvent>(e: E) {
    if !E::PUBLISH_IS_NOOP {
        E::publisher_async().publish_async(e).await;
    }
}
