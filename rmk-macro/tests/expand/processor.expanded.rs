//! Expand tests for #[processor] macro.
//!
//! Tests:
//! - Single event subscription
//! - Multiple event subscription
//! - Polling processor with poll_interval
//! - Multiple #[processor] attributes (merged subscriptions)
use rmk_macro::processor;
pub struct KeyEvent {
    pub row: u8,
    pub col: u8,
    pub pressed: bool,
}
#[automatically_derived]
#[doc(hidden)]
unsafe impl ::core::clone::TrivialClone for KeyEvent {}
#[automatically_derived]
impl ::core::clone::Clone for KeyEvent {
    #[inline]
    fn clone(&self) -> KeyEvent {
        let _: ::core::clone::AssertParamIsClone<u8>;
        let _: ::core::clone::AssertParamIsClone<bool>;
        *self
    }
}
#[automatically_derived]
impl ::core::marker::Copy for KeyEvent {}
#[automatically_derived]
impl ::core::fmt::Debug for KeyEvent {
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field3_finish(
            f,
            "KeyEvent",
            "row",
            &self.row,
            "col",
            &self.col,
            "pressed",
            &&self.pressed,
        )
    }
}
pub struct EncoderEvent {
    pub index: u8,
    pub direction: i8,
}
#[automatically_derived]
#[doc(hidden)]
unsafe impl ::core::clone::TrivialClone for EncoderEvent {}
#[automatically_derived]
impl ::core::clone::Clone for EncoderEvent {
    #[inline]
    fn clone(&self) -> EncoderEvent {
        let _: ::core::clone::AssertParamIsClone<u8>;
        let _: ::core::clone::AssertParamIsClone<i8>;
        *self
    }
}
#[automatically_derived]
impl ::core::marker::Copy for EncoderEvent {}
#[automatically_derived]
impl ::core::fmt::Debug for EncoderEvent {
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field2_finish(
            f,
            "EncoderEvent",
            "index",
            &self.index,
            "direction",
            &&self.direction,
        )
    }
}
pub struct ConfigEvent {
    pub threshold: u16,
}
#[automatically_derived]
#[doc(hidden)]
unsafe impl ::core::clone::TrivialClone for ConfigEvent {}
#[automatically_derived]
impl ::core::clone::Clone for ConfigEvent {
    #[inline]
    fn clone(&self) -> ConfigEvent {
        let _: ::core::clone::AssertParamIsClone<u16>;
        *self
    }
}
#[automatically_derived]
impl ::core::marker::Copy for ConfigEvent {}
#[automatically_derived]
impl ::core::fmt::Debug for ConfigEvent {
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field1_finish(
            f,
            "ConfigEvent",
            "threshold",
            &&self.threshold,
        )
    }
}
/// Single event subscription
mod basic {
    use super::{KeyEvent, processor};
    pub struct SingleEventProcessor;
    impl ::rmk::processor::Processor for SingleEventProcessor {
        type Event = KeyEvent;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <KeyEvent as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            self.on_key_event(event).await
        }
    }
    impl ::rmk::core_traits::Runnable for SingleEventProcessor {
        async fn run(&mut self) -> ! {
            use ::rmk::processor::Processor;
            self.process_loop().await
        }
    }
}
/// Multiple event subscription
mod multi_sub {
    use super::{EncoderEvent, KeyEvent, processor};
    pub struct KeyProcessor;
    pub enum KeyProcessorProcessorEventEnum {
        Key(KeyEvent),
        Encoder(EncoderEvent),
    }
    #[automatically_derived]
    impl ::core::clone::Clone for KeyProcessorProcessorEventEnum {
        #[inline]
        fn clone(&self) -> KeyProcessorProcessorEventEnum {
            match self {
                KeyProcessorProcessorEventEnum::Key(__self_0) => {
                    KeyProcessorProcessorEventEnum::Key(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
                KeyProcessorProcessorEventEnum::Encoder(__self_0) => {
                    KeyProcessorProcessorEventEnum::Encoder(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
            }
        }
    }
    /// Event subscriber for aggregated events
    pub struct KeyProcessorProcessorEventSubscriber {
        sub0: <KeyEvent as ::rmk::event::SubscribableEvent>::Subscriber,
        sub1: <EncoderEvent as ::rmk::event::SubscribableEvent>::Subscriber,
    }
    impl KeyProcessorProcessorEventSubscriber {
        /// Create a new event subscriber
        pub fn new() -> Self {
            Self {
                sub0: <KeyEvent as ::rmk::event::SubscribableEvent>::subscriber(),
                sub1: <EncoderEvent as ::rmk::event::SubscribableEvent>::subscriber(),
            }
        }
    }
    impl ::rmk::event::EventSubscriber for KeyProcessorProcessorEventSubscriber {
        type Event = KeyProcessorProcessorEventEnum;
        async fn next_event(&mut self) -> Self::Event {
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            {
                use ::futures_util::__private as __futures_crate;
                {
                    enum __PrivResult<_0, _1> {
                        _0(_0),
                        _1(_1),
                    }
                    let __select_result = {
                        let mut _0 = self.sub0.next_event().fuse();
                        let mut _1 = self.sub1.next_event().fuse();
                        let mut __poll_fn = |
                            __cx: &mut __futures_crate::task::Context<'_>|
                        {
                            let mut __any_polled = false;
                            let mut _0 = |__cx: &mut __futures_crate::task::Context<'_>| {
                                let mut _0 = unsafe {
                                    __futures_crate::Pin::new_unchecked(&mut _0)
                                };
                                if __futures_crate::future::FusedFuture::is_terminated(
                                    &_0,
                                ) {
                                    __futures_crate::None
                                } else {
                                    __futures_crate::Some(
                                        __futures_crate::future::FutureExt::poll_unpin(
                                                &mut _0,
                                                __cx,
                                            )
                                            .map(__PrivResult::_0),
                                    )
                                }
                            };
                            let _0: &mut dyn FnMut(
                                &mut __futures_crate::task::Context<'_>,
                            ) -> __futures_crate::Option<
                                    __futures_crate::task::Poll<_>,
                                > = &mut _0;
                            let mut _1 = |__cx: &mut __futures_crate::task::Context<'_>| {
                                let mut _1 = unsafe {
                                    __futures_crate::Pin::new_unchecked(&mut _1)
                                };
                                if __futures_crate::future::FusedFuture::is_terminated(
                                    &_1,
                                ) {
                                    __futures_crate::None
                                } else {
                                    __futures_crate::Some(
                                        __futures_crate::future::FutureExt::poll_unpin(
                                                &mut _1,
                                                __cx,
                                            )
                                            .map(__PrivResult::_1),
                                    )
                                }
                            };
                            let _1: &mut dyn FnMut(
                                &mut __futures_crate::task::Context<'_>,
                            ) -> __futures_crate::Option<
                                    __futures_crate::task::Poll<_>,
                                > = &mut _1;
                            let mut __select_arr = [_0, _1];
                            for poller in &mut __select_arr {
                                let poller: &mut &mut dyn FnMut(
                                    &mut __futures_crate::task::Context<'_>,
                                ) -> __futures_crate::Option<
                                        __futures_crate::task::Poll<_>,
                                    > = poller;
                                match poller(__cx) {
                                    __futures_crate::Some(
                                        x @ __futures_crate::task::Poll::Ready(_),
                                    ) => return x,
                                    __futures_crate::Some(
                                        __futures_crate::task::Poll::Pending,
                                    ) => {
                                        __any_polled = true;
                                    }
                                    __futures_crate::None => {}
                                }
                            }
                            if !__any_polled {
                                {
                                    ::std::rt::begin_panic(
                                        "all futures in select! were completed, \
                    but no `complete =>` handler was provided",
                                    );
                                }
                            } else {
                                __futures_crate::task::Poll::Pending
                            }
                        };
                        __futures_crate::future::poll_fn(__poll_fn).await
                    };
                    match __select_result {
                        __PrivResult::_0(event) => {
                            KeyProcessorProcessorEventEnum::Key(event)
                        }
                        __PrivResult::_1(event) => {
                            KeyProcessorProcessorEventEnum::Encoder(event)
                        }
                    }
                }
            }
        }
    }
    impl ::rmk::event::SubscribableEvent for KeyProcessorProcessorEventEnum {
        type Subscriber = KeyProcessorProcessorEventSubscriber;
        fn subscriber() -> Self::Subscriber {
            KeyProcessorProcessorEventSubscriber::new()
        }
    }
    impl ::rmk::processor::Processor for KeyProcessor {
        type Event = KeyProcessorProcessorEventEnum;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <KeyProcessorProcessorEventEnum as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {
                KeyProcessorProcessorEventEnum::Key(event) => {
                    self.on_key_event(event).await
                }
                KeyProcessorProcessorEventEnum::Encoder(event) => {
                    self.on_encoder_event(event).await
                }
            }
        }
    }
    impl ::rmk::core_traits::Runnable for KeyProcessor {
        async fn run(&mut self) -> ! {
            use ::rmk::processor::Processor;
            self.process_loop().await
        }
    }
}
/// Polling processor
mod polling {
    use super::{ConfigEvent, processor};
    pub struct PollingProcessor {
        pub counter: u32,
    }
    impl ::rmk::processor::Processor for PollingProcessor {
        type Event = ConfigEvent;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <ConfigEvent as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            self.on_config_event(event).await
        }
    }
    impl ::rmk::processor::PollingProcessor for PollingProcessor {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(100u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::core_traits::Runnable for PollingProcessor {
        async fn run(&mut self) -> ! {
            use ::rmk::processor::PollingProcessor;
            self.polling_loop().await
        }
    }
}
/// Multiple #[processor] attributes (simulates cfg_attr expansion)
mod multi_attr {
    use super::{EncoderEvent, KeyEvent, processor};
    pub struct MultiAttrProcessor;
    pub enum MultiAttrProcessorProcessorEventEnum {
        Key(KeyEvent),
        Encoder(EncoderEvent),
    }
    #[automatically_derived]
    impl ::core::clone::Clone for MultiAttrProcessorProcessorEventEnum {
        #[inline]
        fn clone(&self) -> MultiAttrProcessorProcessorEventEnum {
            match self {
                MultiAttrProcessorProcessorEventEnum::Key(__self_0) => {
                    MultiAttrProcessorProcessorEventEnum::Key(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
                MultiAttrProcessorProcessorEventEnum::Encoder(__self_0) => {
                    MultiAttrProcessorProcessorEventEnum::Encoder(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
            }
        }
    }
    /// Event subscriber for aggregated events
    pub struct MultiAttrProcessorProcessorEventSubscriber {
        sub0: <KeyEvent as ::rmk::event::SubscribableEvent>::Subscriber,
        sub1: <EncoderEvent as ::rmk::event::SubscribableEvent>::Subscriber,
    }
    impl MultiAttrProcessorProcessorEventSubscriber {
        /// Create a new event subscriber
        pub fn new() -> Self {
            Self {
                sub0: <KeyEvent as ::rmk::event::SubscribableEvent>::subscriber(),
                sub1: <EncoderEvent as ::rmk::event::SubscribableEvent>::subscriber(),
            }
        }
    }
    impl ::rmk::event::EventSubscriber for MultiAttrProcessorProcessorEventSubscriber {
        type Event = MultiAttrProcessorProcessorEventEnum;
        async fn next_event(&mut self) -> Self::Event {
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            {
                use ::futures_util::__private as __futures_crate;
                {
                    enum __PrivResult<_0, _1> {
                        _0(_0),
                        _1(_1),
                    }
                    let __select_result = {
                        let mut _0 = self.sub0.next_event().fuse();
                        let mut _1 = self.sub1.next_event().fuse();
                        let mut __poll_fn = |
                            __cx: &mut __futures_crate::task::Context<'_>|
                        {
                            let mut __any_polled = false;
                            let mut _0 = |__cx: &mut __futures_crate::task::Context<'_>| {
                                let mut _0 = unsafe {
                                    __futures_crate::Pin::new_unchecked(&mut _0)
                                };
                                if __futures_crate::future::FusedFuture::is_terminated(
                                    &_0,
                                ) {
                                    __futures_crate::None
                                } else {
                                    __futures_crate::Some(
                                        __futures_crate::future::FutureExt::poll_unpin(
                                                &mut _0,
                                                __cx,
                                            )
                                            .map(__PrivResult::_0),
                                    )
                                }
                            };
                            let _0: &mut dyn FnMut(
                                &mut __futures_crate::task::Context<'_>,
                            ) -> __futures_crate::Option<
                                    __futures_crate::task::Poll<_>,
                                > = &mut _0;
                            let mut _1 = |__cx: &mut __futures_crate::task::Context<'_>| {
                                let mut _1 = unsafe {
                                    __futures_crate::Pin::new_unchecked(&mut _1)
                                };
                                if __futures_crate::future::FusedFuture::is_terminated(
                                    &_1,
                                ) {
                                    __futures_crate::None
                                } else {
                                    __futures_crate::Some(
                                        __futures_crate::future::FutureExt::poll_unpin(
                                                &mut _1,
                                                __cx,
                                            )
                                            .map(__PrivResult::_1),
                                    )
                                }
                            };
                            let _1: &mut dyn FnMut(
                                &mut __futures_crate::task::Context<'_>,
                            ) -> __futures_crate::Option<
                                    __futures_crate::task::Poll<_>,
                                > = &mut _1;
                            let mut __select_arr = [_0, _1];
                            for poller in &mut __select_arr {
                                let poller: &mut &mut dyn FnMut(
                                    &mut __futures_crate::task::Context<'_>,
                                ) -> __futures_crate::Option<
                                        __futures_crate::task::Poll<_>,
                                    > = poller;
                                match poller(__cx) {
                                    __futures_crate::Some(
                                        x @ __futures_crate::task::Poll::Ready(_),
                                    ) => return x,
                                    __futures_crate::Some(
                                        __futures_crate::task::Poll::Pending,
                                    ) => {
                                        __any_polled = true;
                                    }
                                    __futures_crate::None => {}
                                }
                            }
                            if !__any_polled {
                                {
                                    ::std::rt::begin_panic(
                                        "all futures in select! were completed, \
                    but no `complete =>` handler was provided",
                                    );
                                }
                            } else {
                                __futures_crate::task::Poll::Pending
                            }
                        };
                        __futures_crate::future::poll_fn(__poll_fn).await
                    };
                    match __select_result {
                        __PrivResult::_0(event) => {
                            MultiAttrProcessorProcessorEventEnum::Key(event)
                        }
                        __PrivResult::_1(event) => {
                            MultiAttrProcessorProcessorEventEnum::Encoder(event)
                        }
                    }
                }
            }
        }
    }
    impl ::rmk::event::SubscribableEvent for MultiAttrProcessorProcessorEventEnum {
        type Subscriber = MultiAttrProcessorProcessorEventSubscriber;
        fn subscriber() -> Self::Subscriber {
            MultiAttrProcessorProcessorEventSubscriber::new()
        }
    }
    impl ::rmk::processor::Processor for MultiAttrProcessor {
        type Event = MultiAttrProcessorProcessorEventEnum;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <MultiAttrProcessorProcessorEventEnum as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {
                MultiAttrProcessorProcessorEventEnum::Key(event) => {
                    self.on_key_event(event).await
                }
                MultiAttrProcessorProcessorEventEnum::Encoder(event) => {
                    self.on_encoder_event(event).await
                }
            }
        }
    }
    impl ::rmk::processor::PollingProcessor for MultiAttrProcessor {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(100u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::core_traits::Runnable for MultiAttrProcessor {
        async fn run(&mut self) -> ! {
            ::rmk::processor::DeadlineProcessor::polling_deadline_loop(self).await
        }
    }
}
/// Polling processor with multiple events
mod polling_multi {
    use super::{ConfigEvent, EncoderEvent, KeyEvent, processor};
    pub struct MultiPollingProcessor {
        pub state: u8,
    }
    pub enum MultiPollingProcessorProcessorEventEnum {
        Key(KeyEvent),
        Encoder(EncoderEvent),
        Config(ConfigEvent),
    }
    #[automatically_derived]
    impl ::core::clone::Clone for MultiPollingProcessorProcessorEventEnum {
        #[inline]
        fn clone(&self) -> MultiPollingProcessorProcessorEventEnum {
            match self {
                MultiPollingProcessorProcessorEventEnum::Key(__self_0) => {
                    MultiPollingProcessorProcessorEventEnum::Key(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
                MultiPollingProcessorProcessorEventEnum::Encoder(__self_0) => {
                    MultiPollingProcessorProcessorEventEnum::Encoder(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
                MultiPollingProcessorProcessorEventEnum::Config(__self_0) => {
                    MultiPollingProcessorProcessorEventEnum::Config(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
            }
        }
    }
    /// Event subscriber for aggregated events
    pub struct MultiPollingProcessorProcessorEventSubscriber {
        sub0: <KeyEvent as ::rmk::event::SubscribableEvent>::Subscriber,
        sub1: <EncoderEvent as ::rmk::event::SubscribableEvent>::Subscriber,
        sub2: <ConfigEvent as ::rmk::event::SubscribableEvent>::Subscriber,
    }
    impl MultiPollingProcessorProcessorEventSubscriber {
        /// Create a new event subscriber
        pub fn new() -> Self {
            Self {
                sub0: <KeyEvent as ::rmk::event::SubscribableEvent>::subscriber(),
                sub1: <EncoderEvent as ::rmk::event::SubscribableEvent>::subscriber(),
                sub2: <ConfigEvent as ::rmk::event::SubscribableEvent>::subscriber(),
            }
        }
    }
    impl ::rmk::event::EventSubscriber
    for MultiPollingProcessorProcessorEventSubscriber {
        type Event = MultiPollingProcessorProcessorEventEnum;
        async fn next_event(&mut self) -> Self::Event {
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            {
                use ::futures_util::__private as __futures_crate;
                {
                    enum __PrivResult<_0, _1, _2> {
                        _0(_0),
                        _1(_1),
                        _2(_2),
                    }
                    let __select_result = {
                        let mut _0 = self.sub0.next_event().fuse();
                        let mut _1 = self.sub1.next_event().fuse();
                        let mut _2 = self.sub2.next_event().fuse();
                        let mut __poll_fn = |
                            __cx: &mut __futures_crate::task::Context<'_>|
                        {
                            let mut __any_polled = false;
                            let mut _0 = |__cx: &mut __futures_crate::task::Context<'_>| {
                                let mut _0 = unsafe {
                                    __futures_crate::Pin::new_unchecked(&mut _0)
                                };
                                if __futures_crate::future::FusedFuture::is_terminated(
                                    &_0,
                                ) {
                                    __futures_crate::None
                                } else {
                                    __futures_crate::Some(
                                        __futures_crate::future::FutureExt::poll_unpin(
                                                &mut _0,
                                                __cx,
                                            )
                                            .map(__PrivResult::_0),
                                    )
                                }
                            };
                            let _0: &mut dyn FnMut(
                                &mut __futures_crate::task::Context<'_>,
                            ) -> __futures_crate::Option<
                                    __futures_crate::task::Poll<_>,
                                > = &mut _0;
                            let mut _1 = |__cx: &mut __futures_crate::task::Context<'_>| {
                                let mut _1 = unsafe {
                                    __futures_crate::Pin::new_unchecked(&mut _1)
                                };
                                if __futures_crate::future::FusedFuture::is_terminated(
                                    &_1,
                                ) {
                                    __futures_crate::None
                                } else {
                                    __futures_crate::Some(
                                        __futures_crate::future::FutureExt::poll_unpin(
                                                &mut _1,
                                                __cx,
                                            )
                                            .map(__PrivResult::_1),
                                    )
                                }
                            };
                            let _1: &mut dyn FnMut(
                                &mut __futures_crate::task::Context<'_>,
                            ) -> __futures_crate::Option<
                                    __futures_crate::task::Poll<_>,
                                > = &mut _1;
                            let mut _2 = |__cx: &mut __futures_crate::task::Context<'_>| {
                                let mut _2 = unsafe {
                                    __futures_crate::Pin::new_unchecked(&mut _2)
                                };
                                if __futures_crate::future::FusedFuture::is_terminated(
                                    &_2,
                                ) {
                                    __futures_crate::None
                                } else {
                                    __futures_crate::Some(
                                        __futures_crate::future::FutureExt::poll_unpin(
                                                &mut _2,
                                                __cx,
                                            )
                                            .map(__PrivResult::_2),
                                    )
                                }
                            };
                            let _2: &mut dyn FnMut(
                                &mut __futures_crate::task::Context<'_>,
                            ) -> __futures_crate::Option<
                                    __futures_crate::task::Poll<_>,
                                > = &mut _2;
                            let mut __select_arr = [_0, _1, _2];
                            for poller in &mut __select_arr {
                                let poller: &mut &mut dyn FnMut(
                                    &mut __futures_crate::task::Context<'_>,
                                ) -> __futures_crate::Option<
                                        __futures_crate::task::Poll<_>,
                                    > = poller;
                                match poller(__cx) {
                                    __futures_crate::Some(
                                        x @ __futures_crate::task::Poll::Ready(_),
                                    ) => return x,
                                    __futures_crate::Some(
                                        __futures_crate::task::Poll::Pending,
                                    ) => {
                                        __any_polled = true;
                                    }
                                    __futures_crate::None => {}
                                }
                            }
                            if !__any_polled {
                                {
                                    ::std::rt::begin_panic(
                                        "all futures in select! were completed, \
                    but no `complete =>` handler was provided",
                                    );
                                }
                            } else {
                                __futures_crate::task::Poll::Pending
                            }
                        };
                        __futures_crate::future::poll_fn(__poll_fn).await
                    };
                    match __select_result {
                        __PrivResult::_0(event) => {
                            MultiPollingProcessorProcessorEventEnum::Key(event)
                        }
                        __PrivResult::_1(event) => {
                            MultiPollingProcessorProcessorEventEnum::Encoder(event)
                        }
                        __PrivResult::_2(event) => {
                            MultiPollingProcessorProcessorEventEnum::Config(event)
                        }
                    }
                }
            }
        }
    }
    impl ::rmk::event::SubscribableEvent for MultiPollingProcessorProcessorEventEnum {
        type Subscriber = MultiPollingProcessorProcessorEventSubscriber;
        fn subscriber() -> Self::Subscriber {
            MultiPollingProcessorProcessorEventSubscriber::new()
        }
    }
    impl ::rmk::processor::Processor for MultiPollingProcessor {
        type Event = MultiPollingProcessorProcessorEventEnum;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <MultiPollingProcessorProcessorEventEnum as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {
                MultiPollingProcessorProcessorEventEnum::Key(event) => {
                    self.on_key_event(event).await
                }
                MultiPollingProcessorProcessorEventEnum::Encoder(event) => {
                    self.on_encoder_event(event).await
                }
                MultiPollingProcessorProcessorEventEnum::Config(event) => {
                    self.on_config_event(event).await
                }
            }
        }
    }
    impl ::rmk::processor::PollingProcessor for MultiPollingProcessor {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(50u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::core_traits::Runnable for MultiPollingProcessor {
        async fn run(&mut self) -> ! {
            use ::rmk::processor::PollingProcessor;
            self.polling_loop().await
        }
    }
}
mod timer_only {
    use super::processor;
    pub struct Polling;
    impl ::rmk::processor::Processor for Polling {
        type Event = ::core::convert::Infallible;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            ::core::future::pending::<Self::Event>()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {}
        }
    }
    impl ::rmk::processor::PollingProcessor for Polling {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(100u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::core_traits::Runnable for Polling {
        async fn run(&mut self) -> ! {
            use ::rmk::processor::PollingProcessor;
            self.polling_loop().await
        }
    }
    pub struct Deadline;
    impl ::rmk::processor::Processor for Deadline {
        type Event = ::core::convert::Infallible;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            ::core::future::pending::<Self::Event>()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {}
        }
    }
    impl ::rmk::core_traits::Runnable for Deadline {
        async fn run(&mut self) -> ! {
            ::rmk::processor::DeadlineProcessor::deadline_loop(self).await
        }
    }
    pub struct Custom;
    impl ::rmk::processor::Processor for Custom {
        type Event = ::core::convert::Infallible;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            ::core::future::pending::<Self::Event>()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {}
        }
    }
}
