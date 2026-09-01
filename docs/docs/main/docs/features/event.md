# Event

RMK uses an event-driven architecture for communication between components. Events are type-safe messages that flow through channels, connecting input devices, processors, and the keyboard core.

```
┌───────────┐
│InputDevice│
└───────────┘
      │
   publish
      │
      ▼
┌───────────┐
│   Event   │◀─┐
└───────────┘  │
      │        │
  subscribe publish
      │        │
      ▼        │
┌───────────┐  │
│ Processor │──┘
└───────────┘
```

## Built-in Events

RMK provides built-in event types organized by category. All of them are exported from the `rmk::event` module:

**Input Events**:

- `KeyboardEvent` - Key press/release event from matrix or encoders
- `ModifierEvent` - Modifier key combination changes
- `PointingEvent` - Pointing device events (mouse movement, scroll)
- `PointingSetCpiEvent` - Request to set the CPI (resolution) of a pointing device by `device_id`
- `PointingProcessorEvent` - Switch a pointing device's `PointingMode` (cursor, scroll, sniper, caret)
- `ActionEvent` - The `Action` resolved from a key press/release, published before it is executed. `[event.action].subs` defaults to `0`; raise it in `keyboard.toml` before subscribing

**State Events**:

- `LayerChangeEvent` - Active layer changed
- `LedIndicatorEvent` - LED indicator state changed (NumLock, CapsLock, ScrollLock)
- `WpmUpdateEvent` - Words per minute updated
- `SleepStateEvent` - Sleep state changed

**Battery Events**:

- `BatteryAdcEvent` - Raw battery ADC reading
- `ChargingStateEvent` - Charging state changed
- `BatteryStatusEvent` - Battery status changed (includes level and charging status)

**Connection Events**:

- `ConnectionStatusChangeEvent` - Full `ConnectionStatus` snapshot (USB lifecycle, BLE profile/state, preferred transport); fires on every transition

**Split Keyboard Events** (when split is enabled):

- `PeripheralConnectedEvent` - Peripheral connection state changed
- `CentralConnectedEvent` - Connected to central state changed
- `PeripheralBatteryEvent` - Peripheral battery status changed
- `ClearPeerEvent` - BLE peer clearing event (BLE split builds only)

**DFU Events** (when the `dfu` feature is enabled):

- `DfuStatusEvent` - DFU status changed (`DfuStatus`: idle, started, downloading, finished, error, lock waiting, unlocked)

## Defining Custom Events

Use the `#[event]` macro to define custom events:

```rust
use rmk::macros::event;

// Channel - each event consumed by ONE subscriber
#[event(channel_size = 2)]
#[derive(Clone, Copy, Debug)]
pub struct MyCustomEvent(pub u16);

// PubSub - each event received by ALL subscribers
#[event(channel_size = 2, subs = 4, pubs = 1)]
#[derive(Clone, Copy, Debug)]
pub struct AnotherEvent {
    pub value: u8,
}
```

**Parameters:**

- `channel_size` (optional): Buffer size of the event channel. Default is 8 for Channel, 1 for PubSub.
- `subs` (optional): Max subscribers. If specified, uses PubSub channel. Default is 4.
- `pubs` (optional): Max publishers. If specified, uses PubSub channel. Default is 1.

::: note Channel types

- **Channel**: Each event is consumed by **one** subscriber. If multiple subscribers exist, only one receives each event.
- **PubSub**: Each event is broadcast to **all** subscribers. Specify `subs` or `pubs` to enable.
  :::

### Multi-event Enums

When a component produces multiple types of events, use `#[derive(Event)]` on an enum:

```rust
use rmk::event::{BatteryAdcEvent, PointingEvent};
use rmk::macros::Event;

#[derive(Event, Clone, Debug)]
pub enum NrfAdcEvent {
    Pointing(PointingEvent),
    Battery(BatteryAdcEvent),
}
```

When published, each variant is automatically routed to its underlying event channel.

## Publishing Events

Events are published using `publish_event` or `publish_event_async`:

```rust
use rmk::event::{publish_event, publish_event_async};

// Synchronous (immediate, non-blocking)
publish_event(MyCustomEvent(42));

// Asynchronous (awaitable, may block if channel is full)
publish_event_async(MyCustomEvent(42)).await;
```

`publish_event` never waits: on a full Channel the event is dropped, and on a full PubSub the oldest unread event is overwritten. Use `publish_event_async` when every event must be delivered.

## Related Documentation

- [Input Device](./input_device) - How to create input devices that publish events
- [Processor](./processor) - How to create processors that subscribe to events
- [Event Configuration](../configuration/event) - How to configure event channels in keyboard.toml
