//! Initialize flash boilerplate of RMK, including USB or BLE
//!

use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::ChipSeries;

#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
use rmk_config::resolved::hardware::DfuConfig;

pub(crate) fn expand_flash_init(hardware: &Hardware) -> TokenStream2 {
    if hardware.storage.is_none() {
        // This config actually does nothing if storage is disabled
        return quote! {
            // let storage_config = ::rmk::config::StorageConfig::default();
            // let flash = ::rmk::DummyFlash::new();
        };
    }
    let storage = hardware.storage.as_ref().unwrap();
    let num_sectors = storage.num_sectors;
    let _start_addr = storage.start_addr;
    let clear_storage = storage.clear_storage;
    let clear_layout = storage.clear_layout;

    // With dfu, the flash is already a partition that starts at the
    // storage offset, so the relative offset must be 0.
    #[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
    let storage_start_addr = 0usize;
    #[cfg(not(any(feature = "dfu_rp", feature = "dfu_nrf")))]
    let storage_start_addr = _start_addr;

    let mut flash_init = quote! {
        let storage_config = ::rmk::config::StorageConfig {
            num_sectors: #num_sectors,
            start_addr: #storage_start_addr,
            clear_storage: #clear_storage,
            clear_layout: #clear_layout
        };
    };
    flash_init.extend(
    match hardware.chip.series {
            ChipSeries::Stm32 => {
                quote! {
                    let flash = ::rmk::storage::async_flash_wrapper(::embassy_stm32::flash::Flash::new_blocking(p.FLASH));
                }
            }
            ChipSeries::Nrf52 => {
                #[cfg(feature = "dfu_nrf")]
                let flash_code = {
                    let dfu = hardware.dfu.as_ref().expect(
                        "[dfu] section is required in keyboard.toml (or chip default) when dfu_nrf is enabled"
                    );
                    let dfu_unlock_keys = expand_dfu_unlock_keys(dfu);
                    quote! {
                        #dfu_unlock_keys
                        let flash = ::rmk::storage::async_flash_wrapper(
                            ::rmk::dfu::init_flash_from_linkerscript(p.NVMC)
                        );
                    }
                };
                #[cfg(not(feature = "dfu_nrf"))]
                let flash_code = quote! {
                    let flash = ::nrf_mpsl::Flash::take(mpsl, p.NVMC);
                };
                flash_code
            }
        ChipSeries::Rp2040 => {
            #[cfg(not(feature = "dfu_rp"))]
            {
                quote! {
                    const FLASH_SIZE: usize = 2 * 1024 * 1024;
                    let flash = ::embassy_rp::flash::Flash::<_, ::embassy_rp::flash::Async, FLASH_SIZE>::new(
                        p.FLASH, p.DMA_CH1, Irqs,
                    );
                }
            }
            #[cfg(feature = "dfu_rp")]
            {
                let dfu = hardware.dfu.as_ref().expect(
                    "[dfu] section is required in keyboard.toml (or chip default) when dfu_rp is enabled"
                );
                let dfu_unlock_keys = expand_dfu_unlock_keys(dfu);
                quote! {
                    #dfu_unlock_keys
                    let flash = ::rmk::storage::async_flash_wrapper(
                        ::rmk::dfu::init_flash_from_linkerscript(p.FLASH)
                    );
                }
            }
            }
            ChipSeries::Esp32 => {
                // ESP32 and ESP32-S3 are dual-core. Flash writes must auto-park it to avoid
                // `FlashStorageError::OtherCoreRunning`.
                let chip_name = hardware.chip.chip.to_lowercase();
                if chip_name == "esp32s3"{
                    quote! {
                        let flash = ::rmk::storage::async_flash_wrapper(
                            ::esp_storage::FlashStorage::new(p.FLASH).multicore_auto_park()
                        );
                    }
                } else {
                    quote! {
                        let flash = ::rmk::storage::async_flash_wrapper(::esp_storage::FlashStorage::new(p.FLASH));
                    }
                }
            },
        }
    );

    flash_init
}

/// Generate the `DFU_UNLOCK_KEYS` constant from the resolved DFU config.
#[cfg(any(feature = "dfu_rp", feature = "dfu_nrf"))]
fn expand_dfu_unlock_keys(dfu: &DfuConfig) -> TokenStream2 {
    if dfu.unlock_keys.is_empty() {
        return quote! {};
    }
    let keys_expr = dfu
        .unlock_keys
        .iter()
        .map(|key| {
            let row = key[0];
            let col = key[1];
            quote! { (#row, #col) }
        })
        .collect::<Vec<_>>();
    quote! {
        const DFU_UNLOCK_KEYS: &[(u8, u8)] = &[#(#keys_expr), *];
    }
}
