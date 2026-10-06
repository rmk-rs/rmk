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

## When stored data is erased

On boot, RMK might erase the stored data according to the change of the current firmware:

| What changed | What's erased |
| --- | --- |
| The RMK version, commit, or Cargo features | Everything, BLE pairings included |
| `macro_space_size`, `combo_max_length`, or `max_patterns_per_key` | Everything, BLE pairings included |
| Anything else, your keymap included | Nothing |

Editing the keymap in `keyboard.toml` or in Rust does **not** by itself replace what is stored: the stored keymap wins on boot, so edits made over Vial or Rynk survive a reflash. To hand the win back to the firmware, set `clear_layout = true` in the `[storage]` section of `keyboard.toml` (or the same field of `StorageConfig`), flash once, then set it back to `false`. It rewrites the keymap, encoders, behaviors, combos, forks, morses and macros from the firmware and keeps the BLE pairings; `clear_storage = true` erases everything, pairings included.
