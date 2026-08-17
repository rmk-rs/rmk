# Event Configuration

RMK allows you to tune event channel parameters in `keyboard.toml` based on your specific needs and hardware constraints.

For an overview of events and how to define custom events, see the [Event](../features/event) documentation.

## Configuration Parameters

Each event channel has three configurable parameters:

- **`channel_size`**: Buffer size - how many events can be queued
- **`pubs`**: Number of publishers - how many concurrent tasks can publish
- **`subs`**: Number of subscribers - how many concurrent tasks can subscribe

Each event has default values for typical use cases. You can view all defaults in [`rmk-config/src/default_config/event_default.toml`](https://github.com/rmk-rs/rmk/blob/main/rmk-config/src/default_config/event_default.toml).

The `subs` values are base counts. When a Cargo feature is enabled, RMK adds the subscribers its own tasks need on top of `subs`, whether the value comes from the defaults or from your `[event.<name>]` entry. For example, the internal `_ble` feature (enabled by every BLE chip feature) adds 1 to `keyboard` and `pointing`, `split` adds 2 to `led_indicator`, and `display` adds 1 to `wpm_update`, `modifier`, `sleep_state`, `layer_change`, `battery_status` and `led_indicator`. The full list of bumps is in [`rmk-config/src/default_config/subscriber_default.toml`](https://github.com/rmk-rs/rmk/blob/main/rmk-config/src/default_config/subscriber_default.toml). If you add your own subscribers, raise `subs` from its default by that number; you don't need to count the feature-gated built-in subscribers.

## Configuration Syntax

Add an `[event]` section to your `keyboard.toml`:

```toml
[event]
# Configure specific parameters for individual events
event_name.channel_size = <value>
event_name.pubs = <value>
event_name.subs = <value>
```

**Examples:**

```toml
[event]
# Increase key event buffer for fast typing
keyboard.channel_size = 32

# Add more subscribers for multiple displays monitoring layer changes
layer_change.subs = 8

# Reduce subscribers to save memory on constrained devices
keyboard.subs = 2
led_indicator.subs = 2

# Configure multiple parameters for one event
peripheral_battery.channel_size = 4
peripheral_battery.subs = 4
```

## Configurable Event Names

| Config Name                | Event Type                    | Default Notes                   |
| -------------------------- | ----------------------------- | ------------------------------- |
| **Input Events**           |                               |                                 |
| `keyboard`                 | `KeyboardEvent`               | channel_size=16, pubs=2, subs=3 |
| `modifier`                 | `ModifierEvent`               | channel_size=8, subs=2          |
| `pointing`                 | `PointingEvent`               | channel_size=8, pubs=2, subs=2  |
| **State Events**           |                               |                                 |
| `layer_change`             | `LayerChangeEvent`            | pubs=2                          |
| `wpm_update`               | `WpmUpdateEvent`              |                                 |
| `led_indicator`            | `LedIndicatorEvent`           | channel_size=2, pubs=2, subs=3  |
| `sleep_state`              | `SleepStateEvent`             |                                 |
| **Battery Events**         |                               |                                 |
| `battery_adc`              | `BatteryAdcEvent`             | channel_size=2                  |
| `charging_state`           | `ChargingStateEvent`          | channel_size=2                  |
| `battery_status`           | `BatteryStatusEvent`          | subs=0                          |
| **Connection Events**      |                               |                                 |
| `connection_status_change` | `ConnectionStatusChangeEvent` | channel_size=2, pubs=2          |
| **Split Events**           |                               |                                 |
| `peripheral_connected`     | `PeripheralConnectedEvent`    |                                 |
| `central_connected`        | `CentralConnectedEvent`       |                                 |
| `peripheral_battery`       | `PeripheralBatteryEvent`      | channel_size=2, subs=2          |
| `clear_peer`               | `ClearPeerEvent`              | subs=0                          |
| **DFU Events**             |                               |                                 |
| `dfu_status`               | `DfuStatusEvent`              | channel_size=2                  |
| **Action Events**          |                               |                                 |
| `action`                   | `ActionEvent`                 | channel_size=16, subs=0         |

Unlisted parameters default to `1`. The `subs` values are base counts before the feature bumps described above.

## Related Documentation

- [Event](../features/event) - Event concepts, built-in events, and custom event definition
- [Input Device](../features/input_device) - How to create input devices that publish events
- [Processor](../features/processor) - How to create processors that subscribe to events
