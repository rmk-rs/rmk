# Processor

A processor reacts to keyboard events or timers. Use one to update an LED, refresh a display, or handle an inactivity timeout.

## Create an event processor

Add `#[processor]` to a struct and list the events it should receive. For each event, provide an async handler named `on_<event_name>_event`.

This processor keeps track of the active layer:

```rust title="src/layer_tracker.rs"
use rmk::event::LayerChangeEvent;
use rmk::macros::processor;

#[processor(subscribe = [LayerChangeEvent])]
#[derive(Default)]
pub struct LayerTracker {
    layer: u8,
}

impl LayerTracker {
    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        self.layer = event.0;
    }
}
```

The macro implements the processor traits and `Runnable`. Defining the type does not start it; add an instance to your keyboard's tasks as described below.

## Run the processor

### With keyboard.toml

Declare the module in `src/main.rs`, then add a constructor marked with `#[register_processor]` inside your keyboard module:

```rust title="src/main.rs"
#![no_main]
#![no_std]

mod layer_tracker;

use rmk::macros::rmk_keyboard;

#[rmk_keyboard]
mod keyboard {
    #[register_processor]
    fn layer_tracker() -> crate::layer_tracker::LayerTracker {
        crate::layer_tracker::LayerTracker::default()
    }
}
```

For a split keyboard, register it in the central's `#[rmk_central]` module instead.

The constructor runs after chip initialization and can use the peripherals in `p`. Choose pins that `keyboard.toml` does not already use. To enable a processor conditionally, add `#[cfg(...)]` to its registration function; the condition applies to both construction and execution.

### With the Rust API

After initializing your keyboard and transport, add the processor to the existing `run_all!` call:

```rust
use rmk::run_all;

let mut layer_tracker = LayerTracker::default();
run_all!(layer_tracker, keyboard, transport).await;
```

Both approaches run the type's `Runnable` implementation. Select event and timer behavior on the type's `#[processor]` attribute.

## Polling processor

Use `poll_interval` for work that repeats at a fixed interval. The value is in milliseconds and must be greater than zero. Provide an async `poll()` method.

A processor that only polls does not need a `subscribe` list. This example toggles a GPIO every 500 ms:

```rust title="src/blinker.rs"
use embedded_hal::digital::{OutputPin, PinState};
use rmk::macros::processor;

#[processor(poll_interval = 500)]
pub struct Blinker<P: OutputPin> {
    pin: P,
    on: bool,
}

impl<P: OutputPin> Blinker<P> {
    pub fn new(mut pin: P) -> Self {
        pin.set_low().ok();
        Self { pin, on: false }
    }

    async fn poll(&mut self) {
        self.on = !self.on;
        self.pin.set_state(PinState::from(self.on)).ok();
    }
}
```

To also react to events, add `subscribe = [...]` and the corresponding event handlers. Events do not restart the polling interval.

## Deadline processor

Use `deadline` for a timeout that can be reset or cancelled. Provide these methods:

- `fn deadline(&self) -> Option<Instant>` returns the next timeout, or `None` when no timeout is armed.
- `async fn on_deadline(&mut self)` handles the timeout and clears or advances it.

This example marks pointing activity as inactive 500 ms after the last `PointingEvent`:

```rust title="src/motion_activity.rs"
use embassy_time::{Duration, Instant};
use rmk::event::PointingEvent;
use rmk::macros::processor;

#[processor(subscribe = [PointingEvent], deadline)]
#[derive(Default)]
pub struct MotionActivity {
    active: bool,
    armed_until: Option<Instant>,
}

impl MotionActivity {
    async fn on_pointing_event(&mut self, _event: PointingEvent) {
        self.active = true;
        self.armed_until = Some(Instant::now() + Duration::from_millis(500));
    }

    fn deadline(&self) -> Option<Instant> {
        self.armed_until
    }

    async fn on_deadline(&mut self) {
        self.active = false;
        self.armed_until = None;
    }
}
```

An event handler can reset the timeout by storing a later deadline, or cancel it by setting the deadline to `None`. The task checks the deadline again after each callback. A deadline-only processor can omit `subscribe`; its initial state must arm the first timeout.

### Combine polling and deadlines

Set both `poll_interval` and `deadline` on the same `#[processor]` attribute. Provide `poll()`, `deadline()`, and `on_deadline()`; either polling or an event handler can arm a timeout.

Callbacks run one at a time. When several sources are ready, RMK handles deadlines first, polling ticks second, and events third. Clear or advance a deadline in `on_deadline()` to avoid calling it repeatedly for the same expired timeout.

## Handle multiple event types

List each event in `subscribe` and provide a handler for each one:

```rust title="src/status.rs"
use rmk::event::{LayerChangeEvent, LedIndicatorEvent};
use rmk::macros::processor;

#[processor(subscribe = [LayerChangeEvent, LedIndicatorEvent])]
#[derive(Default)]
pub struct Status {
    layer: u8,
    caps_lock: bool,
}

impl Status {
    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        self.layer = event.0;
    }

    async fn on_led_indicator_event(&mut self, event: LedIndicatorEvent) {
        self.caps_lock = event.caps_lock();
    }
}
```

Event names use snake case: `LayerChangeEvent` maps to `on_layer_change_event`, and `LedIndicatorEvent` maps to `on_led_indicator_event`. Ensure each event has enough subscriber slots; see [event configuration](../configuration/event).

## Attribute reference

| Option                         | Required methods                                                              | Behavior                                                  |
| ------------------------------ | ----------------------------------------------------------------------------- | --------------------------------------------------------- |
| `subscribe = [EventType, ...]` | `async fn on_<event_name>_event(&mut self, event: EventType)` for each event  | Handle events as they arrive.                             |
| `poll_interval = <ms>`         | `async fn poll(&mut self)`                                                    | Run periodic work at a positive interval in milliseconds. |
| `deadline`                     | `fn deadline(&self) -> Option<Instant>` and `async fn on_deadline(&mut self)` | Handle a timeout determined by the processor's state.     |

You can combine these options. Omit `subscribe` or set it to `[]` when no events are needed. At least one event or timer source is required unless you provide a custom `Runnable`.

## Custom Runnable

To provide your own run loop, implement `rmk::core_traits::Runnable` and place `#[rmk::macros::runnable_generated]` below `#[processor]`. The marker suppresses the generated run loop; the macro still implements the requested processor traits. A bare `#[processor]` is valid with this marker.

Combining `deadline` with `#[input_device]` requires a custom run loop. Choose how that loop schedules input reads, event handling, and timeouts.

## Related documentation

- [Events](./event): built-in events and custom event types.
- [Input devices](./input_device): publish events from hardware.
- [Event configuration](../configuration/event): reserve channel capacity and subscriber slots.
- [Upgrade from v0.9 to v0.10](../migration/v09_v10#processors): migrate existing processor registrations and trait implementations.
