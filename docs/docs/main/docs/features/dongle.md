# Dongle

A dongle is a small USB board that stays plugged into your computer and relays a
wireless keyboard's reports to it. The keyboard talks Bluetooth to the dongle,
the dongle talks USB to the host.

Use one when the host's own Bluetooth isn't an option: a machine with no
Bluetooth at all, a BIOS or bootloader that can't see BLE keyboards, a locked-down
work computer, or a desk where BLE reconnection is slow enough to be annoying.
The keyboard stays wireless either way.

## How it works

The keyboard keeps a **bond slot of its own for the dongle**, separate from the
BLE host profiles. Profile cycling never lands on it, so switching between your
laptop and your phone doesn't disturb the dongle link, and pairing a new host
doesn't cost you the dongle.

The dongle bonds exactly one keyboard. When the bonded keyboard stops answering
it goes looking for a new one, so replacing a keyboard needs no host involvement
and no reflashing.

The dongle also relays the **host configurator protocol**, so [Vial](./vial_support)
and [Rynk](./rynk) work through the dongle exactly as they do over a direct
connection — the dongle passes those frames through untouched.

## Hardware

A dongle build needs a chip with both a BLE controller and a USB device
peripheral. RMK's example targets nRF; other BLE chips with USB use the same
API, but are untested.

The keyboard side works on any BLE keyboard, unibody or split. On a split, only
the central talks to the dongle.

## Building the firmware

Both sides enable the `dongle` Cargo feature — it means "this build is part of a
dongle setup", and what it turns on depends on which side you're building.

**The keyboard** gets the extra bond slot and the `SwitchToDongle` key:

```toml title="keyboard/Cargo.toml"
rmk = { path = "...", default-features = false, features = [
    "defmt",
    "storage",
    "vial",       # or "rynk"
    "dongle",
    "split",      # only if the keyboard is a split
    "nrf52840_ble",
] }
```

**The dongle** gets `rmk::dongle`:

```toml title="dongle/Cargo.toml"
rmk = { path = "...", default-features = false, features = [
    "defmt",
    "storage",
    "dongle",
    "nrf52840_ble",
] }
```

:::warning
The two builds must agree on the host protocol. The dongle relays Rynk by
default; to relay Vial, add the `vial` feature to **both** the keyboard and the
dongle crate. A mismatch leaves the configurator unable to reach the keyboard,
even though typing still works.
:::

The dongle binary is short — a USB transport, the dongle itself, and storage for
the bond, wired together by a shared router:

```rust title="dongle/src/main.rs"
use rmk::dongle::{Dongle, DongleRouter};

// Shared by the two tasks that relay: the dongle's BLE link and the USB host sessions.
let router = DongleRouter::new();
let mut dongle = Dongle::new(sdc, ble_addr(), &router);
let mut usb_transport = UsbTransport::new(driver, device_config).with_dongle_router(&router);

run_all!(usb_transport, dongle, storage).await;
```

The dongle has no matrix, no keymap, and no storage of your layout — it only
stores which keyboard it is bonded to.

## Pairing

1. Flash both boards and plug the dongle in. A dongle with no bonded keyboard
   opens a pairing window and repeats it until a keyboard shows up.
2. Press the `SwitchToDongle` key on the keyboard. The keyboard switches to its
   dongle slot and starts seeking; the scanning dongle picks it up and pairs.
3. From then on both sides reconnect on their own. A bonded dongle opens the
   pairing window exactly once, at power-on, and afterwards only reconnects its
   bonded keyboard.

`SwitchToDongle` is `User(N+5)`, where N is the number of BLE profiles — see
[Wireless](./wireless) for the full list of profile keycodes. It exists only in
builds with the `dongle` feature.

To move the keyboard to a different dongle, **hold `SwitchToDongle` for 5
seconds**. That clears the keyboard's dongle bond and sends it seeking again.

The pairing window length is configurable:

```toml title="keyboard.toml"
[rmk]
# Seconds per pairing window, default 30. Dongle builds only.
dongle_pairing_window_secs = 30
```

## Example

[`examples/use_rust/nrf_dongle`](https://github.com/rmk-rs/rmk/tree/main/examples/use_rust/nrf_dongle)
is a complete three-board setup: a split BLE keyboard (central + peripheral)
relayed to the host by a USB dongle, with Rynk reaching the central's keymap
through the dongle. Its README walks through the bring-up order.
