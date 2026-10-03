//! Expand tests for #[processor] macro.
//!
//! Tests:
//! - Single event subscription
//! - Multiple event subscription
//! - Polling processor with poll_interval
//! - Multiple #[processor] attributes (merged subscriptions)
//! - Deadline processor (`deadline`)

use rmk_macro::processor;

#[derive(Clone, Copy, Debug)]
pub struct KeyEvent {
    pub row: u8,
    pub col: u8,
    pub pressed: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct EncoderEvent {
    pub index: u8,
    pub direction: i8,
}

#[derive(Clone, Copy, Debug)]
pub struct ConfigEvent {
    pub threshold: u16,
}

/// Single event subscription
mod basic {
    use super::{KeyEvent, processor};

    #[processor(subscribe = [KeyEvent])]
    pub struct SingleEventProcessor;
}

/// Multiple event subscription
mod multi_sub {
    use super::{EncoderEvent, KeyEvent, processor};

    #[processor(subscribe = [KeyEvent, EncoderEvent])]
    pub struct KeyProcessor;
}

/// Polling processor
mod polling {
    use super::{ConfigEvent, processor};

    #[processor(subscribe = [ConfigEvent], poll_interval = 100)]
    pub struct PollingProcessor {
        pub counter: u32,
    }
}

/// Multiple #[processor] attributes (simulates cfg_attr expansion)
mod multi_attr {
    use super::{EncoderEvent, KeyEvent, processor};

    #[processor(subscribe = [KeyEvent])]
    #[processor(subscribe = [EncoderEvent])]
    pub struct MultiAttrProcessor;
}

/// Polling processor with multiple events
mod polling_multi {
    use super::{ConfigEvent, EncoderEvent, KeyEvent, processor};

    #[processor(subscribe = [KeyEvent, EncoderEvent, ConfigEvent], poll_interval = 50)]
    pub struct MultiPollingProcessor {
        pub state: u8,
    }
}

/// Deadline processor: the generated `Runnable` drives `deadline_loop()`
mod deadline {
    use super::{KeyEvent, processor};

    #[processor(subscribe = [KeyEvent], deadline)]
    pub struct Blinker {
        pub armed: bool,
    }
}

/// Independent timing sources can be declared on sibling attributes.
mod polling_deadline {
    use super::{ConfigEvent, KeyEvent, processor};

    #[processor(subscribe = [KeyEvent], poll_interval = 100)]
    #[processor(subscribe = [ConfigEvent], deadline)]
    pub struct StatusDisplay;
}

mod timer_only {
    use super::processor;

    #[processor(poll_interval = 100)]
    pub struct Polling;

    #[processor(subscribe = [], deadline)]
    pub struct Deadline;

    #[processor]
    #[processor(poll_interval = 100, deadline)]
    pub struct Both;

    #[processor]
    #[::rmk::macros::runnable_generated]
    pub struct Custom;
}
