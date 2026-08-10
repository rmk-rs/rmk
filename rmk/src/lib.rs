#![doc = include_str!("../README.md")]
//! ## Feature flags
#![doc = document_features::document_features!()]
// Add docs.rs logo
#![doc(
    html_logo_url = "https://github.com/rmk-rs/rmk/blob/dad1f922f471127f5449262c4cb4a922e351bf43/docs/images/rmk_logo.svg?raw=true"
)]
// Make compiler and rust analyzer happy
#![allow(dead_code)]
#![allow(non_snake_case, non_upper_case_globals)]
#![allow(async_fn_in_trait)]
// Lints below fire inside `#[gatt_service]`/`#[gatt_server]` attribute-macro
// expansions from trouble-host; we can't annotate the generated code, so
// suppress them crate-wide rather than littering individual BLE structs.
#![allow(clippy::needless_borrows_for_generic_args)]
#![allow(clippy::needless_update)]
// Enable std for espidf and test. The `std` feature is test-only, and
// `test_support` needs std in the build the integration tests link against —
// that one is not `cfg(test)`, since `tests/` is a separate target.
#![cfg_attr(not(any(test, feature = "std")), no_std)]

// Mutual exclusivity guard
#[cfg(all(feature = "rynk", feature = "vial"))]
compile_error!("features `rynk` and `vial` are mutually exclusive");

// `host` needs a concrete configurator protocol to expose `HostService`.
#[cfg(all(feature = "host", not(any(feature = "rynk", feature = "vial"))))]
compile_error!("feature `host` requires enabling either `rynk` or `vial`");

#[cfg(all(feature = "dongle", not(feature = "_ble")))]
compile_error!("feature `dongle` requires a BLE chip feature (e.g. `nrf52840_ble`)");

#[cfg(all(feature = "dongle", feature = "_no_usb"))]
compile_error!("feature `dongle` requires a chip with USB");

#[cfg(all(feature = "usb_log", feature = "_usb_high_speed"))]
compile_error!(
    "`usb_log` is not supported on high-speed USB chips yet: embassy-usb-logger \
     only handles 64-byte packets, which high-speed bulk endpoints can't use. \
     Use `defmt` logging on these chips."
);

#[cfg(all(feature = "dfu_split", feature = "_ble"))]
compile_error!(
    "`dfu_split` is not supported on BLE keyboards yet: the DFU passthrough only \
     runs over the wired split transport. Disable `dfu_split` on BLE builds."
);

// Re-export self as ::rmk for macro-generated code to work both inside and outside the crate
extern crate self as rmk;

include!(concat!(env!("OUT_DIR"), "/constants.rs"));

// TODO: re-export to `constants`?
pub(crate) use rmk_types::constants::*;

// This mod MUST go first, so that the others see its macros.
pub(crate) mod fmt;

pub use embassy_futures;
#[cfg(not(any(cortex_m)))]
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex as RawMutex;
#[cfg(cortex_m)]
pub use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex as RawMutex;
pub use embassy_time;
pub use futures;
pub use heapless;
// Re-exported here so generated code from `#[rmk_keyboard]` and pure-Rust API
// users can spawn the auto mouse layer task without depending on internal
// module paths.
pub use keyboard::auto_mouse_layer::AutoMouseLayerRunner;
use keymap::KeyMap;
pub use keymap::KeymapData;
pub use rmk_macro as macros;
pub use rmk_types as types;
#[cfg(all(feature = "storage", feature = "host"))]
use rmk_types::action::EncoderAction;
#[cfg(feature = "_ble")]
pub use trouble_host::prelude::*;
#[cfg(feature = "storage")]
use {embedded_storage_async::nor_flash::NorFlash as AsyncNorFlash, storage::Storage};

use crate::config::PositionalConfig;

#[cfg(feature = "_ble")]
pub mod ble;
pub mod boot;
pub mod channel;
pub mod config;
pub mod core_traits;
#[cfg(feature = "dfu_split")]
pub mod crc32;
pub mod debounce;
#[cfg(feature = "dfu")]
pub mod dfu;
#[cfg(feature = "display")]
pub mod display;
#[cfg(feature = "dongle")]
pub mod dongle;
pub mod driver;
pub mod event;
pub mod helper_macro;
pub mod hid;
#[cfg(feature = "host")]
pub mod host;
pub mod input_device;
pub mod keyboard;
pub mod keyboard_macros;
pub mod keymap;
pub mod layout_macro;
pub mod light;
pub mod matrix;
pub mod processor;
#[cfg(feature = "split")]
pub mod split;
pub mod state;
#[cfg(feature = "storage")]
pub mod storage;
#[cfg(not(feature = "_no_usb"))]
pub mod usb;
#[cfg(feature = "watchdog")]
pub mod watchdog;

// Test-only helpers for `#[cfg(test)]` modules under `src/` and for the
// simulator harness in `tests/integration/simulator`; never part of a firmware
// build.
#[cfg(any(test, feature = "std"))]
#[doc(hidden)]
pub mod test_support;

pub async fn initialize_keymap<
    'a,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize,
>(
    data: &'a mut KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>,
    behavior_config: &'a mut config::BehaviorConfig,
    positional_config: &'a PositionalConfig<ROW, COL>,
) -> KeyMap<'a> {
    KeyMap::new(data, behavior_config, positional_config).await
}

#[cfg(feature = "storage")]
pub async fn initialize_keymap_and_storage<
    'a,
    F: AsyncNorFlash,
    const ROW: usize,
    const COL: usize,
    const NUM_LAYER: usize,
    const NUM_ENCODER: usize,
>(
    data: &'a mut KeymapData<ROW, COL, NUM_LAYER, NUM_ENCODER>,
    flash: F,
    storage_config: &config::StorageConfig,
    behavior_config: &'a mut config::BehaviorConfig,
    positional_config: &'a PositionalConfig<ROW, COL>,
) -> (KeyMap<'a>, Storage<F, ROW, COL, NUM_LAYER, NUM_ENCODER>) {
    #[cfg(feature = "host")]
    {
        let mut storage = {
            let encoder_opt: Option<&mut [[EncoderAction; NUM_ENCODER]; NUM_LAYER]> = if NUM_ENCODER > 0 {
                Some(&mut data.encoder_map)
            } else {
                None
            };
            Storage::new(flash, &data.keymap, &encoder_opt, storage_config, behavior_config).await
        };

        let keymap = KeyMap::new_from_storage(data, Some(&mut storage), behavior_config, positional_config).await;
        (keymap, storage)
    }

    #[cfg(not(feature = "host"))]
    {
        let storage = Storage::new(flash, storage_config, behavior_config).await;
        let keymap = KeyMap::new(data, behavior_config, positional_config).await;
        (keymap, storage)
    }
}
