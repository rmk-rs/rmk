# Processor

RMK's processor system provides a unified interface for components that consume events and react to them, such as displays, LEDs, and other output peripherals.

## Overview

Processors can subscribe to events published by [Input Devices](./input_device) or other processors. For details about events, see the [Event](./event) documentation.

Processors can combine events, polling, and deadlines:

- **Event-driven** - React to events as they arrive
- **Polling** - Perform periodic updates at specified intervals
- **Deadline** - Run work after a resettable timeout

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

- `subscribe = [Event1, Event2, ...]`: Events to handle (see [Built-in Events](./event#built-in-events)).
- `poll_interval = <ms>`: Call `poll()` at a fixed interval greater than zero.
- `deadline`: Enable a resettable timeout using `deadline()` and `on_deadline()`.

Choose at least one option, unless you provide a [custom run loop](#custom-runnable). For timer-only processors, omit `subscribe`.

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

    #[register_processor]
    fn my_processor() -> MyProcessor {
        MyProcessor::new()
    }
}
```

Replace `#[register_processor(event)]` or `#[register_processor(poll)]` with `#[register_processor]`. Set timer options on the processor's `#[processor]` attribute.

The registration function can use `p` to take pins not used by `keyboard.toml`. Add `#[cfg(...)]` to the function to enable it conditionally.

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

Use `deadline` for a timeout that can be reset, such as deactivating a layer after the last mouse motion. `deadline()` returns when to call `on_deadline()`, or `None` to disable the timeout. Clear or advance the deadline after it fires.

```rust
use embassy_time::{Duration, Instant};
use rmk::event::PointingEvent;
use rmk::macros::processor;

#[processor(subscribe = [PointingEvent], deadline)]
pub struct MotionTimeout {
    armed_until: Option<Instant>,
}

impl MotionTimeout {
    async fn on_pointing_event(&mut self, _event: PointingEvent) {
        // Every motion pushes the deadline back
        self.armed_until = Some(Instant::now() + Duration::from_millis(500));
    }

    fn deadline(&self) -> Option<Instant> {
        self.armed_until
    }

    async fn on_deadline(&mut self) {
        self.armed_until = None;
        // Motion stopped 500ms ago
    }
}
```

You can combine `deadline` with `poll_interval` when you also need periodic work. Add a `poll()` method as shown above.

## Custom Runnable

To write your own run loop, add `#[rmk::macros::runnable_generated]` to the processor and implement `rmk::core_traits::Runnable`. This is required when combining `deadline` with `#[input_device]`.

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
