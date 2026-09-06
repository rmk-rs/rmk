//! Initialize flash boilerplate of RMK, including USB or BLE
//!

#[cfg(feature = "_dfu")]
use crate::codegen::feature::is_feature_enabled;
use proc_macro2::TokenStream as TokenStream2;
#[cfg(feature = "_dfu")]
use quote::format_ident;
use quote::quote;
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::{ChipSeries, DfuConfig};
#[cfg(feature = "_dfu")]
use rmk_config::resolved::hardware::{ExternalFlashDriver, SpiConfig};

pub(crate) fn expand_flash_init(hardware: &Hardware, dfu: Option<&DfuConfig>) -> TokenStream2 {
    // Only consulted by the DFU-capable chip arms below; the `#[cfg]`s strip
    // those away on other chips.
    let _ = dfu;

    let storage_config = hardware.storage.as_ref().map(|storage| {
        let num_sectors = storage.num_sectors;
        let _start_addr = storage.start_addr;
        let clear_storage = storage.clear_storage;
        let clear_layout = storage.clear_layout;

        // With dfu, the flash is already a partition that starts at the
        // storage offset, so the relative offset must be 0.
        #[cfg(feature = "_dfu")]
        let storage_start_addr = 0usize;
        #[cfg(not(feature = "_dfu"))]
        let storage_start_addr = _start_addr;

        quote! {
            let storage_config = ::rmk::config::StorageConfig {
                num_sectors: #num_sectors,
                start_addr: #storage_start_addr,
                clear_storage: #clear_storage,
                clear_layout: #clear_layout
            };
        }
    });

    let flash_init = match hardware.chip.series {
        ChipSeries::Stm32 => {
            hardware.storage.as_ref().map(|_| {
                quote! {
                    let storage_partition = ::rmk::storage::async_flash_wrapper(::embassy_stm32::flash::Flash::new_blocking(p.FLASH));
                }
            })
        }
        ChipSeries::Nrf52 => {
            #[cfg(feature = "dfu_nrf")]
            {
                let dfu = dfu.expect(
                    "[dfu] section is required in keyboard.toml (or chip default) when dfu_nrf is enabled"
                );
                let dfu_unlock_keys = expand_dfu_unlock_keys(dfu);
                let dfu_ext_error = dfu_ext_misconfig_error(Some(dfu));
                let external_dfu = if dfu_ext_error.is_some() {
                    None
                } else {
                    expand_external_flash_init(&ChipSeries::Nrf52, Some(dfu))
                };
                let partitions = expand_dfu_partitions(
                    &ChipSeries::Nrf52,
                    external_dfu.as_ref(),
                    hardware.storage.is_some(),
                );
                Some(quote! {
                    #dfu_unlock_keys
                    #dfu_ext_error
                    #external_dfu
                    #partitions
                })
            }
            #[cfg(not(feature = "dfu_nrf"))]
            {
                hardware.storage.as_ref().map(|_| {
                    quote! {
                        let storage_partition = ::nrf_mpsl::Flash::take(mpsl, p.NVMC);
                    }
                })
            }
        }
        ChipSeries::Rp2040 => {
            #[cfg(feature = "dfu_rp")]
            {
                let dfu = dfu.expect(
                    "[dfu] section is required in keyboard.toml (or chip default) when dfu_rp is enabled"
                );
                let dfu_unlock_keys = expand_dfu_unlock_keys(dfu);
                let dfu_ext_error = dfu_ext_misconfig_error(Some(dfu));
                let external_dfu = if dfu_ext_error.is_some() {
                    None
                } else {
                    expand_external_flash_init(&ChipSeries::Rp2040, Some(dfu))
                };
                let partitions = expand_dfu_partitions(
                    &ChipSeries::Rp2040,
                    external_dfu.as_ref(),
                    hardware.storage.is_some(),
                );
                Some(quote! {
                    #dfu_unlock_keys
                    #dfu_ext_error
                    #external_dfu
                    #partitions
                })
            }
            #[cfg(not(feature = "dfu_rp"))]
            {
                hardware.storage.as_ref().map(|_| {
                    quote! {
                        const FLASH_SIZE: usize = 2 * 1024 * 1024;
                        let storage_partition = ::embassy_rp::flash::Flash::<_, ::embassy_rp::flash::Async, FLASH_SIZE>::new(
                            p.FLASH, p.DMA_CH1, Irqs,
                        );
                    }
                })
            }
        }
        ChipSeries::Esp32 => {
            hardware.storage.as_ref().map(|_| {
                // ESP32 and ESP32-S3 are dual-core. Flash writes must auto-park it to avoid
                // `FlashStorageError::OtherCoreRunning`.
                let chip_name = hardware.chip.chip.to_lowercase();
                if chip_name == "esp32s3" {
                    quote! {
                        let storage_partition = ::rmk::storage::async_flash_wrapper(
                            ::esp_storage::FlashStorage::new(p.FLASH).multicore_auto_park()
                        );
                    }
                } else {
                    quote! {
                        let storage_partition = ::rmk::storage::async_flash_wrapper(::esp_storage::FlashStorage::new(p.FLASH));
                    }
                }
            })
        }
    };

    quote! {
        #storage_config
        #flash_init
    }
}

/// The internal flash driver expression wrapped by the DFU flash mutex.
///
/// nRF takes the raw `NVMC` directly; RP2040 needs the blocking `Flash`
/// wrapper sized by `rmk::dfu::FLASH_SIZE` (16 MB, matching the memory.x
/// conventions).
#[cfg(feature = "_dfu")]
fn expand_dfu_flash_driver(
    chip_series: &ChipSeries,
    flash_peripheral: &TokenStream2,
) -> TokenStream2 {
    match chip_series {
        ChipSeries::Nrf52 => quote! {
            ::embassy_nrf::nvmc::Nvmc::new(#flash_peripheral)
        },
        ChipSeries::Rp2040 => quote! {
            ::embassy_rp::flash::Flash::<_, ::embassy_rp::flash::Blocking, {::rmk::dfu::FLASH_SIZE}>::new_blocking(#flash_peripheral)
        },
        _ => panic!("Internal flash DFU is only supported on nRF52 and RP2040"),
    }
}

/// Build the internal `dfu_partition`/`state_partition`/`storage_partition`
/// lets from the DFU layout in `memory.x`, via
/// [`partitions_from_linkerscript`](::rmk::dfu::partitions_from_linkerscript).
///
/// When `external_dfu` is `Some` the DFU download partition lives on the
/// external (SPI) flash — defined there as `dfu_partition` — and only state +
/// storage come from the internal flash here; the internal `dfu` partition is
/// discarded.
///
/// The `storage_partition` let is only bound when `emit_storage_flash` is set,
/// i.e. the keymap/storage layer is enabled (`[storage]` in `keyboard.toml`).
/// Without it, the storage offset of the DFU layout stays unused and no
/// binding leaks into the final scope.
///
/// The flash mutex is a local (`let`), not a leaked `'static`: the storage
/// partition and the DFU/state partitions all borrow it, and every consumer
/// lives in the same main task, so no `'static` is required.
#[cfg(feature = "_dfu")]
fn expand_dfu_partitions(
    chip_series: &ChipSeries,
    external_dfu: Option<&TokenStream2>,
    emit_storage_flash: bool,
) -> TokenStream2 {
    let flash_peripheral = match chip_series {
        ChipSeries::Nrf52 => quote! { p.NVMC },
        ChipSeries::Rp2040 => quote! { p.FLASH },
        _ => panic!("Internal flash DFU is only supported on nRF52 and RP2040"),
    };
    let driver = expand_dfu_flash_driver(chip_series, &flash_peripheral);
    let storage_binding = if emit_storage_flash {
        quote! { storage_partition }
    } else {
        quote! { _storage_partition }
    };
    let dfu_binding = if external_dfu.is_some() {
        quote! { _ }
    } else {
        quote! { dfu_partition }
    };
    quote! {
        let flash_mutex = ::rmk::dfu::FlashMutex::new(
            ::rmk::storage::async_flash_wrapper(#driver)
        );
        let (#storage_binding, mut state_partition, #dfu_binding) =
            ::rmk::dfu::partitions_from_linkerscript(&flash_mutex);
    }
}

/// Generate the `DFU_UNLOCK_KEYS` constant from the resolved DFU config.
#[cfg(feature = "_dfu")]
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

/// Generate external SPI flash initialization for DFU.
/// `Some(compile_error!)` when an external flash is configured in keyboard.toml
/// but the user crate lacks the `dfu_ext` cargo feature. Callers fall back to
/// the internal-partition expansion so the build fails with this single error
/// instead of follow-on resolution noise.
#[cfg(feature = "_dfu")]
fn dfu_ext_misconfig_error(dfu: Option<&DfuConfig>) -> Option<TokenStream2> {
    let configured = dfu.and_then(|d| d.external_flash.as_ref()).is_some();
    let enabled = is_feature_enabled(&crate::codegen::feature::get_rmk_features(), "dfu_ext");
    (configured && !enabled).then(|| {
        quote! {
            ::core::compile_error!(
                "[dfu.external_flash] is configured in keyboard.toml, but the cargo feature \
                 `dfu_ext` is not enabled; enable it or remove the section"
            );
        }
    })
}

///
/// Creates the external flash, wraps it in a local mutex, then slices a
/// `dfu_partition` from it. The tokens must run inside `expand_flash_init`,
/// directly before the internal-parts setup.
///
/// Returns `None` if `dfu_ext` is not enabled or no external flash is
/// configured. A configured external flash without the `dfu_ext` feature is
/// reported by [`dfu_ext_misconfig_error`] (which also forces this function
/// down the internal-partition path so the expansion stays consistent).
#[cfg(feature = "_dfu")]
fn expand_external_flash_init(
    chip_series: &ChipSeries,
    dfu: Option<&DfuConfig>,
) -> Option<TokenStream2> {
    let rmk_features = crate::codegen::feature::get_rmk_features();
    if !is_feature_enabled(&rmk_features, "dfu_ext") {
        return None;
    }
    let ext_flash = dfu?.external_flash.as_ref()?;
    let dfu_partition_size = ext_flash.dfu_partition_size;
    let spi_init = expand_spi_init(chip_series, &ext_flash.spi);
    let flash_init = match &ext_flash.driver {
        ExternalFlashDriver::W25q => {
            let size = ext_flash.flash_size;
            quote! {
                let ext_flash = ::rmk::driver::w25q::W25qNorFlash::new(dfu_spi, dfu_cs, #size);
            }
        }
        ExternalFlashDriver::Custom => {
            let init_fn_str = ext_flash
                .init_fn
                .as_ref()
                .expect("[dfu.external_flash] driver = \"custom\" requires `init_fn`");
            let init_fn: syn::Path = syn::parse_str(init_fn_str).unwrap_or_else(|e| {
                panic!(
                    "[dfu.external_flash] init_fn `{init_fn_str}` is not a valid function path: {e}"
                )
            });
            let size = ext_flash.flash_size;
            quote! {
                let ext_flash = #init_fn(dfu_spi, dfu_cs, #size);
            }
        }
    };
    Some(quote! {
        #spi_init
        #flash_init
        let dfu_mutex = ::embassy_sync::mutex::Mutex::<
            ::embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex,
            _,
        >::new(ext_flash);
        let dfu_partition = ::rmk::dfu::Partition::new(
            &dfu_mutex,
            0,
            #dfu_partition_size,
        );
    })
}

#[cfg(feature = "_dfu")]
fn expand_spi_init(chip_series: &ChipSeries, spi: &SpiConfig) -> TokenStream2 {
    let instance = format_ident!("{}", spi.instance);
    match chip_series {
        ChipSeries::Rp2040 => {
            let sck = format_ident!("{}", spi.sck);
            let mosi = format_ident!("{}", spi.mosi);
            let miso = format_ident!("{}", spi.miso);
            let cs = format_ident!("{}", spi.cs.as_ref().unwrap());
            let tx_dma = format_ident!(
                "{}",
                spi.tx_dma
                    .as_ref()
                    .expect("tx_dma required for async SPI on RP2040")
            );
            let rx_dma = format_ident!(
                "{}",
                spi.rx_dma
                    .as_ref()
                    .expect("rx_dma required for async SPI on RP2040")
            );
            quote! {
                let dfu_spi = ::embassy_rp::spi::Spi::new(
                    p.#instance,
                    p.#sck,
                    p.#mosi,
                    p.#miso,
                    p.#tx_dma,
                    p.#rx_dma,
                    Irqs,
                    ::embassy_rp::spi::Config::default(),
                );
                let dfu_cs = ::embassy_rp::gpio::Output::new(
                    p.#cs,
                    ::embassy_rp::gpio::Level::High,
                );
            }
        }
        ChipSeries::Nrf52 => {
            let sck = format_ident!("{}", spi.sck);
            let mosi = format_ident!("{}", spi.mosi);
            let miso = format_ident!("{}", spi.miso);
            let cs = format_ident!("{}", spi.cs.as_ref().unwrap());
            quote! {
                let mut dfu_spi_cfg = ::embassy_nrf::spim::Config::default();
                dfu_spi_cfg.frequency = ::embassy_nrf::spim::Frequency::M8;
                let dfu_spi = ::embassy_nrf::spim::Spim::new(
                    p.#instance, Irqs,
                    p.#sck, p.#miso, p.#mosi, dfu_spi_cfg,
                );
                let dfu_cs = ::embassy_nrf::gpio::Output::new(
                    p.#cs,
                    ::embassy_nrf::gpio::Level::High,
                    ::embassy_nrf::gpio::OutputDrive::Standard,
                );
            }
        }
        _ => panic!("External flash DFU is only supported on RP2040 and nRF52"),
    }
}

/// Generate the `dfu_iface` updater (`let mut dfu_iface =
/// ::rmk::dfu::FlashDfuHandler::new(dfu_partition, state_partition)`) for the
/// current board.
///
/// The updater is a [`Runnable`](::rmk::core_traits::Runnable) that runs in
/// the caller's main task, so no parking in a `'static` cell is needed — the
/// partitions are moved in by value and the transport builds its own
/// USB-side proxy internally.
pub(crate) fn expand_dfu_interface(dfu: Option<&DfuConfig>) -> TokenStream2 {
    if !cfg!(feature = "_dfu") {
        return quote! {};
    }
    let _ = dfu
        .expect("[dfu] section is required in keyboard.toml (or chip default) when dfu is enabled");

    quote! {
        let mut dfu_iface = ::rmk::dfu::FlashDfuHandler::new(dfu_partition, state_partition);
    }
}
