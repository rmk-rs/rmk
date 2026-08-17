# Processor

RMK's processor system provides a unified interface for components that consume events and react to them, such as displays, LEDs, and other output peripherals.

## Overview

Processors subscribe to events and react accordingly. Events are published by [Input Devices](./input_device) or other processors. For details about events, see the [Event](./event) documentation.

Processors can operate in three modes:

- **Event-driven** - React to events as they arrive
- **Polling** - Perform periodic updates at specified intervals (in addition to handling events)
- **Deadline** - Fire a callback at a deadline the processor computes from its own state (in addition to handling events)

## Defining Processors

Use the `#[processor]` macro to define custom processors:

```rust
use rmk::event::LayerChangeEvent;
use rmk::macros::processor;

#[processor(subscribe = [LayerChangeEvent])]
pub struct MyProcessor {
    // Your processor fields
}

impl MyProcessor {
    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        // Handle layer changes
    }
}
```

**Parameters:**

- `subscribe = [Event1, Event2, ...]` (required): Event types to subscribe to (see [Built-in Events](./event#built-in-events))
- `poll_interval = <ms>` (optional): Enable polling with fixed interval, requires `poll()` method

**How it works:**

- `#[processor]` implements `Processor` and `Runnable` traits automatically
- Event handlers are automatically routed based on method naming: `on_<event_name>_event()`
- Method names follow snake_case conversion of event type names

## Registering Processors

If you use the Rust API directly, no registration is needed — every processor implements `Runnable`, so just pass it to `run_all!` alongside your other tasks (see [Input Device](./input_device#running-input-devices)).

For `keyboard.toml` users, processors are registered in the `#[rmk_keyboard]` module using the `#[register_processor]` attribute:

```rust
#[rmk_keyboard]
mod my_keyboard {
    use super::*;

    #[register_processor(event)]  // Event-driven mode
    fn my_processor() -> MyProcessor {
        MyProcessor::new()
    }
}
```

Available registration modes:

- `#[register_processor(event)]`: Event-driven mode, reacts to subscribed events
- `#[register_processor(poll)]`: Polling mode, requires `poll_interval` parameter in `#[processor]` macro

## Multi-event Subscription

Processors can subscribe to multiple event types and handle them with separate methods:

```rust
use rmk::event::{BatteryStatusEvent, LayerChangeEvent};
use rmk::macros::processor;

#[processor(subscribe = [LayerChangeEvent, BatteryStatusEvent])]
pub struct MultiEventProcessor {
    layer: u8,
}

impl MultiEventProcessor {
    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        self.layer = event.0;
        // Update display with new layer
    }

    async fn on_battery_status_event(&mut self, event: BatteryStatusEvent) {
        // Update battery indicator
    }
}
```

## Polling Processor

For processors that need periodic updates (e.g., display refresh, LED animations), use the `poll_interval` parameter:

```rust
use rmk::event::LayerChangeEvent;
use rmk::macros::processor;

#[processor(subscribe = [LayerChangeEvent], poll_interval = 500)]
pub struct StatusScreen<D: DrawTarget> {
    display: D,
    layer: u8,
    needs_refresh: bool,
}

impl<D: DrawTarget> StatusScreen<D> {
    pub fn new(display: D) -> Self {
        Self {
            display,
            layer: 0,
            needs_refresh: true,
        }
    }

    // Event handler - triggered when layer changes
    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        self.layer = event.0;
        self.needs_refresh = true;
    }

    // Called every 500ms
    async fn poll(&mut self) {
        if self.needs_refresh {
            self.render_layer();
            self.needs_refresh = false;
        }
    }

    fn render_layer(&mut self) {
        // Render current layer to display
    }
}
```

## Deadline Processor

For timeouts that move — a layer that deactivates some time after the last mouse motion, for example — a fixed `poll_interval` doesn't fit. Implement `DeadlineProcessor` instead: `deadline()` returns the next `Instant` to fire at, or `None` when nothing is armed, and `on_deadline()` runs when that instant passes without an event in between. `deadline_loop()` drives it, re-reading `deadline()` after every event.

The `Runnable` that `#[processor]` generates only runs the event loop, so mark the struct with `#[::rmk::macros::runnable_generated]` to suppress it and write `Runnable` yourself:

```rust
use embassy_time::{Duration, Instant};
use rmk::core_traits::Runnable;
use rmk::event::PointingEvent;
use rmk::macros::processor;
use rmk::processor::DeadlineProcessor;

#[processor(subscribe = [PointingEvent])]
#[::rmk::macros::runnable_generated]
pub struct MotionTimeout {
    armed_until: Option<Instant>,
}

impl MotionTimeout {
    async fn on_pointing_event(&mut self, _event: PointingEvent) {
        // Every motion pushes the deadline back
        self.armed_until = Some(Instant::now() + Duration::from_millis(500));
    }
}

impl DeadlineProcessor for MotionTimeout {
    fn deadline(&self) -> Option<Instant> {
        self.armed_until
    }

    async fn on_deadline(&mut self) {
        self.armed_until = None;
        // Motion stopped 500ms ago
    }
}

impl Runnable for MotionTimeout {
    async fn run(&mut self) -> ! {
        self.deadline_loop().await
    }
}
```

## Example: LED Indicator Processor

A complete example of a processor that controls an LED based on keyboard indicators:

```rust
use embedded_hal::digital::StatefulOutputPin;
use rmk::event::LedIndicatorEvent;
use rmk::macros::processor;

#[processor(subscribe = [LedIndicatorEvent])]
pub struct CapsLockLed<P: StatefulOutputPin> {
    led: P,
    low_active: bool,
}

impl<P: StatefulOutputPin> CapsLockLed<P> {
    pub fn new(pin: P, low_active: bool) -> Self {
        Self {
            led: pin,
            low_active,
        }
    }

    async fn on_led_indicator_event(&mut self, event: LedIndicatorEvent) {
        let should_light = event.caps_lock();
        if should_light != self.low_active {
            self.led.set_high().ok();
        } else {
            self.led.set_low().ok();
        }
    }
}
```

RMK ships this functionality as the built-in `rmk::processor::builtin::led_indicator::KeyboardIndicatorProcessor`, so you only need a custom processor like this for behavior the built-in doesn't cover.

## Related Documentation

- [Event](./event) - Event concepts, built-in events, and custom event definition
- [Input Device](./input_device) - How to create input devices that publish events
