# Matrix

The keyboard matrix is the core hardware system responsible for scanning switches and detecting key presses. It serves as the bridge between the physical keyboard hardware and the firmware's key processing logic.

## Matrix Types in RMK

RMK provides four built-in matrix implementations to match different hardware designs:

### Normal Matrix

The standard approach. Keys are wired in a row-column grid, using diodes to prevent [ghosting](https://en.wikipedia.org/wiki/Key_rollover#Key_jamming_and_ghosting). RMK supports both col2row and row2col diode configurations to match your PCB design. You can set the diode direction in the [matrix configuration](../configuration/keyboard_matrix#matrix-configuration).

### Direct Pin Matrix

Each key connects directly to its own GPIO pin, eliminating the matrix grid and the need for diodes. All key states are read simultaneously without scanning. This method requires a high number of GPIO pins, so it's best for small keyboards and macropads. In the Rust API this is [`rmk::matrix::direct_pin::DirectPinMatrix`](https://github.com/rmk-rs/rmk/blob/main/rmk/src/matrix/direct_pin.rs); see the [`rp2040_direct_pin`](https://github.com/rmk-rs/rmk/blob/main/examples/use_rust/rp2040_direct_pin/src/main.rs) example.

### Bidirectional Matrix

The bidirectional matrix design uses dynamically switchable GPIO pins that can change between input and output modes during the scan cycle. Because the bidirectional matrix is more complicated than the normal matrix, only the [Rust API](https://github.com/rmk-rs/rmk/blob/main/rmk/src/matrix/bidirectional_matrix.rs) is provided at the moment.

### 74HC595 Shift Register Matrix

Columns are driven by one or more daisy-chained 74HC595 shift registers on an SPI bus (up to 32 columns), while rows stay on GPIO input pins. This saves MCU pins on wide keyboards. Only the Rust API is provided: [`rmk::matrix::hc595_matrix::Hc595Matrix`](https://github.com/rmk-rs/rmk/blob/main/rmk/src/matrix/hc595_matrix.rs) takes an `SpiDevice`, the latch (RCLK) output pin, the row input pins and a debouncer, and derives the chain length from the `COL` const generic:

```rust
use rmk::matrix::hc595_matrix::Hc595Matrix;

let mut matrix = Hc595Matrix::<_, _, _, _, ROW, COL>::new(spi_device, latch_pin, row_pins, debouncer).await;
```

## Async Matrix Feature

Async matrix is a power-saving feature that transforms how the matrix operates, dramatically reducing power consumption for wireless keyboards. This feature works out-of-the-box for nRF and RP2040. STM32 requires additional EXTI (external interrupt) configuration due to hardware limitations—see the [Low Power](./low_power) documentation for details.

To enable it, add the `async_matrix` feature in `Cargo.toml`:

```toml
rmk = { version = "0.9", features = ["async_matrix"] }
```

## Configuration

For detailed matrix configuration options, pin assignments, and platform-specific setup, see the [Matrix Configuration](../configuration/keyboard_matrix#matrix-configuration) documentation.

## Customization via Traits

RMK's matrix system is built on a trait-based architecture. Any matrix or debouncer that implements the corresponding trait can be seamlessly integrated into RMK, making both components highly extensible without touching core firmware code:

**`MatrixTrait`**: Defines the core scanning interface. Implement this trait to support external I/O expanders, non-standard electrical designs, or specialized scanning algorithms.

**`DebouncerTrait`**: Controls switch bounce filtering. RMK includes default and fast debouncing algorithms, and you can also implement custom debouncing logic optimized for your own use cases.

A matrix is an input device that publishes `KeyboardEvent`s, so `MatrixTrait` builds on the [`InputDevice`](./input_device) trait. The following is an example demonstrating how to use a customized matrix:

```rust
use rmk::event::KeyboardEvent;
use rmk::macros::input_device;
use rmk::matrix::MatrixTrait;
use rmk::run_all;

// `#[input_device]` generates the `InputDevice` and `Runnable` impls
#[input_device(publish = KeyboardEvent)]
struct YourOwnMatrix<const ROW: usize, const COL: usize> {}

impl<const ROW: usize, const COL: usize> YourOwnMatrix<ROW, COL> {
    // Required by `#[input_device(publish = KeyboardEvent)]`
    async fn read_keyboard_event(&mut self) -> KeyboardEvent {
        // Scan your hardware, debounce, and return
        // `KeyboardEvent::key(row, col, pressed)` for each key state change
    }
}

impl<const ROW: usize, const COL: usize> MatrixTrait<ROW, COL> for YourOwnMatrix<ROW, COL> {
    // With the `async_matrix` feature, implement `wait_for_key()` here
}

let mut my_matrix = YourOwnMatrix::<ROW, COL> {}; // Create the matrix struct

// .. Other initialization

// Run the main process
run_all!(my_matrix, keyboard, storage, usb_transport).await;
```

## See Also

- [How key matrices work](https://pcbheaven.com/wikipages/How_Key_Matrices_Works/)
