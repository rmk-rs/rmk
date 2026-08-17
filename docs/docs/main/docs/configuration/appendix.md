# Appendix

### `keyboard.toml`

The following TOML contains all available settings in `keyboard.toml`

```toml
# Basic info of the keyboard
[keyboard]
name = "RMK Keyboard" # A label for your configuration; it does not appear on USB or BLE
product_name = "RMK Keyboard" # USB product string and BLE device name, defaults to "RMK Keyboard"
vendor_id = 0x4c4b
product_id = 0x4643
manufacturer = "haobo"
serial_number = "vial:f64c2b3c:000001"
# The chip or existing board used in keyboard
# Either "board" or "chip" can be set, but not both
chip = "rp2040"
board = "nice!nano_v2"
# USB is enabled by default for most chips
# Set to false if you don't want USB
usb_enable = true

# Set matrix IO for the board. This section is for non-split keyboards and is in conflict with the [split] section
[matrix]
# `matrix_type` is optional. Default is "normal"
matrix_type = "normal"
# Input and output pins
row_pins = ["PIN_6", "PIN_7", "PIN_8", "PIN_9"]
col_pins = ["PIN_19", "PIN_20", "PIN_21"]
# RMK uses col2row as the default matrix diode direction, if you want to use a row2col matrix, add `row2col = true`
row2col = false

# Direct Pin Matrix is a Matrix of buttons connected directly to pins. It conflicts with the above.
matrix_type = "direct_pin"
direct_pins = [
    ["PIN_0", "PIN_1", "PIN_2"],
    ["PIN_3", "_", "PIN_5"]
]

# `direct_pin_low_active` is optional. Default is `true`.
# If your pin needs to be pulled up and the pin is pulled down when the button is turned on, please set it to true
# WARNING: If you use a normal matrix, it will be ineffective
direct_pin_low_active = true

# Debouncer mode, "default" or "fast". Default is "default"
debouncer = "default"
# Bootmagic: jump to the chip's USB bootloader when the key at (row, col) is held at boot. Optional
bootmagic = [0, 0]

# Layout info for the keyboard, this section is mandatory
[layout]
# Number of rows. For a split keyboard, this is the total number of rows for all splits
rows = 5
# Number of cols. For a split keyboard, this is the total number of cols for all splits
cols = 4
# keypad example
# ┌───┬───┬───┬───┐
# │NUM│ / │ * │ - │ <-- row 0, col 0..4
# ├───┼───┼───┼───┤
# │ 7 │ 8 │ 9 │   │
# ├───┼───┼───┤ + │
# │ 4 │ 5 │ 6 │   │
# ├───┼───┼───┼───┤
# │ 1 │ 2 │ 3 │ E │
# ├───┴───┼───┤ N │
# │   0   │ . │ T │
# └───────┴───┴───┘
map = """
(0,0,R)  (0,1,R)  (0,2,R)  (0,3,R)
(1,0,R)  (1,1,R)  (1,2,R)  (1,3,R)
(2,0,R)  (2,1,R)  (2,2,R)
(3,0,R)  (3,1,R)  (3,2,R)  (3,3,R)
(4,0,R)           (4,1,R)
"""
# Optional rendered-layout settings, see the Layout documentation page. They only change how
# the keyboard is drawn in Vial/Rynk, never what a key does
# Name of the [[layout.variant]] shown first
default_variant = "full"

# Custom key shapes, referenced as `@name` in `map`
[layout.shapes]
wide_zero = { w = 2.0 }

# Render variants of the same keymap: each hides some keys and reshapes others
[[layout.variant]]
name = "full"

[[layout.variant]]
name = "no_numlock"
hidden = ["(0,0)"]
shapes = { "(4,0)" = "@wide_zero" }

[keymap]
# Number of layers. Be careful, since large layer number takes more flash and RAM
# Optional: defaults to the number of `[[keymap.layer]]` entries. Set it larger
# to reserve extra empty layers (e.g. for editing in Vial/Rynk)
layers = 4

# here are the aliases for the example layer.keys below
[aliases]
MyCut = "WM(X, LCtrl)"
MyCopy = "WM(C, LCtrl)"
MyPaste = "WM(V, LCtrl)"

# Key map definitions per layer:
# The number (and order) of entries on each layer should be
# identical with the number (and order) of entries in `layout.map`.
# Empty layers will be used to fill if the number of explicitly
# defined layers is smaller than the `keymap.layers` setting

# layer 0 (default):
# (the number comes from the order of '[[keymap.layer]] entries' in the file)
[[keymap.layer]]
name = "base_layer" #optional name for the layer
keys = """
NumLock KpSlash KpAsterisk KpMinus
Kp7     Kp8     Kp9        KpPlus
Kp4     Kp5     Kp6
Kp1     Kp2     Kp3        Enter
    Kp0         KpDot
"""
# Per-layer encoder actions, one [clockwise, counter-clockwise] pair per encoder
# defined in [[input_device.encoder]]. Optional: a layer without `encoders` has no encoder actions
encoders = [["AudioVolUp", "AudioVolDown"]]

# layer 1:
[[keymap.layer]]
name = "mouse_navigation" #optional name for the layer
keys = """
TO(base_layer)   @MyCut     @MyCopy          @MyPaste
MouseBtn1        MouseUp    MouseBtn2        MouseWheelUp
MouseLeft        MouseBtn4  MouseRight
MouseWheelLeft   MouseDown  MouseWheelRight  MouseWheelDown
          MouseBtn1         MouseBtn2
"""

# Behavior configuration, if you don't want to customize anything, just ignore this section
[behavior]
# Tri Layer configuration
tri_layer = { upper = 1, lower = 2, adjust = 3 }

# OneShot configuration
one_shot = { timeout = "1s" }

# One Shot Modifiers configuration
one_shot_modifiers = { activate_on_keypress = false, quick_release = false }

[behavior.morse]
# default profile for morse, tap dance and tap-hold keys:
enable_flow_tap = true
prior_idle_time = "120ms"
hold_on_other_press = true
unilateral_tap = false
hold_timeout = "250ms"
gap_timeout = "250ms"

# list of morse (tap dance) keys:
morses = [
  # TD(0) Function key that outputs F1 on tap, F2 on double tap, layer 1 on hold
  { tap = "F1", hold = "MO(1)", double_tap = "F2" },

  # TD(1) Extended tap dance representation for function keys
  { tap_actions = ["F1", "F2", "F3", "F4", "F5"], hold_actions = ["MO(1)", "MO(2)", "MO(3)", "MO(4)", "MO(5)"] },

  # TD(2) Morse code like representation
  { morse_actions = [
      {pattern = ".-", action = "A"},
      {pattern = "-...", action = "B"},
      {pattern = "-.-.", action = "C"},
      {pattern = "-..", action = "D"},
      {pattern = ".", action = "E"},
      {pattern = "..-.", action = "F"},
      {pattern = "--.", action = "G"},
      {pattern = "....", action = "H"},
      {pattern = "..", action = "I"},
      {pattern = ".---", action = "J"},
      {pattern = "-.-", action = "K"},
      {pattern = ".-..", action = "L"},
      {pattern = "--", action = "M"},
      {pattern = "-.", action = "N"},
      {pattern = "---", action = "O"},
      {pattern = ".--.", action = "P"},
      {pattern = "--.-", action = "Q"},
      {pattern = ".-.", action = "R"},
      {pattern = "...", action = "S"},
      {pattern = "-", action = "T"},
      {pattern = "..-", action = "U"},
      {pattern = "...-", action = "V"},
      {pattern = ".--", action = "W"},
      {pattern = "-..-", action = "X"},
      {pattern = "-.--", action = "Y"},
      {pattern = "--..", action = "Z"}
    ], profile = "MRZ" }
]

[behavior.morse.profiles]
# Named overrides of the [behavior.morse] defaults, referenced by name from a morse key
# (profile = "...") or a tap-hold action's optional profile arg — e.g. a home row mod is MT(key, mod, H1).
H1 = { permissive_hold = true, unilateral_tap = true, hold_timeout = "250ms", gap_timeout = "250ms" }
H2 = { enable_flow_tap = false, permissive_hold = true, unilateral_tap = true, hold_timeout = "200ms", gap_timeout = "200ms" }
MRZ = { normal_mode = true, unilateral_tap = false, hold_timeout = "200ms", gap_timeout = "200ms" }

# Combo configuration
[behavior.combo]
timeout = "150ms"
prior_idle_time = "130ms"  # optional, prevents accidental triggers during fast typing
combos = [
  # Press J and K keys simultaneously to output Escape key
  { actions = ["J", "K"], output = "Escape" }
]

# Macro configuration
[[behavior.macro.macros]]
operations = [
    { operation = "text", text = "Hello" }
]

# Fork configuration
[behavior.fork]
forks = [
  # Shift + '.' output ':' key
  { trigger = "Dot", negative_output = "Dot", positive_output = "WM(Semicolon, LShift)", match_any = "LShift|RShift" }
]

# Auto mouse layer: activate a layer on pointing-device motion, see the Behavior documentation page.
# Note the double brackets [[ ]], one entry per pointing device id
[[behavior.auto_mouse_layer]]
# Pointing device id this entry applies to; omit for a fallback that matches any device
device_id = 0
# Layer to activate on cursor motion
target_layer = 3
# Inactivity time before the layer is deactivated, defaults to "500ms"
timeout = "600ms"
# Minimum absolute X/Y delta counted as motion, defaults to 1
threshold = 2
# Deactivate the layer on any non-mouse key press; requires [event.action] subs >= 1
deactivate_on_key = false
# Keys that don't deactivate the layer when deactivate_on_key is set
extra_mouse_keys = ["LCtrl", "LShift"]
# Non-deactivating key presses push the timeout forward
reset_timeout_on_key = false

# Input devices, see the Input Device documentation pages.
# For split keyboards, use [split.central.input_device] / [split.peripheral.input_device] instead
# Rotary encoder; its per-layer actions go in [[keymap.layer]].encoders
[[input_device.encoder]]
pin_a = "PIN_14"
pin_b = "PIN_15"
# Use the MCU's internal pull-up resistor, defaults to false
internal_pullup = false
# Working mode: "default", "e8h7" or "resolution" (then set `resolution`, or `detent` and `pulse`)
phase = "default"

# Analog joystick
[[input_device.joystick]]
name = "default"
pin_x = "P0_31"
pin_y = "P0_29"
pin_z = "_"
transform = [[80, 0], [0, 80]]
bias = [29130, 29365]
resolution = 6

# PMW3610 optical sensor. [[input_device.pmw33xx]] and [[input_device.iqs5xx]] follow the same
# pattern, see their documentation pages
[[input_device.pmw3610]]
name = "trackball0"
# Pointing device id, pairs the sensor with its PointingProcessor and [[behavior.auto_mouse_layer]]. Defaults to 0
id = 0
spi = { instance = "bitbang0", sck = "P0_05", mosi = "P0_04", miso = "P0_04", cs = "P0_09" }
# Motion interrupt pin; omit to poll the sensor
motion = "P0_02"
cpi = 800

# Lighting configuration, if you don't have any light, just ignore this section.
[light]
# LED pins, capslock, scrolllock, numslock. You can safely ignore any of them if you don't have
capslock = { pin = "PIN_0", low_active = true }
scrolllock = { pin = "PIN_1", low_active = true }
numslock = { pin = "PIN_2", low_active = true }

# Output configuration, if you don't neet to set an output pin, just ignore this section.
# Note the double brackets [[ ]], which indicate that multiple outputs can be defined.
[[output]]
# Only the pin name is required, the rest of the fields are optional
pin = "PIN_13"
initial_state_active = false
low_active = false

# Display configuration, see the Display documentation page.
# For split keyboards, use [split.central.display] / [split.peripheral.display] instead
[display]
# Display driver: "ssd1306", "sh1106", "sh1107", "sh1108" or "ssd1309"
driver = "ssd1306"
# Display resolution. Each driver accepts a fixed set of sizes, see the Display documentation page
size = "128x32"
# Display rotation in degrees: 0, 90, 180 or 270. Default is 0
rotation = 0
# Renderer, defaults to the built-in "LogoRenderer"
renderer = "OledRenderer"
# Poll interval in ms for periodic redraws; omit for event-driven rendering only
# render_interval = 33
# Minimum time in ms between event-driven renders, defaults to 33
min_render_interval = 33

# The bus the display is connected to
[display.protocol.i2c]
instance = "I2C1"
scl = "PIN_3"
sda = "PIN_2"
# 7-bit I2C address, defaults to 0x3C
address = 0x3C

# Storage configuration.
# To use the default configuration, ignore this section completely
[storage]
# Whether the storage is enabled
enabled = true
# The start address of storage
# Note: When the `dfu_rp` or `dfu_nrf` feature is enabled, this value is ignored.
# The storage partition is automatically placed after the DFU download slot.
start_addr = 0xA0000
# Number of sectors used for storage, >= 2. Defaults to 8 when a [dfu] section is present, either
# here or in the chip default (nRF52840, nice!nano, RP2040 and Pico W ship one); otherwise 2 unless
# the chip default sets a value. With DFU it cannot exceed the storage partition from rmk-memory.x
num_sectors = 16
# Clear storage at keyboard boot.
# Set it to true will reset the storage(including keymap, BLE bond info, etc.) at each reboot.
# This option is useful when testing the firmware.
clear_storage = false
# Clear the saved layout at keyboard boot, set this to true if you want to reset the layout
clear_layout = false

# DFU configuration (embassy-boot, experimental), see the Bootloader documentation page.
# All fields are optional. Partition offsets are NOT set here — they come from the
# `memory.x` linker symbols generated by rmk-boot.
[dfu]
# DFU activity LED pin ("none" to disable; defaults to "PIN_25" on RP2040, "P0_15" on nRF52)
led = "PIN_25"
# Unlock keys for the DFU lock (requires the `dfu_lock` Cargo feature)
unlock_keys = [[0, 0], [1, 1]]

# Ble configuration
# To use the default configuration, ignore this section completely
[ble]
# Whether the ble is enabled
enabled = true
# BLE related pins, ignore any of them if you don't have
battery_adc_pin = "vddh"
# If the voltage divider is used for adc, you can use the following two values to define a voltage divider.
# For example, nice!nano have 806 + 2M resistors, the saadc measures voltage on 2M resistor, so the two values should be set to 2000 and 2806
# Measured resistance for input adc, it should be less than adc_divider_total
adc_divider_measured = 2000
# Total resistance of the full path for input adc
adc_divider_total = 2806
# BLE tx power; higher means better signal but more power consumption. nRF52 only, ignored on other chips
default_tx_power = 0
# Whether to enable 2M PHY, defaults to true. nRF52 only, ignored on other chips
use_2m_phy = true
# Enable passkey entry during BLE pairing, defaults to false
passkey_entry = false
# Timeout in seconds for passkey entry, defaults to 120, minimum 30
passkey_entry_timeout = 120
# [Deprecated] Pin that reads battery's charging state, `low-active` means the battery is charging when `charge_state.pin` is low
# Input pin that indicates the charging state
# charge_state = { pin = "PIN_1", low_active = true }
# [Deprecated] Output LED pin that blinks when the battery is low
# charge_led= { pin = "PIN_2", low_active = true }

# RMK internal configuration
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
# (Each morse key is a programmable multi-tap/hold key)
morse_max_num = 8
# Capacity of the named morse profile table ([behavior.morse.profiles], max 255)
morse_profile_max_num = 16
# Maximum number of patterns a morse key can handle (min 4, max 32; raised automatically to fit the
# largest configured morse key)
max_patterns_per_key = 32
# Macro space size in bytes for storing sequences
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
# Rynk RX/TX buffer size in bytes. 488 bytes = 2*BLE maximum packet size
rynk_buffer_size = 488
# Length of one dongle pairing window in seconds. Dongle builds only; repeated
# while no keyboard is bonded, opened once at power-on otherwise
dongle_pairing_window_secs = 30
# Maximum number of [[behavior.auto_mouse_layer]] entries.
# Auto-derived from the number of configured entries when unset
# auto_mouse_layer_max_num = 2

# Event channel configuration.
# Tune the pub/sub channel of a single event type with [event.<name>], where
# <name> is one of: connection_status_change, modifier, keyboard, layer_change,
# wpm_update, led_indicator, sleep_state, battery_status, battery_adc,
# charging_state, pointing, peripheral_connected, central_connected,
# peripheral_battery, clear_peer, dfu_status, action.
# All defaults are in rmk-config/src/default_config/event_default.toml
[event.keyboard]
# Channel buffer size
channel_size = 16
# Number of publishers
pubs = 2
# Number of subscribers
subs = 3

# Split configuration
# This section conflicts with the [matrix] section. You can only have either [matrix] or [split], but NOT BOTH
[split]
# Connection type of split, "serial" or "ble"
connection = "serial"

# Split central config
[split.central]
# Number of rows on central board
rows = 2
# Number of cols on central board
cols = 2
# Row offset of central matrix to the whole matrix
row_offset = 0
# Col offset of central matrix to the whole matrix
col_offset = 0
# If the connection type is "serial", the serial instances used on the central board are defined using "serial" field.
# It's a list of serial instances with a length equal to the number of splits.
# The order of the serial instances is important: the first serial instance on the central board
# communicates with the first split peripheral defined, and so on.
serial = [
    { instance = "UART0", tx_pin = "PIN_0", rx_pin = "PIN_1" },
    { instance = "UART1", tx_pin = "PIN_4", rx_pin = "PIN_5" },
    # For the RP2040 only, you can also use RMK's Programmable IO (PIO) UART serial port using either or both of the RP2040's two PIO blocks, PIO0 and PIO1, by enabling the RMK `rp2040` feature gate in Cargo.toml.
    # The PIO serial port can be used in half-duplex mode using the same pin for RX/TX
    { instance = "PIO0", tx_pin = "PIN_6", rx_pin = "PIN_6" },
    # Or use the PIO serial port in full-duplex mode using different pins for RX/TX
    { instance = "PIO1", tx_pin = "PIN_7", rx_pin = "PIN_8" },
]
# If the connection type is "ble", we can override the BLE static address used by setting `ble_addr`.
# This address should be a valid BLE random static address, see: https://academy.nordicsemi.com/courses/bluetooth-low-energy-fundamentals/lessons/lesson-2-bluetooth-le-advertising/topic/bluetooth-address/
ble_addr = [0x18, 0xe2, 0x21, 0x80, 0xc0, 0xc7]
# Optional battery ADC config for the central (overrides [ble] battery settings)
battery_adc_pin = "vddh"
adc_divider_measured = 2000
adc_divider_total = 2806

[split.central.matrix]
matrix_type = "normal"
# Matrix IO definition on central board
row_pins = ["PIN_9", "PIN_11"]
col_pins = ["PIN_10", "PIN_12"]
# Bootmagic key of the central board, in the central's local (row, col). Optional
bootmagic = [0, 0]

# Output configuration for split central (optional)
[[split.central.output]]
# Only the pin name is required, the rest of the fields are optional
pin = "PIN_12"
initial_state_active = false
low_active = false

# Display on the central board (optional), same fields as [display]
[split.central.display]
driver = "ssd1306"
size = "128x64"

[split.central.display.protocol.i2c]
instance = "I2C1"
scl = "PIN_3"
sda = "PIN_2"

# Input devices on the central board (optional), same tables as [input_device]
[[split.central.input_device.encoder]]
pin_a = "PIN_14"
pin_b = "PIN_15"

# Configuration for the first split peripheral
# Note the double brackets [[ ]], which indicate that multiple split peripherals can be defined.
# The order of peripherals is important: it should match the order of the serial instances (if serial is used).
[[split.peripheral]]
# Number of rows on peripheral board
rows = 2
# Number of cols on peripheral board
cols = 1
# Row offset of peripheral matrix to the whole matrix
row_offset = 2
# Col offset of peripheral matrix to the whole matrix
col_offset = 2
# The serial instance used to communicate with the central board, if the connection type is "serial"
serial = [{ instance = "UART0", tx_pin = "PIN_0", rx_pin = "PIN_1" }]
# Override the BLE random static address of the peripheral board
ble_addr = [0x7e, 0xfe, 0x73, 0x9e, 0x66, 0xe3]
# Optional battery ADC config for this peripheral
battery_adc_pin = "P0_02"
adc_divider_measured = 2000
adc_divider_total = 2806
# Peripheral firmware for automatic dfu_split updates (requires the `dfu_split` Cargo feature),
# path relative to Cargo.toml. See the Bootloader documentation page
firmware = "./peripheral.bin"
# "MatchHash" (default) flashes only when the peripheral's firmware differs; "force" flashes at every start
update_policy = "MatchHash"

[split.peripheral.matrix]
matrix_type = "normal"
# Matrix IO definition on peripheral board
row_pins = ["PIN_9", "PIN_11"]
col_pins = ["PIN_10"]
# Bootmagic key of this peripheral, in the peripheral's local (row, col). Optional
bootmagic = [0, 0]

# Output configuration, if you don't need to set an output pin, just ignore this section.
# Note the double brackets [[ ]], which indicate that multiple outputs can be defined.
[[split.peripheral.output]]
# Only the pin name is required, the rest of the fields are optional
pin = "PIN_13"
initial_state_active = false
low_active = false

# Display on this peripheral (optional), same fields as [display]
[split.peripheral.display]
driver = "ssd1306"
size = "128x32"

[split.peripheral.display.protocol.i2c]
instance = "I2C1"
scl = "PIN_3"
sda = "PIN_2"

# Input devices on this peripheral (optional), same tables as [input_device]
[[split.peripheral.input_device.encoder]]
pin_a = "PIN_14"
pin_b = "PIN_15"

# More split peripherals (if you have any)
[[split.peripheral]]
# The configuration is the same as the first split peripheral
...
...
...

# Dependency config
[dependency]
# Whether to enable defmt, set to false for reducing binary size
defmt_log = true

# Host-side tools configuration
[host]
# Whether Vial is enabled (default: true in keyboard.toml config)
vial_enabled = true
# Whether Rynk is enabled (experimental, default: false in keyboard.toml config)
# Rynk and Vial are mutually exclusive and must match Cargo features.
rynk_enabled = false
# The unlock keys are the combo of the row 0, col 0 key and
# the row 0, col 1 key. Shared by the Vial lock and the Rynk lock gate.
unlock_keys = [[0, 0], [0, 1]]
# Start (and stay) unlocked. Renamed from `vial_insecure` (still parses).
insecure = false
# Rynk only: move config writes into the locked tier (default: false).
write_requires_unlock = false

# Chip-specific configuration
# To use the default configuration, ignore this section completely
# Use chip-specific sections like [chip.nrf52840] for chip-specific settings
[chip.nrf52840]
# DCDC regulator 0 enabled (nrf52840 only; default: false for chip = "nrf52840" and the XIAO BLE,
# nrfmicro, bluemicro840 and puchi_ble boards, true for nice!nano and nice!nano_v2)
# **Note**: Do not enable DC/DC regulator without an external LC filter being connected
# as this will inhibit device operation, including debug access, until an LC filter is connected.
dcdc_reg0 = true
# DCDC regulator 1 enabled (nrf52840, nrf52833; default: false for chip = "nrf52840" and the XIAO BLE,
# nrfmicro, bluemicro840 and puchi_ble boards, true for nice!nano, nice!nano_v2 and nrf52833)
# **Note**: Do not enable DC/DC regulator without an external LC filter being connected
# as this will inhibit device operation, including debug access, until an LC filter is connected.
dcdc_reg1 = true
# DCDC regulator 0 voltage (nrf52840 only, default: "3V3")
# Valid values: "3V3" or "1V8"
dcdc_reg0_voltage = "3V3"
```

### Available chip names

Available chip names in `chip` field:

- rp2040
- nrf52840
- nrf52833
- nrf52832
- nrf52811
- nrf52810
- esp32c3
- esp32c6
- esp32h2
- esp32s3
- ALL stm32s supported by [embassy-stm32](https://github.com/embassy-rs/embassy/blob/main/embassy-stm32/Cargo.toml) with USB

### Available board names

Available board names in `board` field:

- `nice!nano`
- `nice!nano_v2`
- `XIAO BLE`
- `nrfmicro`
- `bluemicro840`
- `puchi_ble`
- `pi_pico_w`

If you want to add more built-in boards, feel free to open a PR!
