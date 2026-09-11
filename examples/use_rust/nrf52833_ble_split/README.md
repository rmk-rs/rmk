# nRF52833 BLE split example

A two-piece split keyboard on two nRF52833 boards: the left half is the BLE
split central and also the half that talks to the host, the right half is split
peripheral id 0. The keymap is one 5x15 grid, columns 0..7 on the left and
columns 7..15 on the right.

Matrix pins, the flash map and the SDC configuration here match a real nRF52833
split board. Change `config_matrix_pins_nrf!` in `src/central.rs` and
`src/peripheral.rs` for your own wiring.

`keyboard.toml` here carries compile-time constants only, the `[rmk]` section
that `rmk-types`' build script reads. `split_central_sleep_timeout_seconds`
matters most: its default of 0 disables sleep management altogether, so the
central would never drop the split link into its low-power subrated state.

## Build

```shell
cargo build --release --bin central
cargo build --release --bin peripheral
```

## Flash map

`memory.x` assumes the [Adafruit nRF52
bootloader](https://github.com/adafruit/Adafruit_nRF52_Bootloader), which owns
flash from 0x74000 up. That leaves 460K for the app plus its storage partition,
and a vial + USB + BLE split central is already about 426K of it, so the app
region is nearly all of what remains. The commented-out block in `memory.x`
covers a board with no bootloader.

Both halves keep 4 x 4K pages of storage at 0x70000, declared in `memory.x` and
again in each binary's `StorageConfig`. Change both together.

If you need headroom, `.cargo/config.toml` carries a commented-out nightly
configuration that drops the central to about 364K. It is off by default because
this repo builds on stable.

## Flash

With a debug probe:

```shell
cargo run --release --bin central
cargo run --release --bin peripheral
```

With the Adafruit bootloader and no probe, build .uf2 images instead and drag
them onto each half's USB drive:

```shell
cargo install --force cargo-make
cargo make uf2 --release
```

RMK switches to USB mode whenever a cable is connected, so unplug after
flashing.

## Logs

`.cargo/config.toml` sets `DEFMT_LOG = "info"`. The split path logs one `debug!`
per key event on each half, which backs up RTT during fast bursts and distorts
any timing measured across the split link. Raise it only when you need those
lines, and remember that an environment change alone does not rebuild
dependencies:

```shell
cargo clean -p rmk --release
```

`run.sh` flashes one half over J-Link and then streams its defmt log. defmt-rtt
blocks when its buffer fills, so start the reader before resetting the board and
leave it attached.
