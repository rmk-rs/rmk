# RP2040 split keyboard with mixed DFU backends

A split keyboard example for the RP2040 that combines two DFU approaches in a
single firmware:

- **central** uses `dfu_ext` — the DFU download slot lives on an external SPI
  NOR flash chip (a W25Q 8 MB), so the internal flash is freed up for a much
  larger ACTIVE partition.
- **peripheral** uses the plain internal DFU partition — firmware updates for
  the peripheral arrive over the split link from the central and are written
  into the internal DFU slot.

Because the two halves use different flash layouts, they are built from the
same crate but with **two different memory.x files** (`memory-central.x` and
`memory-peripheral.x`), one linker script per binary. The build script picks
the right one automatically based on `CARGO_BIN_NAME` (e.g. `--bin central`
or `--bin peripheral`).

## Wiring

Both halves talk over UART0 (TX `PIN_0`, RX `PIN_1`).

| Role       | Matrix input          | Matrix output | Notes                                  |
|------------|-----------------------|---------------|----------------------------------------|
| central    | `PIN_6`, `PIN_7`      | `PIN_10`, `PIN_11` | SPI0 DFU flash: SCK `PIN_18`, MOSI `PIN_19`, MISO `PIN_16`, CS `PIN_17`; DFU LED `PIN_25` |
| peripheral | `PIN_8`, `PIN_9`      | `PIN_12`      | DFU LED `PIN_25`                       |

Adjust `config_matrix_pins_rp!` in `src/central.rs` and `src/peripheral.rs`
to match your board.

## Building

The two halves need **different bootloaders**:

- **central** must be flashed with an `rmk-boot` build that uses the `dfu_ext`
  feature, e.g. `rmk-boot-rp2040-2mb-dfu_ext.uf2`.
- **peripheral** must be flashed with the plain (non-`dfu_ext`) build, e.g.
  `rmk-boot-rp2040-2mb.uf2`.

Build the firmware with `cargo-make`:

```shell
cargo install cargo-make
cargo make uf2
```

This produces four artifacts in the example directory:

- `rmk-rp2040-dfu-split-dfu-ext-central.uf2` — for the central half
- `rmk-rp2040-dfu-split-dfu-ext-peripheral.uf2` — for the peripheral half
- plus the matching `.hex` files and the peripheral `.bin` (embedded into the
  central via `include_bytes!` so the central can push peripheral firmware
  updates over the split link).

To build a single half directly:

```shell
cargo build --bin peripheral --release
cargo build --bin central   --release
```

## Flashing

1. Flash the matching bootloader to each half first (see above).
2. Put each half into bootloader mode and drag the corresponding `.uf2` onto
   it.

If you use a debugging probe instead, flash `cargo run --release --bin
central|peripheral` per half.