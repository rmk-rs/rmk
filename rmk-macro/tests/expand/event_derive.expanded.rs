use rmk_macro::{Event, event};
pub struct BatteryEvent {
    pub level: u8,
}
#[automatically_derived]
#[doc(hidden)]
unsafe impl ::core::clone::TrivialClone for BatteryEvent {}
#[automatically_derived]
impl ::core::clone::Clone for BatteryEvent {
    #[inline]
    fn clone(&self) -> BatteryEvent {
        let _: ::core::clone::AssertParamIsClone<u8>;
        *self
    }
}
#[automatically_derived]
impl ::core::marker::Copy for BatteryEvent {}
#[automatically_derived]
impl ::core::fmt::Debug for BatteryEvent {
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field1_finish(
            f,
            "BatteryEvent",
            "level",
            &&self.level,
        )
    }
}
#[doc(hidden)]
static BATTERY_EVENT_EVENT_CHANNEL: ::embassy_sync::channel::Channel<
    ::rmk::RawMutex,
    BatteryEvent,
    { 8 },
> = ::embassy_sync::channel::Channel::new();
impl ::rmk::event::PublishableEvent for BatteryEvent {
    type Publisher = ::embassy_sync::channel::Sender<
        'static,
        ::rmk::RawMutex,
        BatteryEvent,
        { 8 },
    >;
    const PUBLISH_IS_NOOP: bool = false;
    fn publisher() -> Self::Publisher {
        BATTERY_EVENT_EVENT_CHANNEL.sender()
    }
}
impl ::rmk::event::SubscribableEvent for BatteryEvent {
    type Subscriber = ::embassy_sync::channel::Receiver<
        'static,
        ::rmk::RawMutex,
        BatteryEvent,
        { 8 },
    >;
    fn subscriber() -> Self::Subscriber {
        BATTERY_EVENT_EVENT_CHANNEL.receiver()
    }
}
impl ::rmk::event::AsyncPublishableEvent for BatteryEvent {
    type AsyncPublisher = ::embassy_sync::channel::Sender<
        'static,
        ::rmk::RawMutex,
        BatteryEvent,
        { 8 },
    >;
    fn publisher_async() -> Result<Self::AsyncPublisher, ::embassy_sync::pubsub::Error> {
        Ok(BATTERY_EVENT_EVENT_CHANNEL.sender())
    }
}
pub struct PointingEvent {
    pub x: i16,
    pub y: i16,
}
#[automatically_derived]
#[doc(hidden)]
unsafe impl ::core::clone::TrivialClone for PointingEvent {}
#[automatically_derived]
impl ::core::clone::Clone for PointingEvent {
    #[inline]
    fn clone(&self) -> PointingEvent {
        let _: ::core::clone::AssertParamIsClone<i16>;
        *self
    }
}
#[automatically_derived]
impl ::core::marker::Copy for PointingEvent {}
#[automatically_derived]
impl ::core::fmt::Debug for PointingEvent {
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field2_finish(
            f,
            "PointingEvent",
            "x",
            &self.x,
            "y",
            &&self.y,
        )
    }
}
#[doc(hidden)]
static POINTING_EVENT_EVENT_CHANNEL: ::embassy_sync::channel::Channel<
    ::rmk::RawMutex,
    PointingEvent,
    { 8 },
> = ::embassy_sync::channel::Channel::new();
impl ::rmk::event::PublishableEvent for PointingEvent {
    type Publisher = ::embassy_sync::channel::Sender<
        'static,
        ::rmk::RawMutex,
        PointingEvent,
        { 8 },
    >;
    const PUBLISH_IS_NOOP: bool = false;
    fn publisher() -> Self::Publisher {
        POINTING_EVENT_EVENT_CHANNEL.sender()
    }
}
impl ::rmk::event::SubscribableEvent for PointingEvent {
    type Subscriber = ::embassy_sync::channel::Receiver<
        'static,
        ::rmk::RawMutex,
        PointingEvent,
        { 8 },
    >;
    fn subscriber() -> Self::Subscriber {
        POINTING_EVENT_EVENT_CHANNEL.receiver()
    }
}
impl ::rmk::event::AsyncPublishableEvent for PointingEvent {
    type AsyncPublisher = ::embassy_sync::channel::Sender<
        'static,
        ::rmk::RawMutex,
        PointingEvent,
        { 8 },
    >;
    fn publisher_async() -> Result<Self::AsyncPublisher, ::embassy_sync::pubsub::Error> {
        Ok(POINTING_EVENT_EVENT_CHANNEL.sender())
    }
}
pub enum MultiSensorEvent {
    Battery(BatteryEvent),
    Pointing(PointingEvent),
}
/// Publisher for the wrapper enum.
/// Routes each variant to its event channel.
pub struct MultiSensorEventPublisher;
impl ::rmk::event::AsyncEventPublisher for MultiSensorEventPublisher {
    type Event = MultiSensorEvent;
    async fn publish_async(&self, event: MultiSensorEvent) {
        match event {
            MultiSensorEvent::Battery(e) => ::rmk::event::publish_event_async(e).await,
            MultiSensorEvent::Pointing(e) => ::rmk::event::publish_event_async(e).await,
        }
    }
}
impl ::rmk::event::EventPublisher for MultiSensorEventPublisher {
    type Event = MultiSensorEvent;
    fn publish(&self, event: MultiSensorEvent) {
        match event {
            MultiSensorEvent::Battery(e) => ::rmk::event::publish_event(e),
            MultiSensorEvent::Pointing(e) => ::rmk::event::publish_event(e),
        }
    }
}
impl ::rmk::event::PublishableEvent for MultiSensorEvent {
    type Publisher = MultiSensorEventPublisher;
    const PUBLISH_IS_NOOP: bool = true
        && (<BatteryEvent as ::rmk::event::PublishableEvent>::PUBLISH_IS_NOOP)
        && (<PointingEvent as ::rmk::event::PublishableEvent>::PUBLISH_IS_NOOP);
    fn publisher() -> Self::Publisher {
        MultiSensorEventPublisher
    }
}
impl ::rmk::event::AsyncPublishableEvent for MultiSensorEvent {
    type AsyncPublisher = MultiSensorEventPublisher;
    fn publisher_async() -> Result<Self::AsyncPublisher, ::embassy_sync::pubsub::Error> {
        Ok(MultiSensorEventPublisher)
    }
}
impl From<BatteryEvent> for MultiSensorEvent {
    fn from(e: BatteryEvent) -> Self {
        MultiSensorEvent::Battery(e)
    }
}
impl From<PointingEvent> for MultiSensorEvent {
    fn from(e: PointingEvent) -> Self {
        MultiSensorEvent::Pointing(e)
    }
}
