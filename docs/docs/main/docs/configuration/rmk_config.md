# RMK Internal Configuration

The `[rmk]` section defines configuration parameters used inside RMK. These parameters affect the firmware's behavior, memory usage, and performance. If you don't need to change these parameters, you can ignore this section.

## Configuration Example

```toml
[rmk]
# Mouse key interval (ms) - controls mouse movement speed
mouse_key_interval = 20
# Mouse wheel interval (ms) - controls scrolling speed
mouse_wheel_interval = 80
# Maximum number of combos keyboard can store
combo_max_num = 8
# Maximum number of keys pressed simultaneously in a combo
combo_max_length = 4
# Maximum number of forks for conditional key actions
fork_max_num = 8
# Maximum number of morse keys keyboard can store (max 255)
morse_max_num = 8
# Maximum number of named morse profiles, shared by morse and tap-hold keys (max 255)
morse_profile_max_num = 16
# Maximum number of patterns a morse key can handle (default: 8, min: 4, max: 32)
max_patterns_per_key = 8
# Macro space size in bytes for storing sequences. The maximum number of Macros depends on the size of each sequence: All sequences combined need to fit into macro_space_size, the number of macro sequences doesn't matter.
macro_space_size = 256
# Default debounce time in ms
debounce_time = 20
# Report channel size
report_channel_size = 16
# Vial channel size
vial_channel_size = 4
# Flash channel size
flash_channel_size = 4
# The number of the split peripherals
split_peripherals_num = 0
# The number of available BLE profiles
ble_profiles_num = 3
# BLE Split Central sleep timeout in seconds (0 = disabled)
split_central_sleep_timeout_seconds = 0
# Maximum macro data bytes in one Rynk macro request or response
protocol_macro_chunk_size = 64
# Maximum number of auto mouse layer entries (auto-derived from [[behavior.auto_mouse_layer]] if unset)
auto_mouse_layer_max_num = 2
# Rynk RX/TX buffer size in bytes. 488 bytes = 2*BLE maximum packet size
# rynk_buffer_size = 488
# Length of one dongle pairing window in seconds (dongle builds only)
# dongle_pairing_window_secs = 30
```

## Parameter Details

### Mouse-Related Configuration

- `mouse_key_interval`: Mouse key interval in milliseconds, default value is 20. This parameter controls the mouse movement speed; lower values result in faster movement.
- `mouse_wheel_interval`: Mouse wheel interval in milliseconds, default value is 80. This parameter controls the scrolling speed; lower values result in faster scrolling.

### Behavior Configuration

::: info

Increasing the number of combos, forks, morses (tap dances), and macros will increase memory usage.

:::

- `combo_max_num`: Maximum number of combos that the keyboard can store, default value is 8. This value must be between 0 and 255.
- `combo_max_length`: Maximum number of keys that can be pressed simultaneously in a combo, default value is 4. This value must be between 0 and 16.
- `fork_max_num`: Maximum number of forks for conditional key actions, default value is 8. This value must be between 0 and 255.
- `morse_max_num`: Maximum number of morses that can be stored, default value is 8. This value must be between 0 and 255.
- `morse_profile_max_num`: Capacity of the morse profile table (the named profiles in `[behavior.morse.profiles]`, referenced by morse and tap-hold keys), default value is 16. This value must be between 0 and 255.
- `max_patterns_per_key` : Maximum number of tap/hold patterns a morse key can handle, default value is 8. This value must be between 4 and 32. (Automatically raised to fit the largest `tap_actions` + `hold_actions` + `morse_actions` count among the configured morse keys.)
- `macro_space_size`: Space size in bytes for storing macro sequences, default value is 256. This value must be between 0 and 65535.

### Matrix Configuration

- `debounce_time`: Default key debounce time in milliseconds, default value is 20.

### Channel Configuration

In RMK there are several channels used for communication between tasks. The length of the channel can be adjusted. Larger channel size means more events can be buffered, but it will increase memory usage.

Per-event pub/sub channels (key events, battery events, etc.) are configured separately in the [`[event]`](./event) section.

- `report_channel_size`: The length of report channel, default value is 16. Used for buffering HID reports to be sent to the host.
- `vial_channel_size`: The length of the legacy Vial channel, default value is 4. Used only by Vial builds with BLE enabled, where it buffers Vial packets received over BLE.
- `flash_channel_size`: The length of flash channel, default value is 4. Used for buffering flash storage operations.

### Rynk Protocol Configuration

These tune the [Rynk](../features/rynk) protocol and rarely need changing.

- `protocol_macro_chunk_size`: How many macro bytes a single macro transfer can carry, default value is 64. This value must be between 0 and 256. Smaller chunks use less firmware RAM but need more back-and-forth with the host.
- `rynk_buffer_size`: Size of Rynk's send/receive frame buffers in bytes. Payload capacity and bulk batch sizes derive from it. Default value is 488, which fills exactly two BLE notifications. This value must be between 0 and 65535.

### Split Keyboard Configuration

- `split_peripherals_num`: The number of split peripherals, default value is 0. If peripherals are specified in `keyboard.toml`, this value is automatically set to the actual count. If you're using the Rust API without `[[split.peripheral]]` entries, set this manually to match your peripheral count. This value must be between 0 and 255; when the `split` feature is enabled, it is raised to at least 1.

### Wireless Configuration

- `ble_profiles_num`: The number of available Bluetooth profiles, default value is 3. This parameter defines how many Bluetooth paired devices the keyboard can store. This value must be between 0 and 255.
- `split_central_sleep_timeout_seconds`: Sleep timeout for BLE split central in seconds, default value is 0 (disabled). When set to a non-zero value, the split central will enter sleep mode after this many seconds of inactivity to save power. Set to 0 to disable automatic sleep.
- `dongle_pairing_window_secs`: Length of one dongle pairing window in seconds, default value is 30. Used only by builds with the `dongle` feature. A dongle without a bonded keyboard repeats the window until one pairs. A bonded dongle opens it exactly once, at power-on, and afterwards reconnects only its bonded keyboard.

### Auto Mouse Layer Configuration

- `auto_mouse_layer_max_num`: Maximum number of `[[behavior.auto_mouse_layer]]` entries. Auto-derived from your config if unset; set explicitly to override (e.g. when using the Rust API without `keyboard.toml`).
