# Rynk

Rynk is RMK's own protocol for configuring a keyboard while it's running — change
your keymap, layers, combos, tap-dance, macros and more without reflashing the
firmware. It plays the same role as [Vial](./vial_support), but it is built
natively for RMK and understands every RMK feature.

::: warning Experimental
Rynk is experimental. The wire protocol, the host crates, and the `[host]`
options on this page can change in any release, and there is no ready-made
desktop app yet — only the reference web page described below. Firmware built
against one RMK version may not talk to host tools from another.

[Vial](./vial_support) is the default and the supported choice for a keyboard you
rely on. Reach for Rynk if you want RMK's full feature set and don't mind
tracking the changes yourself.
:::

## What you can configure

Through Rynk, a host tool can read and change:

- Keymap entries per layer, plus encoders and the default layer
- Combos, forks, tap-dance / morse keys, and macros
- Tap-hold and other behavior settings

It can also watch live status — the current layer, a key tester (matrix state),
typing speed (WPM), the caps-lock/num-lock indicators, battery level, and, on
wireless boards, the connection and BLE profile (including switching or clearing
a profile). Finally it can reboot the keyboard, enter the bootloader, and reset
stored settings.

## Enable Rynk

Rynk and Vial are mutually exclusive — pick one. For `keyboard.toml` projects,
select Rynk in `[host]`:

```toml title="keyboard.toml"
[host]
vial_enabled = false
rynk_enabled = true
```

Then disable RMK's default features, which include Vial, and enable `rynk`
explicitly. Add the other features your keyboard needs:

```toml title="Cargo.toml"
rmk = { version = "0.9", default-features = false, features = [
    "defmt",
    "storage",
    "rynk",
    "watchdog",
    "rp2040",
] }
```

::: warning
The `[host]` setting and the Cargo feature must agree. For `keyboard.toml`
projects RMK checks this when it builds: if the `rynk` feature is on,
`rynk_enabled` must be `true` (and `vial_enabled` `false`), otherwise the build
stops with an error. To use Vial instead, flip both settings and swap the `rynk`
feature for `vial`.
:::

::: warning
Keep the `storage` feature enabled (it's on by default) so your changes survive
a reboot. Without it, anything you set over Rynk is lost when the keyboard
restarts. See [Storage](./storage).
:::

## Connecting to your keyboard

Rynk works over the connection you already use:

- **USB** — plug the keyboard in. RMK exposes Rynk on its own vendor interface
  (class `0xFF`, subclass and protocol `0x52`), so host tools find RMK keyboards
  by that interface instead of asking you to pick a port.
- **Bluetooth** — native tools connect to an already-connected keyboard (the OS
  must have it bonded and currently connected).
- **Browser** — Chromium browsers (Chrome or Edge) reach USB through WebUSB and
  Bluetooth through WebHID. Firefox and Safari are not supported.

Rynk tooling is still young. Today you have two options:

- **A browser demo** — the `rynk-wasm` package ships a reference web page
  (`index.html`) you build and serve locally; follow the steps in its README.
- **Rust libraries** — the `rynk`, `rynk-usb`, and `rynk-ble` crates let you
  build a native tool (see [For tool authors](#for-tool-authors) below). The
  browser build is a separate package, `rynk-wasm`.

A ready-made desktop app is not available yet.

## Locking dangerous operations

By default Rynk locks the operations that could plant a persistent implant or
leak what you type, so a background process — or a web page you granted access
to once — can't quietly reflash your keyboard or read your keystrokes. To use a
locked operation you prove you're physically present by holding a key combo.

There are three tiers:

- **Open** — everything you normally reach for: read the keymap, change keys,
  layers, combos, macros, switch BLE profiles, reboot. These stay available so
  on-the-fly configuration is friction-free.
- **Locked** — the dangerous ones, always gated: entering the bootloader,
  resetting stored settings and bonds, reading the live key matrix (a keylogger
  if left open), and clearing a BLE bond. A host tool gets a "locked" error
  until you unlock.
- **Config writes** — open by default, because on-the-fly configuration is the
  point. Set `write_requires_unlock = true` to move every write (keymap, macros,
  …) into the locked tier as well.

### Unlocking

Set the keys the owner holds to unlock in `[host].unlock_keys` (up to four):

```toml title="keyboard.toml"
[host]
rynk_enabled = true
unlock_keys = [[0, 0], [3, 12]]  # hold (row 0, col 0) and (row 3, col 12)
```

When you trigger a locked action, the host tool reads the challenge and shows
"hold these keys". Hold them together; the tool polls briefly until the device
unlocks, and the session stays unlocked until it ends. Unplugging, a Bluetooth
disconnect, or an explicit lock re-locks the device.

If you leave `unlock_keys` unset, the locked operations can never be unlocked —
a safe default, but it means a fresh config can't enter the bootloader over Rynk
or use the matrix tester until you add the keys.

For local development you can bypass the gate entirely:

```toml title="keyboard.toml"
[host]
insecure = true  # start and stay unlocked — don't ship this
```

::: warning Macros and combos are readable
Config _reads_ stay open, so anything that can reach the protocol can read your
stored macros. Don't put passwords or other secrets in a macro.
:::

::: tip Two different `unlock_keys`
`[host].unlock_keys` guards this Rynk lock gate. A separate `[dfu].unlock_keys`
guards the firmware _download_ once the device is already in the bootloader (the
`dfu_lock` feature). They're independent — set either, both, or neither. A setup
that cares most about firmware replacement wants both: the Rynk gate blocks the
remote route into the bootloader, and `dfu_lock` covers a physically-present
attacker who reaches it another way.
:::

## Advanced tuning

Rynk's firmware buffers size themselves automatically and rarely need touching.
There are several parameters in `keyboard.toml`'s [`[rmk]`](../configuration/rmk_config#rynk-protocol-configuration) section that you can adjust:

- `rynk_buffer_size`: the exact RAM of each Rynk frame buffer, holding one COBS-encoded wire frame. The payload a message can carry is what remains after framing overhead and the 3-byte header. A larger buffer moves more per round-trip at the cost of RAM.
- `protocol_macro_chunk_size`: macro chunk size.

Bulk transfers (faster multi-entry reads and writes) are always available with
`rynk` — there's no separate feature to enable. Their batch size is derived from
`rynk_buffer_size`, so a larger buffer also moves more per bulk round-trip. The
keyboard advertises bulk support in its capabilities, so host tools pick it up
automatically and the same tool works on boards of any buffer size.

## For tool authors

If you're building your own host tool, RMK ships ready-made client crates so you
don't have to implement the protocol yourself:

- `rynk` — the core typed client; talks to a keyboard over any byte link.
- `rynk-usb` — USB discovery and connection.
- `rynk-ble` — native Bluetooth discovery and connection.
- `rynk-wasm` — the browser build, driven from JavaScript over Web Serial / WebHID.

While Rynk is experimental these crates are not published to crates.io. Depend on
them by path or git from the [`rynk/` workspace](https://github.com/rmk-rs/rmk/tree/main/rynk),
and pin the same revision as the firmware you're talking to.

A minimal native USB tool looks like this (add `embassy-futures` to your
`Cargo.toml` for `select`):

```rust
use embassy_futures::select::{Either, select};
use rynk::RynkDevice;
use rynk_usb::UsbDevice;

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    // Find a connected Rynk keyboard and open it (the handshake runs in `connect`).
    let device = UsbDevice::discover()
        .await?
        .into_iter()
        .next()
        .ok_or("no Rynk keyboard found")?;

    let (client, mut driver) = device.connect().await?;
    let work = async {
        let caps = client.get_capabilities().await?;
        println!("{}x{}x{} keymap", caps.num_layers, caps.num_rows, caps.num_cols);

        let key = client.get_key(0, 0, 0).await?;
        println!("L0(0,0) = {key:?}");
        Ok::<_, rynk::RynkHostError>(())
    };

    match select(driver.run(&client), work).await {
        Either::First(err) => return Err(err.into()),
        Either::Second(result) => result?,
    }
    Ok(())
}
```

Live status arrives as "topics" you poll for in work running alongside the
driver:

```rust
loop {
    let topic = client.next_topic().await;
    println!("{topic:?}");
}
```

Topics are best-effort. If you miss one, just read the current value again with
the matching `get_*` method.

Firmware and host share the same message types (via `rmk-types`), so the two ends
can never disagree about a message's format. The client crates handle all the
encoding for you, so you never touch the raw bytes on the wire.
