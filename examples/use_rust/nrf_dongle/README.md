# Tri-mode dongle, end to end

A split BLE keyboard whose central is an ordinary `rynk` BLE keyboard, relayed
to the host by a USB dongle. Three firmwares, one per board:

| Crate | Board | Role |
| --- | --- | --- |
| [`dongle/`](dongle/) | nRF54LM20A | USB dongle: relays its bonded keyboard (HID + Rynk) |
| [`central/`](central/) | nRF52833 | Split central: the keyboard itself (`rynk` + `dongle` + `split`) |
| [`peripheral/`](peripheral/) | nRF54L15 | Split peripheral: the other half |

Each crate builds independently (`cargo build --release` inside it) and flashes
via J-Link (`./run.sh <elf>`).

## Bring-up order

1. Flash all three. The central is an Elytra left hand (5x7); its top-left key
   is the dongle key (`User8`). The peripheral is the nRF54L15 DK's four
   buttons as a 2x2 matrix, at row offset 5.
2. The split halves find each other on their own (central scans, peripheral
   advertises).
3. Press the dongle key on the central to switch to the dongle slot. An unbonded
   dongle is already scanning, so it picks up the seeking central and pairs.
   From then on everything reconnects automatically.
4. Typing from either half arrives on the host through the dongle; a Rynk host
   tool on the dongle's USB interface reaches the central's keymap
   transparently — the dongle passes those frames through untouched.

Holding the dongle key for 5s clears the central's dongle bond, which is how a
keyboard moves to another dongle. A dongle bonds one keyboard, and goes looking
for a new one whenever the bonded keyboard stops answering, so the replacement
just has to be seeking — no host involvement, and no reflashing.

The dongle relays whichever host protocol its keyboard speaks. This example is
Rynk; swapping `rynk` for `vial` on the central and adding `vial` to the dongle
crate yields a Vial dongle instead, whose USB side is the standard Vial HID
interface.
