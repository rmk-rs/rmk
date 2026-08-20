# Low Power

RMK supports low-power mode by utilizing the `Wait` trait in `embedded-hal-async`.

## Usage

By default, RMK uses a busy-loop for matrix scanning, which is not very power efficient. To enable the low-power mode, add the `async_matrix` feature to your `Cargo.toml`:

```toml {3}
rmk = { version = "0.9", features = [
    "nrf52840_ble",
    "async_matrix",
] }
```

If you're using nRF chips or RP2040, you're all set! Your keyboard is now running in low-power mode. The `async_matrix` feature enables interrupt-based input detection, and puts your microcontroller into sleep mode when no keys are being pressed.

For STM32, there are some limitations about EXTI (see [here](https://docs.embassy.dev/embassy-stm32/git/stm32g474pc/exti/struct.ExtiInput.html)):

> EXTI is not built into Input itself because it needs to take ownership of the corresponding EXTI channel, which is a limited resource.
>
> Pins PA5, PB5, PC5… all use EXTI channel 5, so you can’t use EXTI on, say, PA5 and PC5 at the same time.

There are a few more things that you need to do:

1. Enable the `exti` feature for your `embassy-stm32` dependency in `Cargo.toml`
2. Ensure that your input pins don't share the same EXTI channel
3. For configuration:
   - If you're using `keyboard.toml`, you are all set. The `#[rmk_keyboard]` macro will automatically check your `Cargo.toml` and handle it for you.
   - If you're using Rust code, you'll need to use `ExtiInput` for your input pins and bind the EXTI interrupts of those pins. EXTI0 to EXTI4 each have their own interrupt, EXTI5 to EXTI9 share `EXTI9_5`, and EXTI10 to EXTI15 share `EXTI15_10`:

```rust
use embassy_stm32::bind_interrupts;
use embassy_stm32::exti::{ExtiInput, InterruptHandler};
use embassy_stm32::gpio::Pull;
use embassy_stm32::interrupt::typelevel::{EXTI9_5, EXTI15_10};

bind_interrupts!(struct Irqs {
    EXTI9_5 => InterruptHandler<EXTI9_5>;
    EXTI15_10 => InterruptHandler<EXTI15_10>;
});

    let pd9 = ExtiInput::new(p.PD9, p.EXTI9, Pull::Down, Irqs);
    let pd8 = ExtiInput::new(p.PD8, p.EXTI8, Pull::Down, Irqs);
    let pb13 = ExtiInput::new(p.PB13, p.EXTI13, Pull::Down, Irqs);
    let pb12 = ExtiInput::new(p.PB12, p.EXTI12, Pull::Down, Irqs);
    let row_pins = [pd9, pd8, pb13, pb12];

    let mut matrix = Matrix::<_, _, _, ROW, COL, true>::new(row_pins, col_pins, debouncer);
```

If your firmware already has a `bind_interrupts!` block (for example, for USB), add the EXTI lines to that block instead of declaring a second `Irqs`.

## BLE idle sleep

BLE builds also run an idle sleep manager. Set `split_central_sleep_timeout_seconds` in the `[rmk]` section of `keyboard.toml` (default `0`, disabled) to put the keyboard to sleep after that many seconds without key or pointing activity. Despite the name, it applies to every BLE keyboard, not only split centrals:

```toml
[rmk]
split_central_sleep_timeout_seconds = 600
```

When the keyboard falls asleep, RMK publishes a `SleepStateEvent`, holds battery level reports, and on a split central switches the peripheral links to slower connection parameters. Any key press wakes the keyboard up. The host's HID suspend and exit-suspend commands also put the keyboard to sleep and wake it.

Two related behaviors are always on:

- When BLE advertising times out without a connection (after 5 minutes), the keyboard sleeps immediately and waits for a key or pointing event before it advertises again.
- `NrfAdc` takes a `light_sleep` interval as its last argument. When the analog inputs have been idle for more than 1.2 seconds, the ADC polls at that interval instead of `polling_interval`. A joystick configured in `keyboard.toml` uses 350ms.

### Split central sleep using BLE Connection Subrating (tested on nrf52840 only)

To improve central power usage and decrease peripheral latency problems during wake-up you can activate the `subrating` feature in your `Cargo.toml`.
This enables BLE Connection Subrating which lets the central sleep for longer intervals, while allowing for a quick switch back to the fast connection intervals.

The maximum peripheral latency is ~450ms for the first keypress; the mean latency is roughly half (~225ms). The keyboard instantly switches to the active connection settings after that. Depending on your layout and habits this allows to use the sleep feature more aggressively. Try to reduce the timeout to get lower battery usage, e.g.:
```toml
[rmk]
split_central_sleep_timeout_seconds = 60
```

If the central is not connected to a host, the mean latency is further increased to ~472ms which reduces the power consumption of the central to ~22µA, nearly the same as the peripheral (~21µA).

## External VCC

Some boards, such as the nice!nano have an external 3.3V regulator that can be used to power the LEDs. If not used, the regulator can be disabled by pulling `P0_13` low to safe power.

In the case of the nice!nano, this can be done by adding the following line to `main.rs`

```rust
    // Disable external voltage regulator
    Output::new(peripherals.P0_13, Level::Low, OutputDrive::Standard).persist();
```

or the following snippet to your `keyboard.toml`:

```toml
[[output]]
pin = "P0_13"
initial_state_active = false
```
