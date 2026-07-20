use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use rmk_config::DebouncerType;
use rmk_config::resolved::hardware::{
    BoardConfig, ChipSeries, KeyInfo, MatrixConfig, MatrixType, UniBodyConfig,
};
use rmk_config::resolved::{Behavior, Hardware, Host, Identity, Keymap, Layout};

use super::behavior::expand_behavior_config;
use super::chip::bind_interrupt::expand_bind_interrupt;
use super::chip::ble::expand_ble_config;
use super::chip::chip_init::expand_chip_init;
use super::chip::comm::expand_usb_init;
use super::chip::flash::expand_flash_init;
use super::chip::gpio::expand_output_config;
use super::display::expand_display_config;
use super::entry::expand_rmk_entry;
use super::feature::{get_rmk_features, is_feature_enabled};
use super::import::expand_custom_imports;
use super::input_device::expand_input_device_config;
use super::keyboard_config::{
    expand_keyboard_info, expand_lock_config, expand_vial_config, read_keyboard_toml_config,
};
use super::keymap::expand_default_keymap;
use super::lighting::{expand_lighting_topology, expand_physical_layout};
use super::matrix::{expand_bootmagic_check, expand_matrix_config};
use super::registered_processor::expand_registered_processor_init;
use super::split::central::expand_split_central_config;
use super::watchdog::expand_watchdog_init;

/// Parse keyboard mod and generate a valid RMK main function with all needed code
pub(crate) fn parse_keyboard_mod(item_mod: syn::ItemMod) -> TokenStream2 {
    // Reject invalid `#[Overwritten(...)]` attributes up front instead of
    // silently falling back to the generated defaults (#966)
    if let Some(errors) = crate::codegen::override_helper::validate_overwritten_attrs(&item_mod) {
        return errors;
    }

    let rmk_features = get_rmk_features();

    let keyboard_config = read_keyboard_toml_config();

    // Resolve types from keyboard.toml
    let identity = keyboard_config
        .identity()
        .expect("failed to resolve identity config");
    let host = keyboard_config.host();
    let hardware = keyboard_config
        .hardware()
        .expect("failed to resolve hardware config");
    let behavior = keyboard_config
        .behavior()
        .expect("failed to resolve behavior config");
    let keymap = keyboard_config
        .keymap()
        .expect("failed to resolve keymap config");
    let layout = keyboard_config
        .layout()
        .expect("failed to resolve layout config");
    let lighting = keyboard_config
        .lighting(&layout, &keymap)
        .expect("failed to resolve lighting config");

    validate_feature_config_parity(
        &rmk_features,
        hardware.storage.is_some(),
        host.vial_enabled,
        host.rynk_enabled,
        lighting.is_some(),
    )
    .unwrap_or_else(|err| panic!("{err}"));

    // Generate imports and statics
    let imports_and_statics = expand_imports_and_constants(
        &identity,
        &host,
        &hardware,
        &behavior,
        &keymap,
        &layout,
        lighting.as_ref(),
    );

    // Generate main function body
    let main_function = expand_main(
        &host,
        &hardware,
        &behavior,
        &keymap,
        &layout,
        item_mod,
        &rmk_features,
    );

    quote! {
        #imports_and_statics

        #main_function
    }
}

fn validate_feature_config_parity(
    rmk_features: &Option<Vec<String>>,
    storage_in_config: bool,
    vial_in_config: bool,
    rynk_in_config: bool,
    lighting_in_config: bool,
) -> Result<(), String> {
    // A feature enabled in keyboard.toml must have its rmk Cargo feature enabled, and vice versa.
    // (Cargo feature, keyboard.toml field, enabled in keyboard.toml?)
    for (feature, config_field, in_config) in [
        ("storage", "storage.enabled", storage_in_config),
        ("vial", "host.vial_enabled", vial_in_config),
        ("rynk", "host.rynk_enabled", rynk_in_config),
    ] {
        if in_config == is_feature_enabled(rmk_features, feature) {
            continue;
        }
        return Err(if in_config {
            format!(
                "If the \"{feature}\" Cargo feature is disabled, `{config_field}` must be set to false in keyboard.toml."
            )
        } else {
            format!(
                "`{config_field} = false` in keyboard.toml requires disabling the \"{feature}\" Cargo feature for rmk in Cargo.toml (for example with `default-features = false` and explicitly re-enabling the features you need)."
            )
        });
    }

    if vial_in_config && rynk_in_config {
        return Err(
            "`host.vial_enabled` and `host.rynk_enabled` are mutually exclusive — set exactly one to true (the underlying Cargo features for rmk also conflict).".to_string(),
        );
    }

    // Unlike the host/storage integrations, lighting also has a public Rust
    // construction path, so enabling the feature without TOML is valid. A
    // TOML section does require the feature because codegen emits lighting
    // runtime types.
    if lighting_in_config && !is_feature_enabled(rmk_features, "lighting") {
        return Err(
            "A `[lighting]` section in keyboard.toml requires enabling the \"lighting\" Cargo feature for rmk."
                .to_string(),
        );
    }

    Ok(())
}

pub(crate) fn expand_imports_and_constants(
    identity: &Identity,
    host: &Host,
    hardware: &Hardware,
    behavior: &Behavior,
    keymap: &Keymap,
    layout: &Layout,
    lighting: Option<&rmk_config::resolved::Lighting>,
) -> TokenStream2 {
    // Generate keyboard info and number of rows/cols/layers
    let keyboard_info_static_var = expand_keyboard_info(identity, keymap);
    // Generate default keymap
    let default_keymap = expand_default_keymap(keymap, behavior);
    // Generate vial config
    let vial_static_var = expand_vial_config(host);
    // Generate rynk lock-gate config
    let lock_static_var = expand_lock_config(host);
    let physical_layout = expand_physical_layout(&layout.physical);
    let lighting_topology = expand_lighting_topology(lighting);

    // Generate extra imports, panic handler and logger
    let imports = match hardware.chip.series {
        ChipSeries::Esp32 => quote! {
            use esp_alloc as _;
            use esp_backtrace as _;
            ::esp_bootloader_esp_idf::esp_app_desc!();
        },
        _ => {
            // If defmt_log is disabled, add an empty defmt logger impl
            if hardware.dependency.defmt_log {
                quote! {
                    use panic_probe as _;
                    use defmt_rtt as _;
                }
            } else {
                quote! {
                    use panic_probe as _;

                    #[::defmt::global_logger]
                    struct Logger;

                    unsafe impl ::defmt::Logger for Logger {
                        fn acquire() {}
                        unsafe fn flush() {}
                        unsafe fn release() {}
                        unsafe fn write(_bytes: &[u8]) {}
                    }
                }
            }
        }
    };

    quote! {
        #imports

        #keyboard_info_static_var
        #vial_static_var
        #lock_static_var
        #default_keymap
        #physical_layout
        #lighting_topology
    }
}

#[cfg(test)]
mod tests {
    use super::validate_feature_config_parity;

    /// Build the enabled-Cargo-features list that `validate_feature_config_parity` reads.
    fn features(enabled: &[&str]) -> Option<Vec<String>> {
        Some(enabled.iter().map(|f| f.to_string()).collect())
    }

    #[test]
    fn accepts_matching_storage_vial_rynk_feature_states() {
        assert!(
            validate_feature_config_parity(
                &features(&["storage", "vial"]),
                true,
                true,
                false,
                false
            )
            .is_ok()
        );
        assert!(validate_feature_config_parity(&features(&[]), false, false, false, false).is_ok());
        assert!(
            validate_feature_config_parity(&features(&["storage"]), true, false, false, false)
                .is_ok()
        );
        assert!(
            validate_feature_config_parity(
                &features(&["storage", "rynk"]),
                true,
                false,
                true,
                false
            )
            .is_ok()
        );
        assert!(
            validate_feature_config_parity(&features(&["lighting"]), false, false, false, false)
                .is_ok(),
            "the lighting feature supports public Rust construction without TOML"
        );
        assert!(
            validate_feature_config_parity(&features(&["lighting"]), false, false, false, true)
                .is_ok()
        );
    }

    #[test]
    fn rejects_storage_enabled_in_config_without_feature() {
        let err =
            validate_feature_config_parity(&features(&[]), true, false, false, false).unwrap_err();
        assert_eq!(
            err,
            "If the \"storage\" Cargo feature is disabled, `storage.enabled` must be set to false in keyboard.toml."
        );
    }

    #[test]
    fn rejects_storage_feature_without_config() {
        let err =
            validate_feature_config_parity(&features(&["storage"]), false, false, false, false)
                .unwrap_err();
        assert_eq!(
            err,
            "`storage.enabled = false` in keyboard.toml requires disabling the \"storage\" Cargo feature for rmk in Cargo.toml (for example with `default-features = false` and explicitly re-enabling the features you need)."
        );
    }

    #[test]
    fn rejects_vial_enabled_in_config_without_feature() {
        let err =
            validate_feature_config_parity(&features(&[]), false, true, false, false).unwrap_err();
        assert_eq!(
            err,
            "If the \"vial\" Cargo feature is disabled, `host.vial_enabled` must be set to false in keyboard.toml."
        );
    }

    #[test]
    fn rejects_vial_feature_without_config() {
        let err = validate_feature_config_parity(&features(&["vial"]), false, false, false, false)
            .unwrap_err();
        assert_eq!(
            err,
            "`host.vial_enabled = false` in keyboard.toml requires disabling the \"vial\" Cargo feature for rmk in Cargo.toml (for example with `default-features = false` and explicitly re-enabling the features you need)."
        );
    }

    #[test]
    fn rejects_rynk_enabled_in_config_without_feature() {
        let err =
            validate_feature_config_parity(&features(&[]), false, false, true, false).unwrap_err();
        assert_eq!(
            err,
            "If the \"rynk\" Cargo feature is disabled, `host.rynk_enabled` must be set to false in keyboard.toml."
        );
    }

    #[test]
    fn rejects_rynk_feature_without_config() {
        let err = validate_feature_config_parity(&features(&["rynk"]), false, false, false, false)
            .unwrap_err();
        assert_eq!(
            err,
            "`host.rynk_enabled = false` in keyboard.toml requires disabling the \"rynk\" Cargo feature for rmk in Cargo.toml (for example with `default-features = false` and explicitly re-enabling the features you need)."
        );
    }

    #[test]
    fn rejects_vial_and_rynk_both_enabled() {
        let err =
            validate_feature_config_parity(&features(&["vial", "rynk"]), false, true, true, false)
                .unwrap_err();
        assert_eq!(
            err,
            "`host.vial_enabled` and `host.rynk_enabled` are mutually exclusive — set exactly one to true (the underlying Cargo features for rmk also conflict)."
        );
    }

    #[test]
    fn rejects_lighting_config_without_feature() {
        let err =
            validate_feature_config_parity(&features(&[]), false, false, false, true).unwrap_err();
        assert_eq!(
            err,
            "A `[lighting]` section in keyboard.toml requires enabling the \"lighting\" Cargo feature for rmk."
        );
    }
}

fn expand_main(
    host: &Host,
    hardware: &Hardware,
    behavior: &Behavior,
    keymap: &Keymap,
    layout: &Layout,
    item_mod: syn::ItemMod,
    rmk_features: &Option<Vec<String>>,
) -> TokenStream2 {
    // Expand components of main function
    let imports = expand_custom_imports(&item_mod);
    let bind_interrupt = expand_bind_interrupt(hardware, &item_mod);
    let chip_init = expand_chip_init(hardware, None, &item_mod);
    let usb_init = expand_usb_init(hardware, &item_mod);
    let flash_init = expand_flash_init(hardware);
    let behavior_config = expand_behavior_config(behavior);
    let matrix_config = expand_matrix_config(hardware, rmk_features);
    let output_config = expand_output_config(hardware);
    let (ble_config, set_ble_config) = expand_ble_config(hardware);
    let keymap_and_storage = expand_keymap_and_storage(hardware, keymap);
    let split_central_config = expand_split_central_config(hardware);
    let (input_device_config, devices, processors) = expand_input_device_config(hardware);
    let matrix_and_keyboard = expand_matrix_and_keyboard_init(hardware);
    let (registered_processor_initializers, mut registered_processors) =
        expand_registered_processor_init(hardware, &item_mod, rmk_features);

    // Declare dfu_lock (if enabled) so it can be pushed as a Runnable task.
    // Check the feature at macro-expansion time so we never emit `#[cfg]`
    // into the user's crate (avoids "unexpected cfg condition" warnings).
    let dfu_lock_enabled = is_feature_enabled(rmk_features, "dfu_lock");
    let dfu_lock_init = if let Some(ref dfu) = hardware.dfu {
        if dfu_lock_enabled && !dfu.unlock_keys.is_empty() {
            registered_processors.push(quote! { dfu_lock.run() });
            quote! {
                let mut dfu_lock = ::rmk::dfu::DfuLock::new(&DFU_UNLOCK_KEYS, &keymap);
            }
        } else {
            quote! {}
        }
    } else {
        quote! {}
    };

    // Register peripheral firmware binaries for automatic dfu_split updates
    let split_firmware_init = if is_feature_enabled(rmk_features, "dfu_split") {
        expand_split_firmware_init(&hardware.board)
    } else {
        quote! {}
    };

    // Display configuration — for unibody use top-level, for split use central's config
    let display_config = match &hardware.board {
        BoardConfig::UniBody(_) => hardware.display.as_ref(),
        BoardConfig::Split(split_config) => split_config.central.display.as_ref(),
    };
    let display_init = if let Some(display_config) = display_config {
        let (init, processor) = expand_display_config(&hardware.chip.series, display_config);
        let processor_initializer = processor.initializer;
        let processor_var = processor.var_name;
        registered_processors.push(quote! { #processor_var.run() });
        quote! {
            #init
            #processor_initializer
        }
    } else {
        quote! {}
    };

    // Rynk-only: bake the physical-layout blob as a compile-time const and enable
    // the lock gate. Both fields land adjacently in the `RmkConfig` literal below.
    let (layout_blob_static, rynk_layout_field, lock_config) = if host.rynk_enabled {
        let blob_lit = proc_macro2::Literal::byte_string(&layout.blob);
        (
            quote! { static LAYOUT_BLOB: &[u8] = #blob_lit; },
            quote! { layout_blob: LAYOUT_BLOB, },
            quote! { lock_config: LOCK_CONFIG, },
        )
    } else {
        (quote! {}, quote! {}, quote! {})
    };

    let host_service_init = if host.rynk_enabled || host.vial_enabled {
        quote! {
            let host_service = ::rmk::host::HostService::new(&keymap, &rmk_config);
        }
    } else {
        quote! {}
    };

    let (watchdog_init, watchdog_task) = expand_watchdog_init(hardware);

    let run_rmk = expand_rmk_entry(
        hardware,
        host,
        behavior,
        &item_mod,
        devices,
        processors,
        registered_processors,
        watchdog_task,
    );

    let vial_config = if host.vial_enabled {
        quote! { vial_config: VIAL_CONFIG,}
    } else {
        quote! {}
    };

    let rmk_config = if hardware.storage.is_some() {
        quote! {
            #[allow(clippy::needless_update)]
            let rmk_config = ::rmk::config::RmkConfig {
                device_config: KEYBOARD_DEVICE_CONFIG,
                #vial_config
                #lock_config
                #rynk_layout_field
                storage_config,
                #set_ble_config
                ..Default::default()
            };
        }
    } else {
        quote! {
            #[allow(clippy::needless_update)]
            let rmk_config = ::rmk::config::RmkConfig {
                device_config: KEYBOARD_DEVICE_CONFIG,
                #vial_config
                #lock_config
                #rynk_layout_field
                #set_ble_config
                ..Default::default()
            };
        }
    };

    let main_function_sig = if hardware.chip.series == ChipSeries::Esp32 {
        quote! {
            #[::esp_rtos::main]
            async fn main(_s: ::embassy_executor::Spawner)
        }
    } else {
        quote! {
            #[::embassy_executor::main]
            async fn main(spawner: ::embassy_executor::Spawner)
        }
    };

    quote! {
        #imports

        #bind_interrupt

        #main_function_sig {
            // Initialize peripherals as `p`
            #chip_init

            // Initialize usb driver as `driver`
            #usb_init

            // Initialize behavior config config as `behavior_config`
            #behavior_config

            // Initialize matrix config as `(row_pins, col_pins)` or `direct_pins`
            #matrix_config

            // Initialize static output pins
            #output_config

            // Initialize flash driver as `flash` and storage config as `storage_config`
            #flash_init

            // Initialize ble config as `ble_battery_config`
            #ble_config

            // Bake the physical-layout blob (rynk) as a compile-time const
            #layout_blob_static

            // Set all keyboard config
            #rmk_config

            // Initialize the registered processors
            #registered_processor_initializers

            // Initialize the storage and keymap, as `storage` and `keymap`
            #keymap_and_storage

            // Initialize the matrix + keyboard, as `matrix` and `keyboard`
            #matrix_and_keyboard

            // Initialize the host (Vial / Rynk) service, as `host_service`
            #host_service_init

            // Initialize input device config as `input_device_config` and processor as `processor`
            #input_device_config

            // Initialize display (if configured)
            #display_init

            // Initialize split central config(if needed)
            #split_central_config

            // Initialize watchdog
            #watchdog_init

            // Initialize dfu lock
            #dfu_lock_init

            // Register peripheral firmware binaries for split dfu updates
            #split_firmware_init

            // Start
            #run_rmk
        }
    }
}

/// Generate `set_firmware_update_data` calls for each peripheral that has a
/// `firmware` path configured in `keyboard.toml`.
fn expand_split_firmware_init(board: &BoardConfig) -> TokenStream2 {
    let BoardConfig::Split(split_config) = board else {
        return quote! {};
    };

    let inits: Vec<_> = split_config
        .peripheral
        .iter()
        .enumerate()
        .filter_map(|(id, p)| {
            p.firmware.as_ref().map(|path| {
                quote! {
                    {
                        const FW: &[u8] =
                            ::core::include_bytes!(::core::concat!(
                                ::core::env!("CARGO_MANIFEST_DIR"), "/", #path
                            ));
                        let _ = ::rmk::dfu::set_firmware_update_data(
                            #id, FW, ::rmk::crc32::crc32(FW)
                        );
                    }
                }
            })
        })
        .collect();

    if inits.is_empty() {
        quote! {}
    } else {
        quote! {
            #(#inits)*
        }
    }
}

// TODO: move this function to a separate folder
pub(crate) fn expand_keymap_and_storage(hardware: &Hardware, keymap: &Keymap) -> TokenStream2 {
    let row = keymap.rows as usize;
    let col = keymap.cols as usize;

    let initialize_positional_config = if keymap.key_info.is_empty()
        || keymap.key_info.iter().all(|row| {
            row.iter().all(|key| {
                key.hand != 'L'
                    && key.hand != 'l'
                    && key.hand != 'R'
                    && key.hand != 'r'
                    && key.hand != '*'
            })
        })
        || keymap.key_info.len() != row
        || keymap.key_info[0].len() != col
    {
        quote! { let per_key_config = ::rmk::config::PositionalConfig::default(); }
    } else {
        let key_info_config = expand_key_info(&keymap.key_info);
        quote! { let per_key_config = ::rmk::config::PositionalConfig::new(#key_info_config); }
    };

    let total_num_encoders = keymap.num_encoder;

    let keymap_data_init = if total_num_encoders == 0 {
        quote! {
            let mut keymap_data = ::rmk::KeymapData::new(get_default_keymap());
        }
    } else {
        quote! {
            let mut keymap_data = ::rmk::KeymapData::new_with_encoder(
                get_default_keymap(),
                get_default_encoder_map(),
            );
        }
    };

    if hardware.storage.is_some() {
        #[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
        let mark = quote! { ::rmk::dfu::mark_booted(); };
        #[cfg(not(any(feature = "dfu_rp", feature = "dfu_nrf")))]
        let mark = quote! {};

        quote! {
            #initialize_positional_config
            #keymap_data_init
            let (keymap, mut storage) = ::rmk::initialize_keymap_and_storage(
                &mut keymap_data,
                flash,
                &rmk_config.storage_config,
                &mut behavior_config,
                &per_key_config,
            ).await;
            #mark
        }
    } else {
        quote! {
            #initialize_positional_config
            #keymap_data_init
            let keymap = ::rmk::initialize_keymap(
                &mut keymap_data,
                &mut behavior_config,
                &per_key_config,
            ).await;
        }
    }
}

pub(crate) fn expand_matrix_and_keyboard_init(hardware: &Hardware) -> TokenStream2 {
    let matrix = match &hardware.board {
        BoardConfig::UniBody(UniBodyConfig {
            matrix: matrix_config,
            input_device: _,
        }) => {
            let bootmagic = expand_bootmagic_check(matrix_config);
            let debouncer_type = get_debouncer_type(matrix_config);
            match matrix_config.matrix_type {
                MatrixType::Normal => {
                    let col2row = !matrix_config.row2col;
                    quote! {
                        #bootmagic
                        let debouncer = #debouncer_type::new();
                        let mut matrix = ::rmk::matrix::Matrix::<_, _, _, ROW, COL, #col2row>::new(row_pins, col_pins, debouncer);
                    }
                }
                MatrixType::DirectPin => {
                    let low_active = matrix_config.direct_pin_low_active;
                    quote! {
                        #bootmagic
                        let debouncer = #debouncer_type::new();
                        let mut matrix = ::rmk::matrix::direct_pin::DirectPinMatrix::<_, _, ROW, COL, SIZE>::new(direct_pins, debouncer, #low_active);
                    }
                }
            }
        }
        BoardConfig::Split(split_config) => {
            // Matrix config for split central
            let central_row = split_config.central.rows;
            let central_row_offset = split_config.central.row_offset;
            let central_col = split_config.central.cols;
            let central_col_offset = split_config.central.col_offset;
            let col2row = !split_config.central.matrix.row2col;
            let bootmagic = expand_bootmagic_check(&split_config.central.matrix);
            let debouncer_type = get_debouncer_type(&split_config.central.matrix);
            match split_config.central.matrix.matrix_type {
                MatrixType::Normal => {
                    quote! {
                        #bootmagic
                        let debouncer = #debouncer_type::new();
                        let mut matrix = ::rmk::matrix::Matrix::<_, _, _, #central_row, #central_col, #col2row, #central_row_offset, #central_col_offset>::new(row_pins, col_pins, debouncer);
                    }
                }
                MatrixType::DirectPin => {
                    let low_active = split_config.central.matrix.direct_pin_low_active;
                    let size = split_config.central.rows * split_config.central.cols;
                    quote! {
                        #bootmagic
                        let debouncer = #debouncer_type::new();
                        let mut matrix = ::rmk::matrix::direct_pin::DirectPinMatrix::<_, _, #central_row, #central_col, #size, #central_row_offset, #central_col_offset>::new(direct_pins, debouncer, #low_active);
                    }
                }
            }
        }
    };
    quote! {
        let mut keyboard = ::rmk::keyboard::Keyboard::new(&keymap);
        #matrix
    }
}

/// Push rows in the key_info
fn expand_key_info(info: &Vec<Vec<KeyInfo>>) -> proc_macro2::TokenStream {
    let mut rows = vec![];
    for row in info {
        rows.push(expand_key_info_row(row));
    }
    quote! { [#(#rows), *] }
}

/// Push keys info in the row
fn expand_key_info_row(row: &Vec<KeyInfo>) -> proc_macro2::TokenStream {
    let mut key_info = vec![];
    for key in row {
        let hand = match key.hand {
            'l' | 'L' => quote! { rmk::config::Hand::Left },
            'r' | 'R' => quote! { rmk::config::Hand::Right },
            '*' => quote! { rmk::config::Hand::Bilateral },
            _ => quote! { rmk::config::Hand::Unknown },
        };
        key_info.push(hand);
    }
    quote! { [#(#key_info), *] }
}

/// Get debouncer type
pub(crate) fn get_debouncer_type(matrix_config: &MatrixConfig) -> TokenStream2 {
    match matrix_config.debouncer {
        DebouncerType::Fast => quote! { ::rmk::debounce::fast_debouncer::FastDebouncer },
        DebouncerType::Default => quote! { ::rmk::debounce::default_debouncer::DefaultDebouncer },
    }
}
