# Input Device

RMK's input device system provides a unified interface for hardware components that generate input events.

::: info What are input devices?
Input devices are hardware components that provide input to the keyboard firmware, such as key matrix, rotary encoders, joysticks, pointing devices, ADC sensors, and more.
:::

## Overview

Input devices read from hardware peripherals and publish events. These events are then consumed by [Processors](./processor) or the keyboard core. For details about events, see the [Event](./event) documentation.

## InputDevice Trait

The `InputDevice` trait defines the interface for input devices:

```rust
pub trait InputDevice: Runnable {
    type Event;
    async fn read_event(&mut self) -> Self::Event;
}
```

You don't need to implement this trait manually. Use the `#[input_device]` macro to generate the implementation automatically.

## Defining Input Devices

Use the `#[input_device]` macro on a struct to define an input device:

```rust
use rmk::event::BatteryAdcEvent;
use rmk::macros::input_device;

#[input_device(publish = BatteryAdcEvent)]
pub struct MyBatteryReader {
    pin: u8,
}
```

**Parameters:**

- `publish = EventType` (required): The event type this device publishes.

**How it works:**

- `#[input_device]` implements both `InputDevice` and `Runnable` traits automatically.
- You only need to implement a `read_<event_name>_event()` method that **returns** the event. The macro will automatically publish the returned event to the corresponding event channel.
- The method name is derived from the event type name by converting it to snake_case and stripping the `Event` suffix:
  - `publish = BatteryAdcEvent` → `async fn read_battery_adc_event(&mut self) -> BatteryAdcEvent`
  - `publish = KeyboardEvent` → `async fn read_keyboard_event(&mut self) -> KeyboardEvent`

### Single-event Device Example

A device that reads charging state from a GPIO pin:

```rust
use rmk::event::ChargingStateEvent;
use rmk::macros::input_device;

#[input_device(publish = ChargingStateEvent)]
pub struct ChargingStateReader<I: InputPin> {
    state_input: I,
    low_active: bool,
    current_charging_state: bool,
}

impl<I: InputPin> ChargingStateReader<I> {
    pub fn new(state_input: I, low_active: bool) -> Self {
        Self {
            state_input,
            low_active,
            current_charging_state: false,
        }
    }

    // This method is required by #[input_device(publish = ChargingStateEvent)]
    async fn read_charging_state_event(&mut self) -> ChargingStateEvent {
        loop {
            embassy_time::Timer::after_secs(5).await;
            let charging = if self.low_active {
                self.state_input.is_low().unwrap_or(false)
            } else {
                self.state_input.is_high().unwrap_or(false)
            };
            if charging != self.current_charging_state {
                self.current_charging_state = charging;
                return ChargingStateEvent { charging };
            }
        }
    }
}
```

### Multi-event Device Example

A device that produces multiple event types using a wrapper enum (see [Multi-event Enums](./event#multi-event-enums)):

```rust
use rmk::event::{BatteryAdcEvent, PointingEvent};
use rmk::macros::{Event, input_device};

// Define a wrapper enum for multiple event types
#[derive(Event, Clone, Debug)]
pub enum NrfAdcEvent {
    Pointing(PointingEvent),
    Battery(BatteryAdcEvent),
}

#[input_device(publish = NrfAdcEvent)]
pub struct NrfAdc<'a, const PIN_NUM: usize, const EVENT_NUM: usize> {
    saadc: Saadc<'a, PIN_NUM>,
    // ... other fields
}

impl<'a, const PIN_NUM: usize, const EVENT_NUM: usize> NrfAdc<'a, PIN_NUM, EVENT_NUM> {
    // Returns the wrapper enum
    async fn read_nrf_adc_event(&mut self) -> NrfAdcEvent {
        // Read ADC and return the appropriate variant
        NrfAdcEvent::Battery(BatteryAdcEvent(adc_value))
    }
}
```

## Running Input Devices

All input devices implement the `Runnable` trait. Use the `run_all!` macro to run all runnables — input devices, processors, and the rest of the firmware (keyboard, storage, transports) — concurrently:

```rust
use rmk::run_all;

// Create your devices and processors
let mut matrix = Matrix::<_, _, _, ROW, COL, true>::new(row_pins, col_pins, debouncer);
let mut encoder = RotaryEncoder::new(pin_a, pin_b, 0);
let mut adc_device = NrfAdc::new(saadc, [AnalogEventType::Battery], [0], interval, None);
let mut batt_proc = BatteryProcessor::new(2000, 2806);

// Run everything concurrently
run_all!(
    matrix,
    encoder,
    adc_device,
    batt_proc,
    storage,
    usb_transport,
    keyboard
)
.await;
```

See [`examples/use_rust/nrf52840_ble`](https://github.com/rmk-rs/rmk/blob/main/examples/use_rust/nrf52840_ble/src/main.rs) for a complete `main.rs` using these devices.

## Configuration

RMK provides `keyboard.toml` configuration support for some built-in input devices (such as rotary encoders, joysticks, PMW3610 and PMW3360/PMW3389 sensors, and IQS5xx trackpads), so you can use them without writing any Rust code. See the [Input Device Configuration](../configuration/input_device) documentation for details.

## Combining with Processor

`#[input_device]` can be combined with `#[processor]` on the same struct. This allows a single struct to both produce events and subscribe to other events:

```rust
use rmk::macros::{input_device, processor};

#[input_device(publish = SensorEvent)]
#[processor(subscribe = [ConfigEvent])]
pub struct InputSensor {
    pub threshold: u16,
}

impl InputSensor {
    // Required by #[input_device]: return the event, it will be published automatically
    async fn read_sensor_event(&mut self) -> SensorEvent {
        // Read sensor data
    }

    // Required by #[processor]: handle subscribed events
    async fn on_config_event(&mut self, event: ConfigEvent) {
        self.threshold = event.threshold;
    }
}
```

::: warning Beware of infinite event loops
When combining input device and processor, be careful not to create event loops:

- **Direct loop**: Subscribing to an event you publish yourself (RMK rejects this case at compile time)
- **Indirect loop**: Device A subscribes to X and publishes Y, Device B subscribes to Y and publishes X — this forms a cycle
- **Longer chains**: A→B→C→A loops are also possible (A publishes B, B publishes C, C publishes A)

Event loops cause infinite cycles and hang your firmware.
:::

## Related Documentation

- [Event](./event) - Event concepts, built-in events, and custom event definition
- [Processor](./processor) - How to create processors that subscribe to events
