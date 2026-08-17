# Azoteq IQS5xx Trackpad

The Azoteq IQS5xx-B000 family (IQS550, IQS572, IQS525) are I²C capacitive
trackpad controllers, commonly used in keyboards via Azoteq's TPS43 and TPS65
trackpad modules.

::: note

- `keyboard.toml` configuration is supported on nRF52 and RP2040 only; other chips
  need the [Rust API](#rust-configuration).
- Currently only relative single-finger cursor movement is reported. Gestures,
  multi-finger absolute positions, pressure, area, and raw channel data are
  read from the IC but not yet published as RMK events.
- Scaling is not supported yet; cursor movements will likely feel fast and
  imprecise.
- An `RDY` (ready) pin is strongly recommended. Without it, the driver falls
  back to timed polling and may stall the I²C bus through clock-stretching if
  it polls mid-cycle. See [RDY vs polling](#rdy-vs-polling).
- Each `[[input_device.iqs5xx]]` claims its own I²C peripheral. Sharing a bus
  with another I²C device (e.g. an OLED) isn't supported yet.
- Persisting parameters to the IC's non-volatile memory (which requires
  toggling `NRST`) isn't supported; configuration is rewritten on every boot.

:::

## Hardware

- `SDA` / `SCL` — I²C bus, 7-bit address `0x74`.
- `RDY` — active-high digital output the device drives high during the I²C
  communication window. Connect to a GPIO that supports async edge waits
  (`embedded_hal_async::digital::Wait`).
- `NRST` — active-low reset. Not used by this driver, but required if you
  want to persist parameters to the IC's non-volatile memory (out of scope
  here).

## `toml` configuration

```toml
[[input_device.iqs5xx]]
name = "trackpad0"
id = 0 # optional 0-255 pointing-device id; the generated PointingProcessor
       # is paired to it. Defaults to 0.

i2c.instance = "I2C0"  # RP2040: I2C0 / I2C1.  nRF52: TWISPI0 / TWISPI1.
i2c.sda = "PIN_4"
i2c.scl = "PIN_5"

# Optional: RDY (data-ready) pin. Strongly recommended.
rdy = "PIN_15"

# Axis tweaks applied in PointingProcessor.
# proc_invert_x = true
# proc_invert_y = true
# proc_swap_xy = true
```

### Split

To add the trackpad to the central or a peripheral:

```toml
[[split.central.input_device.iqs5xx]]
name = ...

# resp.
[[split.peripheral.input_device.iqs5xx]]
name = ...
```

For split keyboards the device runs on whichever side it's wired to; the
matching `PointingProcessor` is generated on the central automatically.

## Rust configuration

Construct the device directly. For a split keyboard, add the device to whichever
side (`central.rs` or `peripheral.rs`) the trackpad is physically wired to.

```rust
use embassy_rp::gpio::{Input, Pull};
use embassy_rp::i2c::{Config, I2c};
use rmk::input_device::iqs5xx::Iqs5xx;
use rmk::input_device::pointing::{PointingProcessor, PointingProcessorConfig};

// 1. Bring up the I2C bus the trackpad is on.
let mut i2c_cfg = Config::default();
i2c_cfg.frequency = 400_000;
let i2c = I2c::new_async(p.I2C0, p.PIN_5, p.PIN_4, Irqs, i2c_cfg);

// 2. Configure the RDY pin (recommended). Use `None` if you don't have one.
let rdy = Some(Input::new(p.PIN_15, Pull::None));

// 3. Construct the device. The first argument is an RMK pointing-device id;
//    pick any 0-255, just don't reuse it for another pointing device.
const POINTING_DEV_ID: u8 = 0;
let mut trackpad = Iqs5xx::new(POINTING_DEV_ID, i2c, rdy);

// 4. Add a PointingProcessor on the central side to convert motion events
//    into mouse reports. Axis tweaks (invert / swap) live here.
let proc_config = PointingProcessorConfig {
    // invert_x: true,
    // invert_y: true,
    // swap_xy: true,
    ..Default::default()
};
let mut trackpad_proc = PointingProcessor::new(&keymap, proc_config);

run_all!(trackpad, trackpad_proc, /* matrix, ... */);
```

::: note

`PointingProcessor` must run on the **central** side, even if the trackpad is
wired to a peripheral. The peripheral runs the `Iqs5xx` device and forwards
events over the split link; the central converts them to USB/BLE HID reports.

You can switch between Cursor, Scroll, Sniper and Caret modes per layer.
See the [PointingProcessor](./pointing_processor) page for all options.

:::

## RDY vs polling

The IQS5xx alternates between _scanning_ the touch panel and an I²C
_communication window_. When a window is open it drives `RDY` high.

- **With `RDY`**: the driver waits for `RDY` high before issuing I²C reads,
  so transactions complete inside the window with no clock-stretching. The
  driver puts the IC into "event mode" so the device only opens a window when
  it actually has touch data, which keeps idle bus traffic minimal.
- **Without `RDY`** (`rdy = None` / no `rdy` in TOML): the driver issues
  reads on a fixed ~15 ms cadence. If a read lands mid-scan the IC
  clock-stretches SCL until the current cycle ends, freezing any device
  sharing the bus. The driver compensates with a conservative report
  interval and a longer per-transaction timeout, but you may still see
  latency spikes — particularly during long holds.

If your PCB doesn't route `RDY`, hand-soldering a jumper to a spare GPIO is
generally worth it.

## References

- [IQS5xx-B000 Trackpad and Touchpad Datasheet (Azoteq)](https://www.azoteq.com/images/stories/pdf/iqs5xx-b000_trackpad_datasheet.pdf)
