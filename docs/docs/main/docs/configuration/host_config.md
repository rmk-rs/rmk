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
