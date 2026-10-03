//! Expand tests for combined #[processor] + #[input_device] macros.
//!
//! Tests:
//! - Input device with processor subscription
//! - Polling processor with input device
use rmk_macro::{input_device, processor};
pub struct SensorEvent {
    pub value: u16,
}
#[automatically_derived]
#[doc(hidden)]
unsafe impl ::core::clone::TrivialClone for SensorEvent {}
#[automatically_derived]
impl ::core::clone::Clone for SensorEvent {
    #[inline]
    fn clone(&self) -> SensorEvent {
        let _: ::core::clone::AssertParamIsClone<u16>;
        *self
    }
}
#[automatically_derived]
impl ::core::marker::Copy for SensorEvent {}
#[automatically_derived]
impl ::core::fmt::Debug for SensorEvent {
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field1_finish(
            f,
            "SensorEvent",
            "value",
            &&self.value,
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
pub struct ModeEvent {
    pub mode: u8,
}
#[automatically_derived]
#[doc(hidden)]
unsafe impl ::core::clone::TrivialClone for ModeEvent {}
#[automatically_derived]
impl ::core::clone::Clone for ModeEvent {
    #[inline]
    fn clone(&self) -> ModeEvent {
        let _: ::core::clone::AssertParamIsClone<u8>;
        *self
    }
}
#[automatically_derived]
impl ::core::marker::Copy for ModeEvent {}
#[automatically_derived]
impl ::core::fmt::Debug for ModeEvent {
    #[inline]
    fn fmt(&self, f: &mut ::core::fmt::Formatter) -> ::core::fmt::Result {
        ::core::fmt::Formatter::debug_struct_field1_finish(
            f,
            "ModeEvent",
            "mode",
            &&self.mode,
        )
    }
}
/// Basic combined: input_device + processor
mod basic {
    use super::{ConfigEvent, SensorEvent, input_device, processor};
    #[::rmk::macros::runnable_generated]
    pub struct SensorController {
        pub threshold: u16,
    }
    impl ::rmk::processor::Processor for SensorController {
        type Event = ConfigEvent;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <ConfigEvent as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            self.on_config_event(event).await
        }
    }
    impl ::rmk::input_device::InputDevice for SensorController {
        type Event = SensorEvent;
        async fn read_event(&mut self) -> Self::Event {
            self.read_sensor_event().await
        }
    }
    impl ::rmk::core_traits::Runnable for SensorController {
        async fn run(&mut self) -> ! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            use ::rmk::event::SubscribableEvent;
            use ::rmk::processor::Processor;
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            enum __RmkSelectEventSensorController {
                Input(SensorEvent),
                Processor(ConfigEvent),
            }
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
            loop {
                let select_result = {
                    {
                        use ::futures_util::__private as __futures_crate;
                        {
                            enum __PrivResult<_0, _1> {
                                _0(_0),
                                _1(_1),
                            }
                            let __select_result = {
                                let mut _0 = self.read_event().fuse();
                                let mut _1 = proc_sub.next_event().fuse();
                                let mut __poll_fn = |
                                    __cx: &mut __futures_crate::task::Context<'_>|
                                {
                                    let mut __any_polled = false;
                                    let mut _0 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _1 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    __RmkSelectEventSensorController::Input(event)
                                }
                                __PrivResult::_1(proc_event) => {
                                    __RmkSelectEventSensorController::Processor(proc_event)
                                }
                            }
                        }
                    }
                };
                match select_result {
                    __RmkSelectEventSensorController::Input(event) => {
                        publish_event_async(event).await;
                    }
                    __RmkSelectEventSensorController::Processor(event) => {
                        <Self as ::rmk::processor::Processor>::process(self, event)
                            .await;
                    }
                }
            }
        }
    }
}
/// Reversed order: processor + input_device
mod reversed {
    use super::{ConfigEvent, SensorEvent, input_device, processor};
    #[::rmk::macros::runnable_generated]
    pub struct ReversedSensorController {
        pub threshold: u16,
    }
    impl ::rmk::input_device::InputDevice for ReversedSensorController {
        type Event = SensorEvent;
        async fn read_event(&mut self) -> Self::Event {
            self.read_sensor_event().await
        }
    }
    impl ::rmk::processor::Processor for ReversedSensorController {
        type Event = ConfigEvent;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <ConfigEvent as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            self.on_config_event(event).await
        }
    }
    impl ::rmk::core_traits::Runnable for ReversedSensorController {
        async fn run(&mut self) -> ! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            use ::rmk::event::SubscribableEvent;
            use ::rmk::processor::Processor;
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            enum __RmkSelectEventReversedSensorController {
                Input(SensorEvent),
                Processor(ConfigEvent),
            }
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
            loop {
                let select_result = {
                    {
                        use ::futures_util::__private as __futures_crate;
                        {
                            enum __PrivResult<_0, _1> {
                                _0(_0),
                                _1(_1),
                            }
                            let __select_result = {
                                let mut _0 = self.read_event().fuse();
                                let mut _1 = proc_sub.next_event().fuse();
                                let mut __poll_fn = |
                                    __cx: &mut __futures_crate::task::Context<'_>|
                                {
                                    let mut __any_polled = false;
                                    let mut _0 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _1 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    __RmkSelectEventReversedSensorController::Input(event)
                                }
                                __PrivResult::_1(proc_event) => {
                                    __RmkSelectEventReversedSensorController::Processor(
                                        proc_event,
                                    )
                                }
                            }
                        }
                    }
                };
                match select_result {
                    __RmkSelectEventReversedSensorController::Input(event) => {
                        publish_event_async(event).await;
                    }
                    __RmkSelectEventReversedSensorController::Processor(event) => {
                        <Self as ::rmk::processor::Processor>::process(self, event)
                            .await;
                    }
                }
            }
        }
    }
}
/// Polling combined: input_device + polling processor
mod polling {
    use super::{ConfigEvent, SensorEvent, input_device, processor};
    #[::rmk::macros::runnable_generated]
    pub struct PollingSensorController {
        pub counter: u32,
    }
    impl ::rmk::processor::Processor for PollingSensorController {
        type Event = ConfigEvent;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <ConfigEvent as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            self.on_config_event(event).await
        }
    }
    impl ::rmk::processor::PollingProcessor for PollingSensorController {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(50u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::input_device::InputDevice for PollingSensorController {
        type Event = SensorEvent;
        async fn read_event(&mut self) -> Self::Event {
            self.read_sensor_event().await
        }
    }
    impl ::rmk::core_traits::Runnable for PollingSensorController {
        async fn run(&mut self) -> ! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            use ::rmk::event::SubscribableEvent;
            use ::rmk::processor::Processor;
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            use ::rmk::processor::PollingProcessor;
            enum __RmkSelectEventPollingSensorController {
                Input(SensorEvent),
                Processor(ConfigEvent),
                Timer,
            }
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
            let mut ticker = ::embassy_time::Ticker::every(
                ::embassy_time::Duration::from_millis(50u64),
            );
            loop {
                let select_result = {
                    {
                        use ::futures_util::__private as __futures_crate;
                        {
                            enum __PrivResult<_0, _1, _2> {
                                _0(_0),
                                _1(_1),
                                _2(_2),
                            }
                            let __select_result = {
                                let mut _0 = ticker.next().fuse();
                                let mut _1 = self.read_event().fuse();
                                let mut _2 = proc_sub.next_event().fuse();
                                let mut __poll_fn = |
                                    __cx: &mut __futures_crate::task::Context<'_>|
                                {
                                    let mut __any_polled = false;
                                    let mut _0 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _1 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _2 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                __PrivResult::_0(_) => {
                                    __RmkSelectEventPollingSensorController::Timer
                                }
                                __PrivResult::_1(event) => {
                                    __RmkSelectEventPollingSensorController::Input(event)
                                }
                                __PrivResult::_2(proc_event) => {
                                    __RmkSelectEventPollingSensorController::Processor(
                                        proc_event,
                                    )
                                }
                            }
                        }
                    }
                };
                match select_result {
                    __RmkSelectEventPollingSensorController::Input(event) => {
                        publish_event_async(event).await;
                    }
                    __RmkSelectEventPollingSensorController::Processor(event) => {
                        <Self as ::rmk::processor::Processor>::process(self, event)
                            .await;
                    }
                    __RmkSelectEventPollingSensorController::Timer => {
                        <Self as ::rmk::processor::PollingProcessor>::update(self).await;
                    }
                }
            }
        }
    }
}
/// Multi-event combined: input_device + processor with multiple events
mod multi_event {
    use super::{ConfigEvent, ModeEvent, SensorEvent, input_device, processor};
    #[::rmk::macros::runnable_generated]
    pub struct MultiEventSensorController {
        pub threshold: u16,
        pub mode: u8,
    }
    pub enum MultiEventSensorControllerProcessorEventEnum {
        Config(ConfigEvent),
        Mode(ModeEvent),
    }
    #[automatically_derived]
    impl ::core::clone::Clone for MultiEventSensorControllerProcessorEventEnum {
        #[inline]
        fn clone(&self) -> MultiEventSensorControllerProcessorEventEnum {
            match self {
                MultiEventSensorControllerProcessorEventEnum::Config(__self_0) => {
                    MultiEventSensorControllerProcessorEventEnum::Config(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
                MultiEventSensorControllerProcessorEventEnum::Mode(__self_0) => {
                    MultiEventSensorControllerProcessorEventEnum::Mode(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
            }
        }
    }
    /// Event subscriber for aggregated events
    pub struct MultiEventSensorControllerProcessorEventSubscriber {
        sub0: <ConfigEvent as ::rmk::event::SubscribableEvent>::Subscriber,
        sub1: <ModeEvent as ::rmk::event::SubscribableEvent>::Subscriber,
    }
    impl MultiEventSensorControllerProcessorEventSubscriber {
        /// Create a new event subscriber
        pub fn new() -> Self {
            Self {
                sub0: <ConfigEvent as ::rmk::event::SubscribableEvent>::subscriber(),
                sub1: <ModeEvent as ::rmk::event::SubscribableEvent>::subscriber(),
            }
        }
    }
    impl ::rmk::event::EventSubscriber
    for MultiEventSensorControllerProcessorEventSubscriber {
        type Event = MultiEventSensorControllerProcessorEventEnum;
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
                            MultiEventSensorControllerProcessorEventEnum::Config(event)
                        }
                        __PrivResult::_1(event) => {
                            MultiEventSensorControllerProcessorEventEnum::Mode(event)
                        }
                    }
                }
            }
        }
    }
    impl ::rmk::event::SubscribableEvent
    for MultiEventSensorControllerProcessorEventEnum {
        type Subscriber = MultiEventSensorControllerProcessorEventSubscriber;
        fn subscriber() -> Self::Subscriber {
            MultiEventSensorControllerProcessorEventSubscriber::new()
        }
    }
    impl ::rmk::processor::Processor for MultiEventSensorController {
        type Event = MultiEventSensorControllerProcessorEventEnum;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <MultiEventSensorControllerProcessorEventEnum as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {
                MultiEventSensorControllerProcessorEventEnum::Config(event) => {
                    self.on_config_event(event).await
                }
                MultiEventSensorControllerProcessorEventEnum::Mode(event) => {
                    self.on_mode_event(event).await
                }
            }
        }
    }
    impl ::rmk::input_device::InputDevice for MultiEventSensorController {
        type Event = SensorEvent;
        async fn read_event(&mut self) -> Self::Event {
            self.read_sensor_event().await
        }
    }
    impl ::rmk::core_traits::Runnable for MultiEventSensorController {
        async fn run(&mut self) -> ! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            use ::rmk::event::SubscribableEvent;
            use ::rmk::processor::Processor;
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            enum __RmkSelectEventMultiEventSensorController {
                Input(SensorEvent),
                Processor(MultiEventSensorControllerProcessorEventEnum),
            }
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
            loop {
                let select_result = {
                    {
                        use ::futures_util::__private as __futures_crate;
                        {
                            enum __PrivResult<_0, _1> {
                                _0(_0),
                                _1(_1),
                            }
                            let __select_result = {
                                let mut _0 = self.read_event().fuse();
                                let mut _1 = proc_sub.next_event().fuse();
                                let mut __poll_fn = |
                                    __cx: &mut __futures_crate::task::Context<'_>|
                                {
                                    let mut __any_polled = false;
                                    let mut _0 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _1 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    __RmkSelectEventMultiEventSensorController::Input(event)
                                }
                                __PrivResult::_1(proc_event) => {
                                    __RmkSelectEventMultiEventSensorController::Processor(
                                        proc_event,
                                    )
                                }
                            }
                        }
                    }
                };
                match select_result {
                    __RmkSelectEventMultiEventSensorController::Input(event) => {
                        publish_event_async(event).await;
                    }
                    __RmkSelectEventMultiEventSensorController::Processor(event) => {
                        <Self as ::rmk::processor::Processor>::process(self, event)
                            .await;
                    }
                }
            }
        }
    }
}
/// Multi-event polling combined
mod multi_event_polling {
    use super::{ConfigEvent, ModeEvent, SensorEvent, input_device, processor};
    #[::rmk::macros::runnable_generated]
    pub struct MultiEventPollingSensorController {
        pub threshold: u16,
        pub mode: u8,
        pub counter: u32,
    }
    pub enum MultiEventPollingSensorControllerProcessorEventEnum {
        Config(ConfigEvent),
        Mode(ModeEvent),
    }
    #[automatically_derived]
    impl ::core::clone::Clone for MultiEventPollingSensorControllerProcessorEventEnum {
        #[inline]
        fn clone(&self) -> MultiEventPollingSensorControllerProcessorEventEnum {
            match self {
                MultiEventPollingSensorControllerProcessorEventEnum::Config(__self_0) => {
                    MultiEventPollingSensorControllerProcessorEventEnum::Config(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
                MultiEventPollingSensorControllerProcessorEventEnum::Mode(__self_0) => {
                    MultiEventPollingSensorControllerProcessorEventEnum::Mode(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
            }
        }
    }
    /// Event subscriber for aggregated events
    pub struct MultiEventPollingSensorControllerProcessorEventSubscriber {
        sub0: <ConfigEvent as ::rmk::event::SubscribableEvent>::Subscriber,
        sub1: <ModeEvent as ::rmk::event::SubscribableEvent>::Subscriber,
    }
    impl MultiEventPollingSensorControllerProcessorEventSubscriber {
        /// Create a new event subscriber
        pub fn new() -> Self {
            Self {
                sub0: <ConfigEvent as ::rmk::event::SubscribableEvent>::subscriber(),
                sub1: <ModeEvent as ::rmk::event::SubscribableEvent>::subscriber(),
            }
        }
    }
    impl ::rmk::event::EventSubscriber
    for MultiEventPollingSensorControllerProcessorEventSubscriber {
        type Event = MultiEventPollingSensorControllerProcessorEventEnum;
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
                            MultiEventPollingSensorControllerProcessorEventEnum::Config(
                                event,
                            )
                        }
                        __PrivResult::_1(event) => {
                            MultiEventPollingSensorControllerProcessorEventEnum::Mode(
                                event,
                            )
                        }
                    }
                }
            }
        }
    }
    impl ::rmk::event::SubscribableEvent
    for MultiEventPollingSensorControllerProcessorEventEnum {
        type Subscriber = MultiEventPollingSensorControllerProcessorEventSubscriber;
        fn subscriber() -> Self::Subscriber {
            MultiEventPollingSensorControllerProcessorEventSubscriber::new()
        }
    }
    impl ::rmk::processor::Processor for MultiEventPollingSensorController {
        type Event = MultiEventPollingSensorControllerProcessorEventEnum;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <MultiEventPollingSensorControllerProcessorEventEnum as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {
                MultiEventPollingSensorControllerProcessorEventEnum::Config(event) => {
                    self.on_config_event(event).await
                }
                MultiEventPollingSensorControllerProcessorEventEnum::Mode(event) => {
                    self.on_mode_event(event).await
                }
            }
        }
    }
    impl ::rmk::processor::PollingProcessor for MultiEventPollingSensorController {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(100u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::input_device::InputDevice for MultiEventPollingSensorController {
        type Event = SensorEvent;
        async fn read_event(&mut self) -> Self::Event {
            self.read_sensor_event().await
        }
    }
    impl ::rmk::core_traits::Runnable for MultiEventPollingSensorController {
        async fn run(&mut self) -> ! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            use ::rmk::event::SubscribableEvent;
            use ::rmk::processor::Processor;
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            use ::rmk::processor::PollingProcessor;
            enum __RmkSelectEventMultiEventPollingSensorController {
                Input(SensorEvent),
                Processor(MultiEventPollingSensorControllerProcessorEventEnum),
                Timer,
            }
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
            let mut ticker = ::embassy_time::Ticker::every(
                ::embassy_time::Duration::from_millis(100u64),
            );
            loop {
                let select_result = {
                    {
                        use ::futures_util::__private as __futures_crate;
                        {
                            enum __PrivResult<_0, _1, _2> {
                                _0(_0),
                                _1(_1),
                                _2(_2),
                            }
                            let __select_result = {
                                let mut _0 = ticker.next().fuse();
                                let mut _1 = self.read_event().fuse();
                                let mut _2 = proc_sub.next_event().fuse();
                                let mut __poll_fn = |
                                    __cx: &mut __futures_crate::task::Context<'_>|
                                {
                                    let mut __any_polled = false;
                                    let mut _0 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _1 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _2 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                __PrivResult::_0(_) => {
                                    __RmkSelectEventMultiEventPollingSensorController::Timer
                                }
                                __PrivResult::_1(event) => {
                                    __RmkSelectEventMultiEventPollingSensorController::Input(
                                        event,
                                    )
                                }
                                __PrivResult::_2(proc_event) => {
                                    __RmkSelectEventMultiEventPollingSensorController::Processor(
                                        proc_event,
                                    )
                                }
                            }
                        }
                    }
                };
                match select_result {
                    __RmkSelectEventMultiEventPollingSensorController::Input(event) => {
                        publish_event_async(event).await;
                    }
                    __RmkSelectEventMultiEventPollingSensorController::Processor(
                        event,
                    ) => {
                        <Self as ::rmk::processor::Processor>::process(self, event)
                            .await;
                    }
                    __RmkSelectEventMultiEventPollingSensorController::Timer => {
                        <Self as ::rmk::processor::PollingProcessor>::update(self).await;
                    }
                }
            }
        }
    }
}
/// The input-device macro sees the same merged processor configuration.
mod sibling_polling {
    use super::{ConfigEvent, ModeEvent, SensorEvent, input_device, processor};
    #[::rmk::macros::runnable_generated]
    pub struct MergedSensorController;
    pub enum MergedSensorControllerProcessorEventEnum {
        Config(ConfigEvent),
        Mode(ModeEvent),
    }
    #[automatically_derived]
    impl ::core::clone::Clone for MergedSensorControllerProcessorEventEnum {
        #[inline]
        fn clone(&self) -> MergedSensorControllerProcessorEventEnum {
            match self {
                MergedSensorControllerProcessorEventEnum::Config(__self_0) => {
                    MergedSensorControllerProcessorEventEnum::Config(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
                MergedSensorControllerProcessorEventEnum::Mode(__self_0) => {
                    MergedSensorControllerProcessorEventEnum::Mode(
                        ::core::clone::Clone::clone(__self_0),
                    )
                }
            }
        }
    }
    /// Event subscriber for aggregated events
    pub struct MergedSensorControllerProcessorEventSubscriber {
        sub0: <ConfigEvent as ::rmk::event::SubscribableEvent>::Subscriber,
        sub1: <ModeEvent as ::rmk::event::SubscribableEvent>::Subscriber,
    }
    impl MergedSensorControllerProcessorEventSubscriber {
        /// Create a new event subscriber
        pub fn new() -> Self {
            Self {
                sub0: <ConfigEvent as ::rmk::event::SubscribableEvent>::subscriber(),
                sub1: <ModeEvent as ::rmk::event::SubscribableEvent>::subscriber(),
            }
        }
    }
    impl ::rmk::event::EventSubscriber
    for MergedSensorControllerProcessorEventSubscriber {
        type Event = MergedSensorControllerProcessorEventEnum;
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
                            MergedSensorControllerProcessorEventEnum::Config(event)
                        }
                        __PrivResult::_1(event) => {
                            MergedSensorControllerProcessorEventEnum::Mode(event)
                        }
                    }
                }
            }
        }
    }
    impl ::rmk::event::SubscribableEvent for MergedSensorControllerProcessorEventEnum {
        type Subscriber = MergedSensorControllerProcessorEventSubscriber;
        fn subscriber() -> Self::Subscriber {
            MergedSensorControllerProcessorEventSubscriber::new()
        }
    }
    impl ::rmk::processor::Processor for MergedSensorController {
        type Event = MergedSensorControllerProcessorEventEnum;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            <MergedSensorControllerProcessorEventEnum as ::rmk::event::SubscribableEvent>::subscriber()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {
                MergedSensorControllerProcessorEventEnum::Config(event) => {
                    self.on_config_event(event).await
                }
                MergedSensorControllerProcessorEventEnum::Mode(event) => {
                    self.on_mode_event(event).await
                }
            }
        }
    }
    impl ::rmk::processor::PollingProcessor for MergedSensorController {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(100u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::input_device::InputDevice for MergedSensorController {
        type Event = SensorEvent;
        async fn read_event(&mut self) -> Self::Event {
            self.read_sensor_event().await
        }
    }
    impl ::rmk::core_traits::Runnable for MergedSensorController {
        async fn run(&mut self) -> ! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            use ::rmk::event::SubscribableEvent;
            use ::rmk::processor::Processor;
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            use ::rmk::processor::PollingProcessor;
            enum __RmkSelectEventMergedSensorController {
                Input(SensorEvent),
                Processor(MergedSensorControllerProcessorEventEnum),
                Timer,
            }
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
            let mut ticker = ::embassy_time::Ticker::every(
                ::embassy_time::Duration::from_millis(100u64),
            );
            loop {
                let select_result = {
                    {
                        use ::futures_util::__private as __futures_crate;
                        {
                            enum __PrivResult<_0, _1, _2> {
                                _0(_0),
                                _1(_1),
                                _2(_2),
                            }
                            let __select_result = {
                                let mut _0 = ticker.next().fuse();
                                let mut _1 = self.read_event().fuse();
                                let mut _2 = proc_sub.next_event().fuse();
                                let mut __poll_fn = |
                                    __cx: &mut __futures_crate::task::Context<'_>|
                                {
                                    let mut __any_polled = false;
                                    let mut _0 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _1 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _2 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                __PrivResult::_0(_) => {
                                    __RmkSelectEventMergedSensorController::Timer
                                }
                                __PrivResult::_1(event) => {
                                    __RmkSelectEventMergedSensorController::Input(event)
                                }
                                __PrivResult::_2(proc_event) => {
                                    __RmkSelectEventMergedSensorController::Processor(
                                        proc_event,
                                    )
                                }
                            }
                        }
                    }
                };
                match select_result {
                    __RmkSelectEventMergedSensorController::Input(event) => {
                        publish_event_async(event).await;
                    }
                    __RmkSelectEventMergedSensorController::Processor(event) => {
                        <Self as ::rmk::processor::Processor>::process(self, event)
                            .await;
                    }
                    __RmkSelectEventMergedSensorController::Timer => {
                        <Self as ::rmk::processor::PollingProcessor>::update(self).await;
                    }
                }
            }
        }
    }
}
mod polling_without_subscription {
    use super::{SensorEvent, input_device, processor};
    #[::rmk::macros::runnable_generated]
    pub struct PollingSensor;
    impl ::rmk::processor::Processor for PollingSensor {
        type Event = ::core::convert::Infallible;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            ::core::future::pending::<Self::Event>()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {}
        }
    }
    impl ::rmk::processor::PollingProcessor for PollingSensor {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(100u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::input_device::InputDevice for PollingSensor {
        type Event = SensorEvent;
        async fn read_event(&mut self) -> Self::Event {
            self.read_sensor_event().await
        }
    }
    impl ::rmk::core_traits::Runnable for PollingSensor {
        async fn run(&mut self) -> ! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            use ::rmk::event::SubscribableEvent;
            use ::rmk::processor::Processor;
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            use ::rmk::processor::PollingProcessor;
            enum __RmkSelectEventPollingSensor {
                Input(SensorEvent),
                Processor(::core::convert::Infallible),
                Timer,
            }
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
            let mut ticker = ::embassy_time::Ticker::every(
                ::embassy_time::Duration::from_millis(100u64),
            );
            loop {
                let select_result = {
                    {
                        use ::futures_util::__private as __futures_crate;
                        {
                            enum __PrivResult<_0, _1, _2> {
                                _0(_0),
                                _1(_1),
                                _2(_2),
                            }
                            let __select_result = {
                                let mut _0 = ticker.next().fuse();
                                let mut _1 = self.read_event().fuse();
                                let mut _2 = proc_sub.next_event().fuse();
                                let mut __poll_fn = |
                                    __cx: &mut __futures_crate::task::Context<'_>|
                                {
                                    let mut __any_polled = false;
                                    let mut _0 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _1 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _2 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                __PrivResult::_0(_) => __RmkSelectEventPollingSensor::Timer,
                                __PrivResult::_1(event) => {
                                    __RmkSelectEventPollingSensor::Input(event)
                                }
                                __PrivResult::_2(proc_event) => {
                                    __RmkSelectEventPollingSensor::Processor(proc_event)
                                }
                            }
                        }
                    }
                };
                match select_result {
                    __RmkSelectEventPollingSensor::Input(event) => {
                        publish_event_async(event).await;
                    }
                    __RmkSelectEventPollingSensor::Processor(event) => {
                        <Self as ::rmk::processor::Processor>::process(self, event)
                            .await;
                    }
                    __RmkSelectEventPollingSensor::Timer => {
                        <Self as ::rmk::processor::PollingProcessor>::update(self).await;
                    }
                }
            }
        }
    }
    #[::rmk::macros::runnable_generated]
    pub struct ReversedPollingSensor;
    impl ::rmk::input_device::InputDevice for ReversedPollingSensor {
        type Event = SensorEvent;
        async fn read_event(&mut self) -> Self::Event {
            self.read_sensor_event().await
        }
    }
    impl ::rmk::processor::Processor for ReversedPollingSensor {
        type Event = ::core::convert::Infallible;
        fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
            ::core::future::pending::<Self::Event>()
        }
        async fn process(&mut self, event: Self::Event) {
            match event {}
        }
    }
    impl ::rmk::processor::PollingProcessor for ReversedPollingSensor {
        fn interval(&self) -> ::embassy_time::Duration {
            ::embassy_time::Duration::from_millis(100u64)
        }
        async fn update(&mut self) {
            self.poll().await;
        }
    }
    impl ::rmk::core_traits::Runnable for ReversedPollingSensor {
        async fn run(&mut self) -> ! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            use ::rmk::event::SubscribableEvent;
            use ::rmk::processor::Processor;
            use ::rmk::event::EventSubscriber;
            use ::rmk::futures::FutureExt;
            use ::rmk::processor::PollingProcessor;
            enum __RmkSelectEventReversedPollingSensor {
                Input(SensorEvent),
                Processor(::core::convert::Infallible),
                Timer,
            }
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
            let mut ticker = ::embassy_time::Ticker::every(
                ::embassy_time::Duration::from_millis(100u64),
            );
            loop {
                let select_result = {
                    {
                        use ::futures_util::__private as __futures_crate;
                        {
                            enum __PrivResult<_0, _1, _2> {
                                _0(_0),
                                _1(_1),
                                _2(_2),
                            }
                            let __select_result = {
                                let mut _0 = ticker.next().fuse();
                                let mut _1 = self.read_event().fuse();
                                let mut _2 = proc_sub.next_event().fuse();
                                let mut __poll_fn = |
                                    __cx: &mut __futures_crate::task::Context<'_>|
                                {
                                    let mut __any_polled = false;
                                    let mut _0 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _1 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                    let mut _2 = |
                                        __cx: &mut __futures_crate::task::Context<'_>|
                                    {
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
                                __PrivResult::_0(_) => {
                                    __RmkSelectEventReversedPollingSensor::Timer
                                }
                                __PrivResult::_1(event) => {
                                    __RmkSelectEventReversedPollingSensor::Input(event)
                                }
                                __PrivResult::_2(proc_event) => {
                                    __RmkSelectEventReversedPollingSensor::Processor(proc_event)
                                }
                            }
                        }
                    }
                };
                match select_result {
                    __RmkSelectEventReversedPollingSensor::Input(event) => {
                        publish_event_async(event).await;
                    }
                    __RmkSelectEventReversedPollingSensor::Processor(event) => {
                        <Self as ::rmk::processor::Processor>::process(self, event)
                            .await;
                    }
                    __RmkSelectEventReversedPollingSensor::Timer => {
                        <Self as ::rmk::processor::PollingProcessor>::update(self).await;
                    }
                }
            }
        }
    }
}
