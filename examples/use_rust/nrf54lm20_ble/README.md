# ME54BS13 (nRF54LM20A) BLE example

This example targets the ME54BS13 test board — an `ME54BS13-nRF54LM20A` module driving a 5x8
diode matrix, a scroll-wheel encoder, a PMW3610 trackball and a 0.91" OLED.

## Pin map

| Net | Module pin | Wired up as |
| --- | --- | --- |
| `COL0`–`COL7` | `P0.01`, `P0.00`, `P3.04`, `P1.06`, `P0.08`, `P1.10`, `P1.22`, `P1.25` | Matrix outputs |
| `ROW0`–`ROW4` | `P1.01`, `P1.02`, `P1.00`, `P1.04`, `P3.12` | Matrix inputs, pull-down |
| `EC1_A` / `EC1_C` | `P1.23` / `P1.19` | Encoder 0, internal pull-ups |
| `CS`, `MOSI`, `SCK`, `MISO` | `P1.15`, `P1.16`, `P1.17`, `P1.18` | PMW3610 NCS, SDIO, SCLK, MOT |
| `RGB` / `RGB_EN` | `P0.04` / `P1.30` | `SPIM30` → 2x WS2812, rail gated |
| `CHRG` | `P2.02` | TP4057 charge state, low-active |
| `SCL` / `SDA` | `P2.00` / `P2.01` | Not wired up — see below |
| `BATTERY_ADC` | `P1.29` | Not wired up — see below |
| `STDBY` | `P2.03` | Unused |

The switch diodes point from the columns to the rows, so the columns are driven and the rows are
read: `Matrix<_, _, _, ROW, COL, true>`.

`ROW0` and `ROW1` land on `P1.01`/`P1.02`, which are the NFC antenna pins. `embassy-nrf` only
exposes them as GPIO under its `nfc-pins-as-gpio` feature, which this example enables; on nRF54L
that just clears `NFCT.PADCONFIG` at startup, so no UICR erase is needed.

Storage is backed by the internal flash via `nrf-mpsl`, so keymap/profile changes persist.

## GPIO power domains

Every peripheral pin choice on this chip is constrained by which power domain the port sits in. From
the datasheet's *GPIO — General purpose input/output*: `P0` belongs to the low-power domain, `P1`
and `P3` to the peripheral domain, `P2` to the MCU domain, and "GPIO pins can be used by peripherals
in the same power domain [...] Peripherals cannot mix pins from different ports." So:

| Port | Domain | Serial / PWM instances that can reach it |
| --- | --- | --- |
| `P0` | low-power | `SPIM30`, `TWIM30`, `UARTE30`, `SPIS30`, `TWIS30` |
| `P1`, `P3` | peripheral | `SPIM20`–`SPIM24`, `TWIM20`–`TWIM24`, `UARTE20`–`UARTE24`, `PWM20`–`PWM22`, `SAADC` |
| `P2` | MCU | `SPIM00`, `SPIS00`, `UARTE00`, `QSPI` — **no TWIM at all** |

`P2` also has no pin sense/detect and no GPIOTE, so it can only be polled, never awaited.

## RGB

Two WS2812s hang off `RGB` (`P0.04`), with the `RGB_PWR` rail gated by `RGB_EN` (`P1.30`) through
`Q3`/`Q2`. The usual nRF WS2812 trick — a `SequencePwm` sequence, as in
[elytra_firmware](https://github.com/HaoboGu/elytra_firmware)'s `Ws2812PwmDriver` — is unavailable
here: `PWM20`–`PWM22` are peripheral-domain and cannot drive a `P0` pin. `SPIM30` can, so
[`src/rgb.rs`](src/rgb.rs) clocks the chain out of SPI instead, five 4 MHz bits per WS2812 bit:

| WS2812 bit | SPI bits | High | Low | Datasheet window |
| --- | --- | --- | --- | --- |
| `0` | `11000` | 500 ns | 750 ns | T0H 400±150, T0L 850±150 |
| `1` | `11100` | 750 ns | 500 ns | T1H 800±150, T1L 450±150 |

The 1.25 µs bit period comes out exact, and five bits per bit makes one colour byte exactly five SPI
bytes. A whole frame is 30 bytes. If the LEDs turn out to be SK6812 rather than WS2812B, retune
`CODE_0`/`CODE_1` — SK6812 wants a shorter high time.

`RgbService` renders at 30 fps and drops `RGB_EN` whenever both LEDs are dark, so an idle keyboard
does not power the rail at all. LED 0 mirrors the battery (breathing green while charging, blinking
red below 10%), LED 1 mirrors the BLE link (breathing in the profile's colour while advertising, off
once connected). It is a user processor, so `keyboard.toml` raises the `battery_status` and
`connection_status_change` subscriber counts by one each — RMK's own subscriber budget only covers
its built-in components.

## Trackball

The `TRACKBALL` (14-pin) and `H13` (6-pin) connectors carry the same six signals: `BAT`, `CS`,
`MOSI`, `SCK`, `MISO`, `GND`. A PMW3610 needs exactly four, so the schematic's SPI names map onto
the sensor as `CS` → `NCS`, `MOSI` → `SDIO` (bidirectional, bit-banged half-duplex), `SCK` →
`SCLK` and `MISO` → `MOT`. The example runs in interrupt mode: `PointingDevice` waits on `MOT`
going low instead of polling, so an idle trackball costs nothing.

The `MOSI`/`MISO` split between `SDIO` and `MOT` is inferred from the names, not from the
daughterboard's own schematic. If the sensor never initialises (`read_reg` returns a wrong product
id) or reports no movement, swap the two: `Flex::new(p.P1_18)` for SDIO and `p.P1_16` for `MOT`.

## Not wired up

- **OLED.** Three separate problems on this revision. (1) `SCL`/`SDA` are on `P2.00`/`P2.01`, and
  the MCU domain that owns `P2` has no TWIM instance — see the power-domain table above — so there
  is no hardware I2C on those pins. `TWIM30`, the only I2C that reaches `P0`, is the same `SERIAL30`
  peripheral the RGB chain uses, so `P0` is not an escape either. (2) The header's `VCC` is on
  `+5V`, i.e. USB VBUS, so the display is dead on battery — which is when its BLE and battery
  readouts matter most. (3) Stock 0.91" I2C modules pull `SDA`/`SCL` up to their own `VCC` through
  4.7 kΩ, so with `VCC` at 5 V those lines idle ~1.4 V above the nRF54L's `VDD + 0.3 V` absolute
  maximum, whenever USB is plugged in and regardless of what the firmware does.

  All three are fixed in the planned revision below. Once the OLED has a real bus, `rmk`'s `ssd1306`
  feature plus `DisplayProcessor` is the whole driver side.
- **Battery percentage.** `BATTERY_ADC` sits on `P1.29`, which is `AIN3` on the nRF54LM20A. But
  `embassy-nrf`'s SAADC pin table for this chip (`chips/nrf54lm20_app.rs`) lists the nRF54L15 set —
  `P1.04`–`P1.07` and `P1.11`–`P1.14` — and omits LM20's `P1.00`, `P1.03` and `P1.29`–`P1.31`. The
  `saadc::Input` trait is sealed, so this needs `impl_saadc_input!(P1_29, 1, 29);` upstream. Charge
  state from `CHRG` works today, so the host and LED 0 see charging/discharging without a level.

  `BatteryProcessor`'s hardcoded 1137.8 counts/V needs no chip-specific path: nRF52840's gain 1/6
  over a 0.6 V reference and nRF54L's gain 2/8 over a 0.9 V reference are both 5/18, so the scale is
  the same. What does differ is headroom and acquisition time. At `Gain2_8` the datasheet gives
  `V(P) × GAIN/REFERENCE ≤ 1`, i.e. 3.6 V, but also `VRangeSingleEnded = ±0.5 × VREF/GAIN`, i.e.
  1.8 V; the `806k`/`2M` divider puts 2.99 V on the pin at 4.2 V, which is inside the first and
  outside the second, and is 91% of `VDD` besides ("AIN inputs cannot exceed VDD"). A 1/3 divider
  would be unambiguous. And per Table 56 a ~600 kΩ source needs `TACQ` = 40 µs, where embassy's
  `ChannelConfig::single_ended` defaults to `Time::_10US` (good for 100 kΩ).
- **`STDBY`.** `ChargingStateReader` takes a single pin, and `CHRG` alone distinguishes charging
  from not charging.

## Planned board revision

Six nets move, plus two do-not-populate footprints. Everything else — the matrix, the encoder, the
trackball signals, the RGB chain — stays exactly where it is.

| Net | This revision | Next revision | Module pad | Why |
| --- | --- | --- | --- | --- |
| OLED `SCL` | `P2.00` | `P1.13` | F3 | `P2` has no TWIM; `P1.13` is clock-capable, as Table 79 requires for `SCL` |
| OLED `SDA` | `P2.01` | `P1.14` | F2 | Same port as `SCL` and adjacent to it |
| OLED `VCC` | `+5V` | `VDD` | — | Display works on battery, and the module's pull-ups stop overdriving the pins |
| Trackball `VCC` | `BAT` | `VDD` | — | Removes the question of whether the daughterboard's IO exceeds `VDD + 0.3 V` |
| `CHRG` | `P2.02` | `P1.11` | F5 | `P1` has pin sense, so charger insertion can wake from System OFF |
| `STDBY` | `P2.03` | `P1.12` | F4 | Same |
| I2C pull-ups | — | 2x 4.7 kΩ to `VDD`, DNP | — | Populate only if the OLED module has none of its own |

`F5`/`F4`/`F3`/`F2` are four consecutive free module pads, so this routes cleanly. `P1.11` carries
`RADIO[5]`/`RADIO[6]` (DFE antenna switch) as opt-in alternates, which does not affect GPIO use;
`P1.12` has no alternates.

That frees all of `P2.00`–`P2.10`. Worth keeping free: `P2.00`–`P2.05` are the only pins on the chip
that can carry `QSPI` (`SCK`, `CSN`, `D0`–`D3`) or `HSSPI` (`SCK`, `CSN`, `MOSI`, `MISO`, `DCX`), so
they are the only route to an external QSPI flash or a fast SPI colour display. `P2.06`–`P2.10` are a
second `SPIM00` set and can be spent on output-only GPIO — nothing that needs sensing, since `P2` has
neither SENSE nor GPIOTE.

Deliberately left alone, each with a consequence:

- `BATTERY_ADC` stays on `P1.29`, so battery percentage stays blocked on the upstream
  `impl_saadc_input!(P1_29, 1, 29);` fix described above. Moving it to `P1.05` would have worked with
  stock `embassy-nrf`, since `{P1.04, P1.05, P1.06}` is the whole intersection of "really an AIN on
  LM20" and "declared in `embassy-nrf`".
- The divider stays `806k`/`2M`, so the ADC pin sees 2.99 V at 4.2 V. Whether that saturates depends
  on which of the datasheet's two conflicting range statements governs at `Gain2_8`; settle it by
  reading the raw value at a known battery voltage once the ADC is reachable at all.
- The RGB LEDs must still be a 3.3 V part (`WS2812B-2020`, `SK6812MINI-E`) — that is a BOM
  constraint, not a layout one, so it survives unchanged.
- The trackball header keeps the `MOSI`/`MISO` net names, so which line is `SDIO` and which is `MOT`
  remains an inference. See the Trackball section for how to swap them if the guess is wrong.

## Other things to check on the board

- `R92` is marked `2mΩ`. As drawn that shorts `BATTERY_ADC` to ground; the divider only makes sense
  as `2MΩ`, which is what `BatteryProcessor::new(2000, 2806)` assumes.
- The trackball header feeds the daughterboard from `BAT`, up to 4.2 V, while its four signals go
  straight to `P1.15`–`P1.18` at `VDD`. That is only safe if the daughterboard references its IO to
  its own 3.3 V regulator; otherwise those pins sit above `VDD + 0.3 V`. Feeding the header from
  `VDD` avoids the question.
- `RGB_PWR` is `VDD` (3.3 V) through `Q2`, so the LEDs must be a 3.3 V-rated part. A classic WS2812B
  wants ≥ 3.5 V; `WS2812B-2020` or `SK6812MINI-E` are the parts that work here.
- `CHRG`/`STDBY` are on `P2`, which has no pin sense or detect, so charger insertion cannot wake the
  chip from System OFF — it is polled only. Fine as long as nothing is meant to wake on it.

## Physical layout

rynk serves the physical layout over `GetLayout` so hosts can render the keyboard. Even a
`use_rust` keyboard keeps that layout in `keyboard.toml`, found through `KEYBOARD_TOML_PATH` in
`.cargo/config.toml`: RMK bakes the `[layout]` section into the firmware and `RmkConfig::default()`
picks it up, so `main.rs` wires up nothing. The same file also feeds the `[rmk]` / `[event]` build
constants — everything else stays in Rust. The `[input_device].encoder` entry there is inert for a
`use_rust` board; it only tells the layout builder that the `(e,0)` token in `map` is backed by
real hardware.

## Running

1. Enter the example directory:

   ```shell
   cd examples/use_rust/nrf54lm20_ble
   ```

2. Build, flash, and run it:

   ```shell
   cargo run --release
   ```

If `probe-rs` fails to flash because of the current nRF54LM20A bug, run the same command again.
It can take a second or third attempt.

When the board is attached to a USB host, RMK will prefer USB mode by default until you switch the
saved connection mode. If you want to test BLE first, power it without enumerating USB or toggle to
BLE mode from RMK once the board is running.
