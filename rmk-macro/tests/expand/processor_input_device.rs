//! Expand tests for combined #[processor] + #[input_device] macros.
//!
//! Tests:
//! - Input device with processor subscription
//! - Polling processor with input device

use rmk_macro::{input_device, processor};

#[derive(Clone, Copy, Debug)]
pub struct SensorEvent {
    pub value: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct ConfigEvent {
    pub threshold: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct ModeEvent {
    pub mode: u8,
}

/// Basic combined: input_device + processor
mod basic {
    use super::{ConfigEvent, SensorEvent, input_device, processor};

    #[input_device(publish = SensorEvent)]
    #[processor(subscribe = [ConfigEvent])]
    pub struct SensorController {
        pub threshold: u16,
    }
}

/// Reversed order: processor + input_device
mod reversed {
    use super::{ConfigEvent, SensorEvent, input_device, processor};

    #[processor(subscribe = [ConfigEvent])]
    #[input_device(publish = SensorEvent)]
    pub struct ReversedSensorController {
        pub threshold: u16,
    }
}

/// Polling combined: input_device + polling processor
mod polling {
    use super::{ConfigEvent, SensorEvent, input_device, processor};

    #[input_device(publish = SensorEvent)]
    #[processor(subscribe = [ConfigEvent], poll_interval = 50)]
    pub struct PollingSensorController {
        pub counter: u32,
    }
}

/// Multi-event combined: input_device + processor with multiple events
mod multi_event {
    use super::{ConfigEvent, ModeEvent, SensorEvent, input_device, processor};

    #[input_device(publish = SensorEvent)]
    #[processor(subscribe = [ConfigEvent, ModeEvent])]
    pub struct MultiEventSensorController {
        pub threshold: u16,
        pub mode: u8,
    }
}

/// Multi-event polling combined
mod multi_event_polling {
    use super::{ConfigEvent, ModeEvent, SensorEvent, input_device, processor};

    #[input_device(publish = SensorEvent)]
    #[processor(subscribe = [ConfigEvent, ModeEvent], poll_interval = 100)]
    pub struct MultiEventPollingSensorController {
        pub threshold: u16,
        pub mode: u8,
        pub counter: u32,
    }
}

/// The input-device macro sees the same merged processor configuration.
mod sibling_polling {
    use super::{ConfigEvent, ModeEvent, SensorEvent, input_device, processor};

    #[input_device(publish = SensorEvent)]
    #[processor(subscribe = [ConfigEvent])]
    #[processor(subscribe = [ModeEvent], poll_interval = 100)]
    pub struct MergedSensorController;
}

mod polling_without_subscription {
    use super::{SensorEvent, input_device, processor};

    #[input_device(publish = SensorEvent)]
    #[processor(poll_interval = 100)]
    pub struct PollingSensor;

    #[processor(subscribe = [], poll_interval = 100)]
    #[input_device(publish = SensorEvent)]
    pub struct ReversedPollingSensor;
}
