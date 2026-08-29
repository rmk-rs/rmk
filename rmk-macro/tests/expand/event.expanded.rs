//! Expand tests for #[event] macro.
//!
//! Tests:
//! - MPSC channel (default, single consumer)
//! - PubSub channel (with subs/pubs parameters)
use rmk_macro::event;
/// MPSC channel event (single consumer)
mod mpsc {
    use super::event;
    pub struct KeyboardEvent {
        pub row: u8,
        pub col: u8,
        pub pressed: bool,
    }
    #[automatically_derived]
    #[doc(hidden)]
    unsafe impl ::core::clone::TrivialClone for KeyboardEvent {}
    #[automatically_derived]
    impl ::core::clone::Clone for KeyboardEvent {
        #[inline]
        fn clone(&self) -> KeyboardEvent {
            let _: ::core::clone::AssertParamIsClone<u8>;
            let _: ::core::clone::AssertParamIsClone<bool>;
            *self
        }
    }
    #[automatically_derived]
    impl ::core::marker::Copy for KeyboardEvent {}
    #[automatically_derived]
    impl ::core::fmt::Debug for KeyboardEvent {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field3_finish(
                f,
                "KeyboardEvent",
                "row",
                &self.row,
                "col",
                &self.col,
                "pressed",
                &&self.pressed,
            )
        }
    }
    #[doc(hidden)]
    static KEYBOARD_EVENT_EVENT_CHANNEL: ::embassy_sync::channel::Channel<
        ::rmk::RawMutex,
        KeyboardEvent,
        { 16 },
    > = ::embassy_sync::channel::Channel::new();
    impl ::rmk::event::PublishableEvent for KeyboardEvent {
        type Publisher = ::embassy_sync::channel::Sender<
            'static,
            ::rmk::RawMutex,
            KeyboardEvent,
            { 16 },
        >;
        const PUBLISH_IS_NOOP: bool = false;
        fn publisher() -> Self::Publisher {
            KEYBOARD_EVENT_EVENT_CHANNEL.sender()
        }
    }
    impl ::rmk::event::SubscribableEvent for KeyboardEvent {
        type Subscriber = ::embassy_sync::channel::Receiver<
            'static,
            ::rmk::RawMutex,
            KeyboardEvent,
            { 16 },
        >;
        fn subscriber() -> Self::Subscriber {
            KEYBOARD_EVENT_EVENT_CHANNEL.receiver()
        }
    }
    impl ::rmk::event::AsyncPublishableEvent for KeyboardEvent {
        type AsyncPublisher = ::embassy_sync::channel::Sender<
            'static,
            ::rmk::RawMutex,
            KeyboardEvent,
            { 16 },
        >;
        fn publisher_async() -> Result<
            Self::AsyncPublisher,
            ::embassy_sync::pubsub::Error,
        > {
            Ok(KEYBOARD_EVENT_EVENT_CHANNEL.sender())
        }
    }
}
/// PubSub channel event (multiple subscribers)
mod pubsub {
    use super::event;
    pub struct LedIndicatorEvent {
        pub caps_lock: bool,
        pub num_lock: bool,
        pub scroll_lock: bool,
    }
    #[automatically_derived]
    #[doc(hidden)]
    unsafe impl ::core::clone::TrivialClone for LedIndicatorEvent {}
    #[automatically_derived]
    impl ::core::clone::Clone for LedIndicatorEvent {
        #[inline]
        fn clone(&self) -> LedIndicatorEvent {
            let _: ::core::clone::AssertParamIsClone<bool>;
            *self
        }
    }
    #[automatically_derived]
    impl ::core::marker::Copy for LedIndicatorEvent {}
    #[automatically_derived]
    impl ::core::fmt::Debug for LedIndicatorEvent {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field3_finish(
                f,
                "LedIndicatorEvent",
                "caps_lock",
                &self.caps_lock,
                "num_lock",
                &self.num_lock,
                "scroll_lock",
                &&self.scroll_lock,
            )
        }
    }
    #[doc(hidden)]
    static LED_INDICATOR_EVENT_EVENT_CHANNEL: ::embassy_sync::pubsub::PubSubChannel<
        ::rmk::RawMutex,
        LedIndicatorEvent,
        { 4 },
        { 8 },
        { 2 },
    > = ::embassy_sync::pubsub::PubSubChannel::new();
    impl ::rmk::event::PublishableEvent for LedIndicatorEvent {
        type Publisher = ::embassy_sync::pubsub::ImmediatePublisher<
            'static,
            ::rmk::RawMutex,
            LedIndicatorEvent,
            { 4 },
            { 8 },
            { 2 },
        >;
        const PUBLISH_IS_NOOP: bool = 8 == 0;
        fn publisher() -> Self::Publisher {
            LED_INDICATOR_EVENT_EVENT_CHANNEL.immediate_publisher()
        }
    }
    impl ::rmk::event::SubscribableEvent for LedIndicatorEvent {
        type Subscriber = ::embassy_sync::pubsub::Subscriber<
            'static,
            ::rmk::RawMutex,
            LedIndicatorEvent,
            { 4 },
            { 8 },
            { 2 },
        >;
        fn subscriber() -> Self::Subscriber {
            LED_INDICATOR_EVENT_EVENT_CHANNEL
                .subscriber()
                .expect(
                    "Failed to create subscriber for LedIndicatorEvent. The \'subs\' limit has been exceeded. Increase \'subs\' for this event in keyboard.toml [event] (or #[event(subs = N)] for custom events).",
                )
        }
    }
    impl ::rmk::event::AsyncPublishableEvent for LedIndicatorEvent {
        type AsyncPublisher = ::embassy_sync::pubsub::Publisher<
            'static,
            ::rmk::RawMutex,
            LedIndicatorEvent,
            { 4 },
            { 8 },
            { 2 },
        >;
        fn publisher_async() -> Result<
            Self::AsyncPublisher,
            ::embassy_sync::pubsub::Error,
        > {
            LED_INDICATOR_EVENT_EVENT_CHANNEL.publisher()
        }
    }
}
/// Tuple struct event
mod tuple_struct {
    use super::event;
    pub struct BatteryAdcEvent(pub u16);
    #[automatically_derived]
    #[doc(hidden)]
    unsafe impl ::core::clone::TrivialClone for BatteryAdcEvent {}
    #[automatically_derived]
    impl ::core::clone::Clone for BatteryAdcEvent {
        #[inline]
        fn clone(&self) -> BatteryAdcEvent {
            let _: ::core::clone::AssertParamIsClone<u16>;
            *self
        }
    }
    #[automatically_derived]
    impl ::core::marker::Copy for BatteryAdcEvent {}
    #[automatically_derived]
    impl ::core::fmt::Debug for BatteryAdcEvent {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_tuple_field1_finish(
                f,
                "BatteryAdcEvent",
                &&self.0,
            )
        }
    }
    #[doc(hidden)]
    static BATTERY_ADC_EVENT_EVENT_CHANNEL: ::embassy_sync::channel::Channel<
        ::rmk::RawMutex,
        BatteryAdcEvent,
        { 8 },
    > = ::embassy_sync::channel::Channel::new();
    impl ::rmk::event::PublishableEvent for BatteryAdcEvent {
        type Publisher = ::embassy_sync::channel::Sender<
            'static,
            ::rmk::RawMutex,
            BatteryAdcEvent,
            { 8 },
        >;
        const PUBLISH_IS_NOOP: bool = false;
        fn publisher() -> Self::Publisher {
            BATTERY_ADC_EVENT_EVENT_CHANNEL.sender()
        }
    }
    impl ::rmk::event::SubscribableEvent for BatteryAdcEvent {
        type Subscriber = ::embassy_sync::channel::Receiver<
            'static,
            ::rmk::RawMutex,
            BatteryAdcEvent,
            { 8 },
        >;
        fn subscriber() -> Self::Subscriber {
            BATTERY_ADC_EVENT_EVENT_CHANNEL.receiver()
        }
    }
    impl ::rmk::event::AsyncPublishableEvent for BatteryAdcEvent {
        type AsyncPublisher = ::embassy_sync::channel::Sender<
            'static,
            ::rmk::RawMutex,
            BatteryAdcEvent,
            { 8 },
        >;
        fn publisher_async() -> Result<
            Self::AsyncPublisher,
            ::embassy_sync::pubsub::Error,
        > {
            Ok(BATTERY_ADC_EVENT_EVENT_CHANNEL.sender())
        }
    }
}
/// Event with default channel size
mod default_size {
    use super::event;
    pub struct LayerChangeEvent {
        pub layer: u8,
    }
    #[automatically_derived]
    #[doc(hidden)]
    unsafe impl ::core::clone::TrivialClone for LayerChangeEvent {}
    #[automatically_derived]
    impl ::core::clone::Clone for LayerChangeEvent {
        #[inline]
        fn clone(&self) -> LayerChangeEvent {
            let _: ::core::clone::AssertParamIsClone<u8>;
            *self
        }
    }
    #[automatically_derived]
    impl ::core::marker::Copy for LayerChangeEvent {}
    #[automatically_derived]
    impl ::core::fmt::Debug for LayerChangeEvent {
        #[inline]
        fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
            ::core::fmt::Formatter::debug_struct_field1_finish(
                f,
                "LayerChangeEvent",
                "layer",
                &&self.layer,
            )
        }
    }
    #[doc(hidden)]
    static LAYER_CHANGE_EVENT_EVENT_CHANNEL: ::embassy_sync::channel::Channel<
        ::rmk::RawMutex,
        LayerChangeEvent,
        { 8 },
    > = ::embassy_sync::channel::Channel::new();
    impl ::rmk::event::PublishableEvent for LayerChangeEvent {
        type Publisher = ::embassy_sync::channel::Sender<
            'static,
            ::rmk::RawMutex,
            LayerChangeEvent,
            { 8 },
        >;
        const PUBLISH_IS_NOOP: bool = false;
        fn publisher() -> Self::Publisher {
            LAYER_CHANGE_EVENT_EVENT_CHANNEL.sender()
        }
    }
    impl ::rmk::event::SubscribableEvent for LayerChangeEvent {
        type Subscriber = ::embassy_sync::channel::Receiver<
            'static,
            ::rmk::RawMutex,
            LayerChangeEvent,
            { 8 },
        >;
        fn subscriber() -> Self::Subscriber {
            LAYER_CHANGE_EVENT_EVENT_CHANNEL.receiver()
        }
    }
    impl ::rmk::event::AsyncPublishableEvent for LayerChangeEvent {
        type AsyncPublisher = ::embassy_sync::channel::Sender<
            'static,
            ::rmk::RawMutex,
            LayerChangeEvent,
            { 8 },
        >;
        fn publisher_async() -> Result<
            Self::AsyncPublisher,
            ::embassy_sync::pubsub::Error,
        > {
            Ok(LAYER_CHANGE_EVENT_EVENT_CHANNEL.sender())
        }
    }
}
