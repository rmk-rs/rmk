use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, quote};
use rmk_config::DcdcReg0Voltage;
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::{BoardConfig, ChipModel, ChipSeries, CommunicationConfig};
use syn::{ItemFn, ItemMod};

use crate::codegen::feature::{get_rmk_features, is_feature_enabled};
use crate::codegen::override_helper::{Overwritten, find_overwritten};

/// Expand chip initialization code
///
/// If `peripheral_id` is `None`, it means that the chip initialization is for the central.
/// Otherwise, the `peripheral_id` is the index of the peripheral.
pub(crate) fn expand_chip_init(
    hardware: &Hardware,
    peripheral_id: Option<usize>,
    item_mod: &ItemMod,
) -> TokenStream2 {
    // If there is a function with `#[Overwritten(usb)]`, override the chip initialization
    if let Some((_, items)) = &item_mod.content {
        items
            .iter()
            .find_map(|item| {
                if let syn::Item::Fn(item_fn) = &item
                    && let Some(Ok(overwritten)) = find_overwritten(item_fn)
                {
                    match overwritten {
                        Overwritten::ChipConfig => {
                            return Some(override_chip_config(&hardware.chip, item_fn));
                        }
                        Overwritten::ChipInit => {
                            // Override the whole chip initialization
                            let stmts = &item_fn.block.stmts;
                            return Some(quote! { #(#stmts)* });
                        }
                        _ => (),
                    }
                }
                None
            })
            .unwrap_or(chip_init_default(hardware, peripheral_id))
    } else {
        chip_init_default(hardware, peripheral_id)
    }
}

// Default implementations of chip initialization
pub(crate) fn chip_init_default(hardware: &Hardware, peripheral_id: Option<usize>) -> TokenStream2 {
    let chip = &hardware.chip;
    let communication = &hardware.communication;
    let peri_num = hardware.board.get_num_peripheral();
    match chip.series {
        ChipSeries::Stm32 => quote! {
                let config = ::embassy_stm32::Config::default();
                let mut p = ::embassy_stm32::init(config);
        },
        ChipSeries::Nrf52 => {
            let chip_cfg = &hardware.chip_config;
            let dcdc_config = if chip.chip == "nrf52840" {
                let reg0_enabled = chip_cfg.dcdc_reg0.unwrap_or(true);
                let reg1_enabled = chip_cfg.dcdc_reg1.unwrap_or(true);
                let (reg0_voltage, reg0_voltage_str) = match chip_cfg.dcdc_reg0_voltage {
                    Some(DcdcReg0Voltage::V1_8) => {
                        (quote! { ::embassy_nrf::config::Reg0Voltage::_1V8 }, "1V8")
                    }
                    _ => (quote! { ::embassy_nrf::config::Reg0Voltage::_3V3 }, "3V3"),
                };
                quote! {
                    config.dcdc.reg0_voltage = Some(#reg0_voltage);
                    config.dcdc.reg0 = #reg0_enabled;
                    config.dcdc.reg1 = #reg1_enabled;
                    ::defmt::info!("DCDC config: reg0_voltage={}, reg0={}, reg1={}", #reg0_voltage_str, #reg0_enabled, #reg1_enabled);
                }
            } else if chip.chip == "nrf52833" {
                let reg1_enabled = chip_cfg.dcdc_reg1.unwrap_or(true);
                quote! {
                    config.dcdc.reg1 = #reg1_enabled;
                    ::defmt::info!("DCDC config: reg1={}", #reg1_enabled);
                }
            } else {
                quote! {}
            };
            let ble_addr = get_ble_addr(hardware, peripheral_id);
            // Calculate the size of sdc memory pool.
            // Unibody: 4696. Split central: 6080 + (N-1) * 2288 per peripheral.
            // Base memory sizes for the nrf-sdc memory pool
            const SDC_MEM_UNIBODY: usize = 4696;
            const SDC_MEM_SPLIT_BASE: usize = 6080;
            const SDC_MEM_PER_EXTRA_PERIPHERAL: usize = 2288;

            // Connection subrating adds extra memory per connection
            const SDC_MEM_SUBRATING_BASE: usize = 136;
            const SDC_MEM_SUBRATING_PER_PERIPHERAL: usize = 56;
            const SDC_MEM_SUBRATING_PER_EXTRA_PERIPHERAL: usize = 8;

            let subrating_enabled = is_feature_enabled(&get_rmk_features(), "subrating");

            let sdc_mem_size = if peripheral_id.is_none() && peri_num > 0 {
                // Split central
                let base =
                    SDC_MEM_SPLIT_BASE + peri_num.saturating_sub(1) * SDC_MEM_PER_EXTRA_PERIPHERAL;
                if subrating_enabled {
                    base + SDC_MEM_SUBRATING_BASE
                        + peri_num.saturating_sub(1) * SDC_MEM_SUBRATING_PER_PERIPHERAL
                        + peri_num.saturating_sub(2) * SDC_MEM_SUBRATING_PER_EXTRA_PERIPHERAL
                } else {
                    base
                }
            } else {
                // Unibody or split peripheral
                if subrating_enabled {
                    SDC_MEM_UNIBODY + SDC_MEM_SUBRATING_BASE
                } else {
                    SDC_MEM_UNIBODY
                }
            };
            let ble_init = match &communication {
                CommunicationConfig::Ble(_) | CommunicationConfig::Both(_, _) => quote! {
                    // Initialize nrf-sdc and ble stack
                    let mpsl_p = ::nrf_sdc::mpsl::Peripherals::new(p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31);
                    let lfclk_cfg = ::nrf_sdc::mpsl::raw::mpsl_clock_lfclk_cfg_t {
                        source: ::nrf_sdc::mpsl::raw::MPSL_CLOCK_LF_SRC_RC as u8,
                        rc_ctiv: ::nrf_sdc::mpsl::raw::MPSL_RECOMMENDED_RC_CTIV as u8,
                        rc_temp_ctiv: ::nrf_sdc::mpsl::raw::MPSL_RECOMMENDED_RC_TEMP_CTIV as u8,
                        accuracy_ppm: ::nrf_sdc::mpsl::raw::MPSL_DEFAULT_CLOCK_ACCURACY_PPM as u16,
                        skip_wait_lfclk_started: ::nrf_sdc::mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED != 0,
                    };
                    static MPSL: ::static_cell::StaticCell<::nrf_sdc::mpsl::MultiprotocolServiceLayer> = ::static_cell::StaticCell::new();
                    static SESSION_MEM: ::static_cell::StaticCell<::nrf_sdc::mpsl::SessionMem<1>> = ::static_cell::StaticCell::new();
                    let mpsl = MPSL.init(::defmt::unwrap!(::nrf_sdc::mpsl::MultiprotocolServiceLayer::with_timeslots(
                        mpsl_p,
                        Irqs,
                        lfclk_cfg,
                        SESSION_MEM.init(::nrf_sdc::mpsl::SessionMem::new())
                    )));
                    spawner.spawn(mpsl_task(&*mpsl).unwrap());
                    let sdc_p = ::nrf_sdc::Peripherals::new(
                        p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24, p.PPI_CH25, p.PPI_CH26,
                        p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
                    );
                    let mut rng = ::embassy_nrf::rng::Rng::new(p.RNG, Irqs);
                    let mut sdc_mem = ::nrf_sdc::Mem::<#sdc_mem_size>::new();
                    let ble_controller = ::defmt::unwrap!(build_sdc(sdc_p, &mut rng, &*mpsl, &mut sdc_mem));
                    let ble_addr = #ble_addr;
                },
                _ => quote! {},
            };
            quote! {
                use embassy_nrf::interrupt::InterruptExt;
                let mut config = ::embassy_nrf::config::Config::default();
                #dcdc_config
                let p = ::embassy_nrf::init(config);
                #ble_init
            }
        }
        ChipSeries::Rp2040 => {
            let ble_addr = get_ble_addr(hardware, peripheral_id);
            if communication.ble_enabled() {
                quote! {
                    let config = ::embassy_rp::config::Config::default();
                    let p = ::embassy_rp::init(config);

                    #[cfg(feature = "skip-cyw43-firmware")]
                    let (fw, clm, btfw, nvram) = {
                        static EMPTY: &::cyw43::Aligned<::cyw43::A4, [u8]> = &::cyw43::Aligned([0u8; 0]);
                        (EMPTY, &[] as &[u8], EMPTY, EMPTY)
                    };

                    #[cfg(not(feature = "skip-cyw43-firmware"))]
                    let (fw, clm, btfw, nvram) = {
                        // IMPORTANT
                        //
                        // Download and make sure these files from https://github.com/embassy-rs/embassy/tree/main/cyw43-firmware
                        // are available in `./examples/rp-pico-w`. (should be automatic)
                        //
                        // IMPORTANT
                        let fw = ::cyw43::aligned_bytes!("../cyw43-firmware/43439A0.bin");
                        let clm = ::cyw43::aligned_bytes!("../cyw43-firmware/43439A0_clm.bin");
                        let btfw = ::cyw43::aligned_bytes!("../cyw43-firmware/43439A0_btfw.bin");
                        let nvram = ::cyw43::aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin");
                        (fw, clm, btfw, nvram)
                    };

                    let pwr = ::embassy_rp::gpio::Output::new(p.PIN_23, ::embassy_rp::gpio::Level::Low);
                    let cs = ::embassy_rp::gpio::Output::new(p.PIN_25, ::embassy_rp::gpio::Level::High);
                    let mut pio = ::embassy_rp::pio::Pio::new(p.PIO0, Irqs);
                    let spi = ::cyw43_pio::PioSpi::new(
                        &mut pio.common,
                        pio.sm0,
                        ::cyw43_pio::DEFAULT_CLOCK_DIVIDER,
                        pio.irq0,
                        cs,
                        p.PIN_24,
                        p.PIN_29,
                        ::embassy_rp::dma::Channel::new(p.DMA_CH0, Irqs),
                        ::embassy_rp::dma::Channel::new(p.DMA_CH2, Irqs),
                    );

                    static STATE: ::static_cell::StaticCell<::cyw43::State> = ::static_cell::StaticCell::new();
                    let state = STATE.init(::cyw43::State::new());
                    let (_net_device, bt_device, mut control, runner) = ::cyw43::new_with_bluetooth(state, pwr, spi, fw, btfw, nvram).await;
                    spawner.spawn(cyw43_task(runner).unwrap());
                    control.init(clm).await;

                    let ble_controller: ::bt_hci::controller::ExternalController<_, 10> = ::bt_hci::controller::ExternalController::new(bt_device);
                    let ble_addr = #ble_addr;
                }
            } else {
                quote! {
                    let config = ::embassy_rp::config::Config::default();
                    let p = ::embassy_rp::init(config);
                }
            }
        }
        ChipSeries::Esp32 => {
            let ble_addr = get_ble_addr(hardware, peripheral_id);
            quote! {
                ::esp_println::logger::init_logger_from_env();
                let p = ::esp_hal::init(::esp_hal::Config::default().with_cpu_clock(::esp_hal::clock::CpuClock::max()));
                ::esp_alloc::heap_allocator!(size: 72 * 1024);
                let timg0 = ::esp_hal::timer::timg::TimerGroup::new(p.TIMG0);
                let software_interrupt = ::esp_hal::interrupt::software::SoftwareInterruptControl::new(p.SW_INTERRUPT);
                ::esp_rtos::start(timg0.timer0, software_interrupt.software_interrupt0);
                let _trng_source = ::esp_hal::rng::TrngSource::new(p.RNG, p.ADC1);
                let connector = ::esp_radio::ble::controller::BleConnector::new(p.BT, Default::default()).unwrap();
                let ble_controller: ::bt_hci::controller::ExternalController<_, 64> = ::bt_hci::controller::ExternalController::new(connector);
                let ble_addr = #ble_addr;
            }
        }
    }
}

fn override_chip_config(chip: &ChipModel, item_fn: &ItemFn) -> TokenStream2 {
    let initialization = item_fn.block.to_token_stream();
    let mut initialization_tokens = quote! {
        let config = #initialization;
    };
    match chip.series {
        ChipSeries::Stm32 => initialization_tokens.extend(quote! {
            let mut p = ::embassy_stm32::init(config);
        }),
        ChipSeries::Nrf52 => initialization_tokens.extend(quote! {
            let mut p = ::embassy_nrf::init(config);
        }),
        ChipSeries::Rp2040 => initialization_tokens.extend(quote! {
            let mut p = ::embassy_rp::init(config);
        }),
        ChipSeries::Esp32 => initialization_tokens.extend(quote! {
            let p = ::esp_hal::init(::esp_hal::Config::default().with_cpu_clock(::esp_hal::clock::CpuClock::max()));
        }),
    }

    initialization_tokens
}

fn get_ble_addr(hardware: &Hardware, peripheral_id: Option<usize>) -> TokenStream2 {
    if hardware.chip.series == ChipSeries::Nrf52 {
        quote! {
            {
                let ficr = ::embassy_nrf::pac::FICR;
                let high = u64::from(ficr.deviceid(1).read());
                let addr = high << 32 | u64::from(ficr.deviceid(0).read());
                let addr = addr | 0x0000_c000_0000_0000;
                let ble_addr = addr.to_le_bytes()[..6].try_into().expect("Failed to read BLE address from FICR");
                ble_addr
            }
        }
    } else {
        // Check whether the address is set in the keyboard.toml, if not, use the default address
        let addr = match &hardware.board {
            BoardConfig::Split(split) => {
                match peripheral_id {
                    Some(id) => {
                        // Split peripheral
                        // The 4th byte is the peripheral index to make sure that the BLE address for each peripheral is different
                        let default_addr = [0x7e, 0xfe, 0x73, id as u8, 0x66, 0xe3];
                        split
                            .peripheral
                            .get(id)
                            .unwrap_or_else(|| panic!("There's no config for peripheral {}", id))
                            .ble_addr
                            .unwrap_or(default_addr)
                    }
                    None => {
                        // Split central
                        let default_addr = [0x18, 0xe2, 0x21, 0x80, 0xc0, 0xc7];
                        split.central.ble_addr.unwrap_or(default_addr)
                    }
                }
            }
            // TODO: allow user to set the BLE address for uni-body keyboards
            BoardConfig::UniBody(_uni_body) => [0x18, 0xe2, 0x21, 0x80, 0xc0, 0xc7],
        };
        quote! {
            [
                #(#addr),*
            ]
        }
    }
}
