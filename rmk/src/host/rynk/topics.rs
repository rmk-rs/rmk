//! Rynk topic publishers.
//!
//! Topics are firmware -> host event publishing channels. The wire-facing
//! [`TopicEvent`] union and its `encode` live in `rmk-types`, generated from
//! the protocol's topic table; this module only subscribes to the internal
//! RMK events and forwards their values into it.

use futures::FutureExt;
use rmk_types::protocol::rynk::TopicEvent;

#[cfg(feature = "_ble")]
use crate::event::BatteryStatusEvent;
#[cfg(feature = "lighting")]
use crate::event::LightingChangedEvent;
use crate::event::{
    ConnectionStatusChangeEvent, EventSubscriber, LayerChangeEvent, LedIndicatorEvent, SleepStateEvent,
    SubscribableEvent, WpmUpdateEvent,
};

pub(super) struct TopicSubscribers {
    layer: <LayerChangeEvent as SubscribableEvent>::Subscriber,
    wpm: <WpmUpdateEvent as SubscribableEvent>::Subscriber,
    conn: <ConnectionStatusChangeEvent as SubscribableEvent>::Subscriber,
    sleep: <SleepStateEvent as SubscribableEvent>::Subscriber,
    led: <LedIndicatorEvent as SubscribableEvent>::Subscriber,
    #[cfg(feature = "_ble")]
    battery: <BatteryStatusEvent as SubscribableEvent>::Subscriber,
    #[cfg(feature = "lighting")]
    lighting: <LightingChangedEvent as SubscribableEvent>::Subscriber,
}

impl TopicSubscribers {
    pub(super) fn new() -> Self {
        Self {
            layer: LayerChangeEvent::subscriber(),
            wpm: WpmUpdateEvent::subscriber(),
            conn: ConnectionStatusChangeEvent::subscriber(),
            sleep: SleepStateEvent::subscriber(),
            led: LedIndicatorEvent::subscriber(),
            #[cfg(feature = "_ble")]
            battery: BatteryStatusEvent::subscriber(),
            #[cfg(feature = "lighting")]
            lighting: LightingChangedEvent::subscriber(),
        }
    }

    /// Await the next topic event to forward to the host.
    /// Each value's current snapshot is owned by its producer,
    /// this only forwards change notifications onto the wire.
    pub(super) async fn next_event(&mut self) -> TopicEvent {
        crate::select_biased_with_feature! {
            e = self.layer.next_event().fuse() => TopicEvent::LayerChange(*e),
            e = self.wpm.next_event().fuse() => TopicEvent::WpmUpdate(*e),
            e = self.conn.next_event().fuse() => TopicEvent::ConnectionChange(*e),
            e = self.sleep.next_event().fuse() => TopicEvent::SleepState(*e),
            e = self.led.next_event().fuse() => TopicEvent::LedIndicatorChange(*e),
            with_feature("_ble"): e = self.battery.next_event().fuse() => TopicEvent::BatteryStatusChange(*e),
            with_feature("lighting"): _ = self.lighting.next_event().fuse() => TopicEvent::LightingChange(rmk_types::protocol::rynk::LightingChanged),
        }
    }
}
