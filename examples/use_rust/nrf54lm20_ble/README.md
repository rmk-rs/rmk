# nRF54LM20A BLE example

A `use_rust` keyboard on the nRF54LM20A test board.

## Pin map

| Net | Module pin | Used as |
| --- | --- | --- |
| `COL0`–`COL7` | `P0.01`, `P0.00`, `P3.04`, `P1.06`, `P0.08`, `P1.10`, `P1.22`, `P1.25` | Matrix outputs |
| `ROW0`–`ROW4` | `P1.01`, `P1.02`, `P1.00`, `P1.04`, `P3.12` | Matrix inputs, pull-down |
| `EC1_A` / `EC1_C` | `P1.23` / `P1.19` | Encoder 0, internal pull-ups |
| `CS`, `MOSI`, `SCK`, `MISO` | `P1.15`, `P1.16`, `P1.17`, `P1.18` | PMW3610 NCS, SDIO, SCLK, MOT |
| `RGB` / `RGB_EN` | `P0.04` / `P1.30` | `SPIM30` → 2x WS2812, rail gated |
| `CHRG` | `P2.02` | TP4057 charge state, low-active |
| `SCL` / `SDA` | `P2.00` / `P2.01` | Not wired up — see below |
| `BATTERY_ADC` | `P1.29` | Not wired up — see below |

The switch diodes point from the columns to the rows, so the columns are driven and the rows are
read: `Matrix<_, _, _, ROW, COL, true>`.

`ROW0` and `ROW1` land on `P1.01`/`P1.02`, the NFC antenna pins. `embassy-nrf` only exposes those as
GPIO under its `nfc-pins-as-gpio` feature, which this example enables; on nRF54L that just clears
`NFCT.PADCONFIG` at startup, so no UICR erase is needed.

Storage is backed by the internal flash via `nrf-mpsl`, so keymap/profile changes persist.

## RGB

Two WS2812s hang off `RGB` (`P0.04`), with the `RGB_PWR` rail gated by `RGB_EN` (`P1.30`). The usual
nRF WS2812 trick, a `SequencePwm` sequence, is unavailable here: `PWM20`–`PWM22` sit in the
peripheral power domain and cannot drive a `P0` pin, which belongs to the low-power domain.
`SPIM30` can, so [`src/rgb.rs`](src/rgb.rs) clocks the chain out of SPI instead — five 4 MHz bits
per WS2812 bit:

| WS2812 bit | SPI bits | High | Low | Datasheet window |
| --- | --- | --- | --- | --- |
| `0` | `11000` | 500 ns | 750 ns | T0H 400±150, T0L 850±150 |
| `1` | `11100` | 750 ns | 500 ns | T1H 800±150, T1L 450±150 |

The 1.25 µs bit period comes out exact, and five bits per bit makes one colour byte exactly five SPI
bytes, so a frame is 30 bytes. If the LEDs turn out to be SK6812 rather than WS2812B, shorten
`CODE_0`/`CODE_1`. They must be a 3.3 V part either way (`WS2812B-2020`, `SK6812MINI-E`) — the rail
is `VDD`, and a classic WS2812B wants ≥ 3.5 V.

`RgbService` renders at 30 fps and shows the link state on both LEDs:

| State | LEDs |
| --- | --- |
| Seeking a dongle | Blinking white |
| Advertising to a BLE host | Blinking blue |
| Connected, or USB/idle | Dark, and `RGB_EN` released |

Both advertising paths report `BleState::Advertising`; only the active slot tells them apart, so the
service compares the profile against `NUM_BLE_PROFILE` (the dongle slot). Going dark writes an
all-black frame *before* dropping `RGB_EN`: the gate only turns `Q2` off, which leaves `RGB_PWR`
floating rather than grounded, and a driven `DIN` can keep a WS2812 alive through its input
protection diode.

`RgbService` is a user processor, so `keyboard.toml` raises the `connection_status_change` subscriber
count — RMK's own budget only covers its built-in components.

## Trackball

The `TRACKBALL` and `H13` connectors carry six signals: `BAT`, `CS`, `MOSI`, `SCK`, `MISO`, `GND`. A
PMW3610 needs exactly four, so the schematic's SPI names map onto the sensor as `CS` → `NCS`,
`MOSI` → `SDIO` (bidirectional, bit-banged half-duplex), `SCK` → `SCLK`, `MISO` → `MOT`. The example
waits on `MOT` rather than polling, so an idle trackball costs nothing.

That `MOSI`/`MISO` split is inferred from the names, not from the daughterboard's schematic. If the
sensor never initialises (`read_reg` returns a wrong product id) or reports no movement, swap the
two: `Flex::new(p.P1_18)` for SDIO and `p.P1_16` for `MOT`.

## Running

```shell
cd examples/use_rust/nrf54lm20_ble
cargo run --release
```

If `probe-rs` fails to flash, or the first run after flashing hard-faults, run it again — the
nRF54LM20A support is still rough and a second or third attempt usually takes.

To produce a standalone image instead:

```shell
rust-objcopy -O ihex target/thumbv8m.main-none-eabihf/release/rmk-nrf54lm20 rmk-nrf54lm20.hex
```

It spans `0x0`–`0x67000` and never touches the storage area at `0x120000`, so reflashing keeps the
keymap and bonds.

When the board is attached to a USB host, RMK prefers USB until you switch the saved connection
mode. To test BLE first, power it without enumerating USB, or toggle to BLE once it is running.
