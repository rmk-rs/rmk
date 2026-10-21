# Host Configuration

The `[host]` section selects the firmware protocol used by host-side tools.
RMK currently supports two mutually exclusive protocols:

- `vial_enabled`: the Vial/VIA-compatible protocol for the Vial app. This is the default.
- `rynk_enabled`: RMK's native protocol for RMK-aware host tools. **Experimental** — see
  [Rynk](../features/rynk).

The `keyboard.toml` values must match the `rmk` Cargo features. If
`host.rynk_enabled = true`, enable the `rynk` Cargo feature. If
`host.vial_enabled = true`, enable the `vial` Cargo feature. Do not enable both.

## Configuration Example

```toml
[host]
# Enable Rynk, RMK's native host protocol.
rynk_enabled = true

# Disable Vial when using Rynk. Rynk and Vial are mutually exclusive.
vial_enabled = false

# Physical keys (row, col) held simultaneously to unlock dangerous operations
# (optional, up to 4). Shared by the Vial lock and the Rynk lock gate. See the
# "Locking dangerous operations" section of the Rynk feature page.
unlock_keys = [[0, 0], [0, 1]]  # Keys at (row=0,col=0) and (row=0,col=1)

# Start (and stay) unlocked, bypassing the unlock-key combo (default: false).
# A development escape hatch — don't ship it. Renamed from `vial_insecure`,
# which still parses.
insecure = false

# Rynk only: move config writes (SetKeyAction, SetMacro, …) into the locked
# tier so they also require unlock (default: false).
write_requires_unlock = false
```

## Common Setups

Use Vial with the `rmk` default Cargo features:

```toml title="keyboard.toml"
[host]
vial_enabled = true
rynk_enabled = false
unlock_keys = [[0, 0], [0, 1]]
```

Use Rynk instead:

```toml title="keyboard.toml"
[host]
vial_enabled = false
rynk_enabled = true
unlock_keys = [[0, 0], [0, 1]]
```

```toml title="Cargo.toml"
rmk = { version = "0.9", default-features = false, features = [
    "defmt",
    "storage",
    "rynk",
    "watchdog",
    "rp2040",
] }
```

Disable all host configurator support:

```toml title="keyboard.toml"
[host]
vial_enabled = false
rynk_enabled = false
```

```toml title="Cargo.toml"
rmk = { version = "0.9", default-features = false, features = [
    "defmt",
    "storage",
    "watchdog",
    "rp2040",
] }
```

## Independent split unlock

Configure a local unlock combination for each side. Add these tables alongside
that side's existing matrix configuration:

```toml
[split.central.host]
insecure = false
unlock_keys = [[0, 0], [0, 1]]

[split.peripheral.host]
insecure = false
unlock_keys = [[0, 0], [1, 0]]
```

Each coordinate is local to its side, before `row_offset` and `col_offset`.
Choose up to four real keys on that side. Empty combinations and invalid local
positions are rejected unless an empty combination is explicitly paired with
`insecure = true`. RMK converts positions reported to Vial into the shared
keyboard layout, so the configurator can highlight the correct keys.

The central's table overrides the top-level `[host]` unlock settings. Without
that table, the existing top-level settings and global coordinates still apply.
A peripheral never inherits the central's combination or unlocked state.

Adding `[split.peripheral.host]` enables an update-only USB Vial interface. It
requires the `split`, `vial`, and `host_lock` Cargo features, USB enabled, and a
supported bootloader feature (`adafruit_bl` for the Adafruit nRF52 bootloader,
or `rp2040` for BOOTSEL). It is separate from embassy-boot application DFU and
cannot be combined with the `dfu_nrf` or `dfu_rp` features. A peripheral without
this table keeps its existing USB behavior.

The peripheral scans its own keys and serves USB even when the central is
offline. Its USB product name ends in `(peripheral N)`, where `N` is its
zero-based split index. It exposes identity, unlock, and bootloader commands;
keymap editing is still handled by the central.

With `host_lock` enabled, Vial's `BootloaderJump` requires the current connection
to be unlocked. Both sides start locked, and unlocking one side does not unlock
the other. USB disconnect, bus reset, or deconfiguration ends the session and
locks it again; USB suspend preserves the session. `insecure = true` explicitly
keeps that side unlocked. Hardware bootloader recovery remains available.

An updater must keep the same HID connection while it reads `GetUnlockStatus`,
sends `UnlockStart`, polls `UnlockPoll` while the user holds the displayed keys,
and sends `BootloaderJump` after unlocking. A locked jump returns VIA's
`Unhandled` response (`0xFF`). The updater must then wait for the bootloader to
appear and confirm the application returns after flashing. The RMK GUI updater
needs matching support for this unlock sequence.

See the `examples/use_config/nrf52840_ble_split` example for both sides enabled.
