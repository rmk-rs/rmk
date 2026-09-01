# Storage

RMK's storage system provides persistent flash memory for storing data like keyboard configurations and BLE bonding information.

## Storage Feature

RMK's storage system is enabled by the `storage` feature, which is part of the default feature set. Enabling BLE automatically pulls in `storage`, since BLE bonding data must be persisted to non-volatile storage. The host configurator protocols (`rynk` and `vial`) rely on `storage` to persist keymap edits across reboots but do not enable it themselves, so keep it enabled when you use them.

## Storage Configuration

By default, RMK saves data to your microcontroller's internal flash memory.

- For users configuring with `keyboard.toml`, the default storage space details are located in the `rmk-config/src/default_config` folder. If your microcontroller's configuration isn't found there, RMK uses the **last `num_sectors` flash sectors** of your microcontroller's internal flash memory. `num_sectors` defaults to 8 when a `[dfu]` section is present, either in your `keyboard.toml` or in the chip default (nRF52840, nice!nano, RP2040 and Pico W ship one), otherwise 2. On nRF BLE builds without DFU, the default start address `0` means `0x60000` instead of the end of flash. See [Storage configuration](../configuration/storage) for all `[storage]` fields.

- For Rust API users, create a `StorageConfig` struct and pass it to `initialize_keymap_and_storage`, which sets up the storage from your flash peripheral. Besides `start_addr` and `num_sectors`, `StorageConfig` carries `clear_storage` and `clear_layout`, which erase everything or only the layout at boot:

```rust
use rmk::config::{BehaviorConfig, PositionalConfig, StorageConfig};
use rmk::{KeymapData, initialize_keymap_and_storage};

let storage_config = StorageConfig::default();
let mut behavior_config = BehaviorConfig::default();
let per_key_config = PositionalConfig::default();
let mut keymap_data = KeymapData::new(keymap::get_default_keymap());
let (keymap, mut storage) = initialize_keymap_and_storage(
    &mut keymap_data,
    flash,
    &storage_config,
    &mut behavior_config,
    &per_key_config,
)
.await;

// `storage` is a runnable — pass it to `run_all!` with everything else
run_all!(matrix, storage, usb_transport, keyboard).await;
```

::: warning
Ensure you allocate sufficient storage space for your keymap and bonding information. 32 KiB is generally adequate for most keyboards.
:::

## Storage Is Cleared When RMK Is Rebuilt

The firmware embeds a build hash computed by the `rmk` crate's build script from the git commit and build time. The hash is written to storage when storage is first initialized, and checked on every boot: if the stored hash doesn't match the running firmware's hash, RMK erases the storage and re-initializes it from the firmware's defaults.

The hash only changes when the `rmk` build script re-runs: after `cargo clean`, or when you change the `rmk` version, its Cargo features, or the build profile. Rebuilding your own crate — for example after editing the keymap or `keyboard.toml` — reuses the same hash, so the stored keymap, keymap edits made via Vial/Rynk and BLE bonds survive that flash, and your keymap edits in source do **not** replace the stored keymap. To force a reset in that case, set `clear_layout = true` (keymap only) or `clear_storage = true` (everything, including BLE bonds) in the `[storage]` section of `keyboard.toml`, or the same fields of `StorageConfig`, flash once, then set them back to `false`.

When the hash does change, **all stored data is cleared**, including BLE bonding information, so you'll need to re-pair BLE hosts.
