//! Add `bind_interrupts!` boilerplate of RMK, including USB or BLE
//!

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::{
    BoardConfig, ChipModel, ChipSeries, DfuConfig, InputDeviceConfig, MatrixConfig, MatrixType,
    UniBodyConfig,
};
use syn::{ItemFn, ItemMod};

use crate::codegen::display::expand_display_interrupt;
use crate::codegen::feature::{get_dependency, get_rmk_features, is_feature_enabled};
use crate::codegen::input_device::iqs5xx::expand_iqs5xx_interrupts;
use crate::codegen::override_helper::{Overwritten, find_overwritten};

/// Does this function override the generated `bind_interrupts!` boilerplate?
///
/// Two markers are accepted:
/// - `#[Override(bind_interrupt)]` / `#[Overwritten(bind_interrupt)]` (the form the stm32h7
///   example documents), selected through the shared override matcher so inert attributes and
///   `cfg` gating behave like every other override;
/// - the legacy bare `#[bind_interrupt]` attribute (the original syntax), matched exactly as
///   before for backward compatibility: it must be the function's only attribute.
fn is_bind_interrupt_override(item_fn: &ItemFn) -> bool {
    let current = matches!(
        find_overwritten(item_fn),
        Some(Ok(Overwritten::BindInterrupt))
    );
    let legacy = item_fn.attrs.len() == 1
        && item_fn.attrs[0]
            .meta
            .path()
            .get_ident()
            .is_some_and(|i| i == "bind_interrupt");
    current || legacy
}

/// Expand `bind_interrupt!` stuffs, and other code before `main` function
pub(crate) fn expand_bind_interrupt(
    hardware: &Hardware,
    item_mod: &ItemMod,
    dfu: Option<&DfuConfig>,
) -> TokenStream2 {
    // If there is a function marked as the bind_interrupt override, use its body
    if let Some((_, items)) = &item_mod.content {
        items
            .iter()
            .find_map(|item| {
                if let syn::Item::Fn(item_fn) = &item
                    && is_bind_interrupt_override(item_fn)
                {
                    let content = &item_fn.block.stmts;
                    return Some(quote! {
                        #(#content)*
                    });
                }
                None
            })
            .unwrap_or(bind_interrupt_default(hardware, item_mod, dfu))
    } else {
        bind_interrupt_default(hardware, item_mod, dfu)
    }
}

pub(crate) fn find_extern_irqs(item_mod: &ItemMod) -> Vec<TokenStream2> {
    let mut extern_irqs: Vec<TokenStream2> = Vec::new();
    if let Some((_, items)) = &item_mod.content {
        items.iter().for_each(|item| {
            if let syn::Item::Macro(item_macro) = &item
                && item_macro.mac.path.is_ident("add_interrupt")
            {
                extern_irqs.push(item_macro.mac.tokens.clone());
            }
        });
    }
    extern_irqs
}

/// Bind the configured side's external flash SPIM interrupt on nRF52.
pub(crate) fn expand_external_flash_interrupt(
    chip: &ChipSeries,
    dfu: Option<&DfuConfig>,
) -> Option<TokenStream2> {
    if *chip != ChipSeries::Nrf52 {
        return None;
    }
    let flash = dfu?.external_flash.as_ref()?;
    let instance = format_ident!("{}", flash.spi.instance);
    Some(quote! {
        #instance => ::embassy_nrf::spim::InterruptHandler<::embassy_nrf::peripherals::#instance>;
    })
}

/// Expand default `bind_interrupt!` for different chips and nrf-sdc config for nRF52
pub(crate) fn bind_interrupt_default(
    hardware: &Hardware,
    item_mod: &ItemMod,
    dfu: Option<&DfuConfig>,
) -> TokenStream2 {
    let extern_irqs_vec = find_extern_irqs(item_mod);
    let extern_irqs = if extern_irqs_vec.is_empty() {
        quote! {}
    } else {
        quote! {
            #(#extern_irqs_vec)*
        }
    };

    let chip = &hardware.chip;
    let board = &hardware.board;
    let communication = &hardware.communication;
    let (matrix, input_devices) = match board {
        BoardConfig::UniBody(config) => (&config.matrix, Some(&config.input_device)),
        BoardConfig::Split(config) => {
            (&config.central.matrix, config.central.input_device.as_ref())
        }
    };
    let async_matrix = is_feature_enabled(&get_rmk_features(), "async_matrix");
    let gpio_interrupts = expand_gpio_interrupts(chip, matrix, input_devices, async_matrix);

    let display_config = match board {
        BoardConfig::UniBody(_) => hardware.display.as_ref(),
        BoardConfig::Split(split_config) => split_config.central.display.as_ref(),
    };
    let display_interrupt = if let Some(display_config) = display_config {
        expand_display_interrupt(&chip.series, display_config)
    } else {
        quote! {}
    };

    // IQS5xx devices on the unibody / central side need an I²C interrupt
    // binding so the async I²C driver can service the bus.
    let iqs5xx_config = match board {
        BoardConfig::UniBody(UniBodyConfig { input_device, .. }) => {
            input_device.clone().iqs5xx.unwrap_or(Vec::new())
        }
        BoardConfig::Split(split_config) => split_config
            .central
            .input_device
            .clone()
            .unwrap_or(InputDeviceConfig::default())
            .iqs5xx
            .unwrap_or(Vec::new()),
    };
    let iqs5xx_interrupt = expand_iqs5xx_interrupts(&chip.series, &iqs5xx_config);

    let ext_flash_spi_interrupt = expand_external_flash_interrupt(&chip.series, dfu);

    match chip.series {
        rmk_config::resolved::hardware::ChipSeries::Stm32 => {
            if let Some(usb_info) = communication.get_usb_info() {
                let interrupt_name = format_ident!("{}", usb_info.interrupt_name);
                let peripheral_name = format_ident!("{}", usb_info.peripheral_name);
                quote! {
                    use ::embassy_stm32::bind_interrupts;
                    bind_interrupts!(struct Irqs {
                        #interrupt_name => ::embassy_stm32::usb::InterruptHandler<::embassy_stm32::peripherals::#peripheral_name>;
                        #gpio_interrupts
                        #display_interrupt
                        #extern_irqs
                    });
                }
            } else if async_matrix {
                quote! {
                    use ::embassy_stm32::bind_interrupts;
                    bind_interrupts!(struct Irqs {
                        #gpio_interrupts
                        #display_interrupt
                        #extern_irqs
                    });
                }
            } else if !display_interrupt.is_empty() {
                quote! {
                    use ::embassy_stm32::bind_interrupts;
                    bind_interrupts!(struct Irqs {
                        #display_interrupt
                        #extern_irqs
                    });
                }
            } else {
                quote! {
                    #extern_irqs
                }
            }
        }
        rmk_config::resolved::hardware::ChipSeries::Nrf52 => {
            // Extract PMW33xx configuration
            let pmw33xx_config = match &board {
                BoardConfig::UniBody(UniBodyConfig { input_device, .. }) => {
                    input_device.clone().pmw33xx.unwrap_or(Vec::new())
                }
                BoardConfig::Split(split_config) => split_config
                    .central
                    .input_device
                    .clone()
                    .unwrap_or(InputDeviceConfig::default())
                    .pmw33xx
                    .unwrap_or(Vec::new()),
            };

            // Generate SPI interrupts for each sensor
            let mut pmw33xx_spi_interrupts = Vec::new();

            for sensor in &pmw33xx_config {
                let instance_ident = format_ident!("{}", &sensor.spi.instance);

                pmw33xx_spi_interrupts.push(quote! {
                    #instance_ident => ::embassy_nrf::spim::InterruptHandler<::embassy_nrf::peripherals::#instance_ident>;
                });
            }

            let pmw33xx_spi_interrupts = if pmw33xx_spi_interrupts.is_empty() {
                quote! {}
            } else {
                quote! {
                    #(#pmw33xx_spi_interrupts)*
                }
            };

            if !communication.ble_enabled() {
                let usb_interrupts = communication.get_usb_info().map(|usb| {
                    let interrupt = format_ident!("{}", usb.interrupt_name);
                    let peripheral = format_ident!("{}", usb.peripheral_name);
                    quote! {
                        #interrupt => ::embassy_nrf::usb::InterruptHandler<::embassy_nrf::peripherals::#peripheral>;
                        CLOCK_POWER => ::embassy_nrf::usb::vbus_detect::InterruptHandler;
                    }
                });
                return quote! {
                    ::embassy_nrf::bind_interrupts!(struct Irqs {
                        #usb_interrupts
                        #gpio_interrupts
                        #pmw33xx_spi_interrupts
                        #ext_flash_spi_interrupt
                        #iqs5xx_interrupt
                        #display_interrupt
                        #extern_irqs
                    });
                };
            }

            // Usb and clock interrupt
            let usb_and_clock_interrupt = if let Some(usb_info) = communication.get_usb_info() {
                let interrupt_name = format_ident!("{}", usb_info.interrupt_name);
                let peripheral_name = format_ident!("{}", usb_info.peripheral_name);
                quote! {
                    #interrupt_name => ::embassy_nrf::usb::InterruptHandler<::embassy_nrf::peripherals::#peripheral_name>;
                    CLOCK_POWER => ::nrf_sdc::mpsl::ClockInterruptHandler, ::embassy_nrf::usb::vbus_detect::InterruptHandler;
                }
            } else {
                quote! { CLOCK_POWER => ::nrf_sdc::mpsl::ClockInterruptHandler; }
            };

            let ble_config = communication.get_ble_config().unwrap();
            let tx_power = if let Some(pwr) = ble_config.default_tx_power {
                quote! { .default_tx_power(#pwr)?  }
            } else {
                quote! {}
            };

            let central_count = if get_dependency("nrf-sdc").is_some_and(|dep| {
                dep.req_features()
                    .iter()
                    .any(|feature| matches!(feature.as_str(), "central" | "multirole"))
            }) {
                quote! { .central_count(0)? }
            } else {
                quote! {}
            };

            let support_adv = if cfg!(feature = "ext_adv") {
                quote! { .support_ext_adv() }
            } else {
                quote! { .support_adv() }
            };

            // nrf-sdc interrupt config
            let nrf_sdc_config = match &board {
                BoardConfig::Split(_) => {
                    let num_peri = board.get_num_peripheral() as u8;
                    let support_central = if cfg!(feature = "ext_adv") {
                        quote! { .support_ext_central() }
                    } else {
                        quote! { .support_central() }
                    };
                    let support_subrating = if is_feature_enabled(&get_rmk_features(), "subrating")
                    {
                        quote! { .support_connection_subrating_central() }
                    } else {
                        quote! {}
                    };
                    quote! {
                        ::nrf_sdc::Builder::new()?
                        #support_central
                        #support_adv
                        .support_peripheral()
                        .support_dle_peripheral()
                        .support_dle_central()
                        .support_phy_update_central()
                        .support_phy_update_peripheral()
                        #support_subrating
                        .support_le_2m_phy()
                        #tx_power
                        .central_count(#num_peri)?
                        .peripheral_count(::rmk::ble::NUM_HOST_LINKS as u8)?
                        .buffer_cfg(SDC_PACKET_SIZE as u16, SDC_PACKET_SIZE as u16, SDC_TX_BUFFERS, SDC_RX_BUFFERS)?
                        .build(p, rng, mpsl, mem)
                    }
                }
                BoardConfig::UniBody(_) => quote! {
                    ::nrf_sdc::Builder::new()?
                    #support_adv
                    .support_peripheral()
                    .support_dle_peripheral()
                    .support_phy_update_peripheral()
                    .support_le_2m_phy()
                    #tx_power
                    #central_count
                    .peripheral_count(::rmk::ble::NUM_HOST_LINKS as u8)?
                    .buffer_cfg(SDC_PACKET_SIZE as u16, SDC_PACKET_SIZE as u16, SDC_TX_BUFFERS, SDC_RX_BUFFERS)?
                    .build(p, rng, mpsl, mem)
                },
            };

            quote! {
                use ::embassy_nrf::bind_interrupts;
                bind_interrupts!(struct Irqs {
                    #usb_and_clock_interrupt
                    #gpio_interrupts
                    RNG => ::embassy_nrf::rng::InterruptHandler<::embassy_nrf::peripherals::RNG>;
                    EGU0_SWI0 => ::nrf_sdc::mpsl::LowPrioInterruptHandler;
                    RADIO => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    TIMER0 => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    RTC0 => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    #pmw33xx_spi_interrupts
                    #ext_flash_spi_interrupt
                    #iqs5xx_interrupt
                    #display_interrupt
                    #extern_irqs
                });

                #[::embassy_executor::task]
                async fn mpsl_task(mpsl: &'static ::nrf_sdc::mpsl::MultiprotocolServiceLayer<'static>) -> ! {
                    mpsl.run().await
                }
                use ::rmk::ble::{SDC_PACKET_SIZE, SDC_RX_BUFFERS, SDC_TX_BUFFERS};
                fn build_sdc<'d, const N: usize>(
                    p: ::nrf_sdc::Peripherals<'d>,
                    rng: &'d mut ::embassy_nrf::rng::Rng<::embassy_nrf::mode::Async>,
                    mpsl: &'d ::nrf_sdc::mpsl::MultiprotocolServiceLayer,
                    mem: &'d mut ::nrf_sdc::Mem<N>,
                ) -> Result<::nrf_sdc::SoftdeviceController<'d>, ::nrf_sdc::Error> {
                    #nrf_sdc_config
                }
            }
        }
        rmk_config::resolved::hardware::ChipSeries::Rp2040 => {
            let usb_info = communication
                .get_usb_info()
                .expect("no usb info for the chip");
            let interrupt_name = format_ident!("{}", usb_info.interrupt_name);
            let peripheral_name = format_ident!("{}", usb_info.peripheral_name);
            // Pico W's cyw43 SPI needs a third DMA channel (separate rx); bind it only for BLE boards.
            let dma_ch2 = if communication.ble_enabled() {
                quote! { , ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::DMA_CH2> }
            } else {
                quote! {}
            };
            // DFU external SPI flash DMA channels
            let dfu_dma_channels = dfu
                .and_then(|d| d.external_flash.as_ref())
                .map(|ext| {
                    let tx = format_ident!(
                        "{}",
                        ext.spi
                            .tx_dma
                            .as_ref()
                            .expect("dfu.external_flash.spi.tx_dma is required for RP2040")
                    );
                    let rx = format_ident!(
                        "{}",
                        ext.spi
                            .rx_dma
                            .as_ref()
                            .expect("dfu.external_flash.spi.rx_dma is required for RP2040")
                    );
                    quote! {
                        , ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::#tx>
                        , ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::#rx>
                    }
                })
                .unwrap_or_default();
            let dma_irq_0 = quote! {
                DMA_IRQ_0 => ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::DMA_CH0>, ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::DMA_CH1> #dma_ch2 #dfu_dma_channels;
            };
            // For Pico W, enabled PIO0_IRQ_0 interrupt
            let (pio0_irq_0, ble_task) = if communication.ble_enabled() {
                (
                    quote! {
                        PIO0_IRQ_0 => ::embassy_rp::pio::InterruptHandler<::embassy_rp::peripherals::PIO0>;
                    },
                    quote! {
                        #[::embassy_executor::task]
                        async fn cyw43_task(runner: ::cyw43::Runner<'static, ::cyw43::SpiBus<::embassy_rp::gpio::Output<'static>, ::cyw43_pio::PioSpi<'static, ::embassy_rp::peripherals::PIO0, 0>>, ::cyw43::Cyw43439>) -> ! {
                            runner.run().await
                        }
                    },
                )
            } else {
                (quote! {}, quote! {})
            };
            quote! {
                use ::embassy_rp::bind_interrupts;
                bind_interrupts!(struct Irqs {
                    #interrupt_name => ::embassy_rp::usb::InterruptHandler<::embassy_rp::peripherals::#peripheral_name>;
                    #dma_irq_0
                    #pio0_irq_0
                    #gpio_interrupts
                    #iqs5xx_interrupt
                    #display_interrupt
                });
                #ble_task
            }
        }
        rmk_config::resolved::hardware::ChipSeries::Esp32 => quote! {},
    }
}

/// Bind the per-pin handlers needed by the active matrix and input devices.
pub(crate) fn expand_gpio_interrupts(
    chip: &ChipModel,
    matrix: &MatrixConfig,
    devices: Option<&InputDeviceConfig>,
    async_matrix: bool,
) -> TokenStream2 {
    if chip.series == ChipSeries::Esp32 {
        return TokenStream2::new();
    }

    let mut pins = BTreeSet::new();
    if async_matrix {
        match matrix.matrix_type {
            MatrixType::Normal => {
                let inputs = if matrix.row2col {
                    &matrix.col_pins
                } else {
                    &matrix.row_pins
                };
                pins.extend(inputs.iter().flatten().map(String::as_str));
            }
            MatrixType::DirectPin => {
                pins.extend(
                    matrix
                        .direct_pins
                        .iter()
                        .flatten()
                        .flatten()
                        .map(String::as_str)
                        .filter(|pin| *pin != "_" && !pin.eq_ignore_ascii_case("trns")),
                );
            }
        }
        if let Some(devices) = devices {
            for encoder in devices.encoder.iter().flatten() {
                pins.extend([encoder.pin_a.as_str(), encoder.pin_b.as_str()]);
            }
        }
    }
    if let Some(devices) = devices
        && matches!(chip.series, ChipSeries::Nrf52 | ChipSeries::Rp2040)
    {
        pins.extend(
            devices
                .pmw3610
                .iter()
                .flatten()
                .filter_map(|sensor| sensor.motion.as_deref()),
        );
        pins.extend(
            devices
                .pmw33xx
                .iter()
                .flatten()
                .filter_map(|sensor| sensor.motion.as_deref()),
        );
        pins.extend(
            devices
                .iqs5xx
                .iter()
                .flatten()
                .filter_map(|sensor| sensor.rdy.as_deref()),
        );
    }

    let mut interrupts = BTreeMap::<_, Vec<_>>::new();
    for pin in pins {
        let pin_ident = format_ident!("{pin}");
        let (interrupt, handler) = match chip.series {
            ChipSeries::Nrf52 => {
                let interrupt = if chip.chip.starts_with("nrf54") {
                    if pin.starts_with("P0_") {
                        "GPIOTE30_0"
                    } else {
                        "GPIOTE20_0"
                    }
                } else {
                    "GPIOTE"
                };
                (
                    format_ident!("{interrupt}"),
                    quote! { ::embassy_nrf::gpio::InterruptHandler<::embassy_nrf::peripherals::#pin_ident> },
                )
            }
            ChipSeries::Rp2040 => (
                format_ident!("IO_IRQ_BANK0"),
                quote! { ::embassy_rp::gpio::InterruptHandler<::embassy_rp::peripherals::#pin_ident> },
            ),
            ChipSeries::Stm32 => {
                let line = pin
                    .get(2..)
                    .and_then(|number| number.parse::<u8>().ok())
                    .expect("Invalid STM32 EXTI pin");
                let interrupt = match line {
                    0..=4 => format_ident!("EXTI{line}"),
                    5..=9 => format_ident!("EXTI9_5"),
                    10..=15 => format_ident!("EXTI15_10"),
                    _ => panic!("Invalid STM32 EXTI pin {pin}"),
                };
                let channel = format_ident!("EXTI{line}");
                (
                    interrupt,
                    quote! { ::embassy_stm32::exti::InterruptHandler<::embassy_stm32::peripherals::#channel> },
                )
            }
            ChipSeries::Esp32 => unreachable!(),
        };
        interrupts.entry(interrupt).or_default().push(handler);
    }
    interrupts
        .into_iter()
        .map(|(interrupt, handlers)| {
            quote! {
                #interrupt => #(#handlers),*;
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nrf_usb_codegen_has_no_ble_dependencies() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/use_config/nrf52840_ble/keyboard.toml");
        let config = rmk_config::KeyboardTomlConfig::new_from_toml_path(&path);
        let mut hardware = config.hardware().unwrap();
        hardware.communication = rmk_config::resolved::hardware::CommunicationConfig::Usb(
            hardware.communication.get_usb_info().unwrap(),
        );
        let module = syn::parse_quote!(
            mod keyboard {
                add_interrupt!(TEST => TestHandler;);
            }
        );
        let interrupts = bind_interrupt_default(&hardware, &module, None).to_string();
        let init = crate::codegen::chip::chip_init::chip_init_default(&hardware, None).to_string();
        assert!(interrupts.contains("HardwareVbusDetect") || interrupts.contains("vbus_detect"));
        assert!(interrupts.contains("TEST => TestHandler"));
        assert!(interrupts.contains("USBD"));
        for generated in [&interrupts, &init] {
            assert!(!generated.contains("nrf_sdc"));
            assert!(!generated.contains("mpsl"));
            assert!(!generated.contains("rmk :: ble"));
        }
        #[cfg(not(feature = "_dfu"))]
        {
            assert!(hardware.storage.is_some());
            let flash = crate::codegen::chip::flash::expand_flash_init(&hardware, None).to_string();
            assert!(flash.contains("Nvmc :: new"));
            assert!(flash.contains("async_flash_wrapper"));
            assert!(!flash.contains("mpsl"));
        }
    }

    #[test]
    fn nrf_ble_codegen_keeps_controller_and_flash() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../examples/use_config/nrf52840_ble/keyboard.toml");
        let hardware = rmk_config::KeyboardTomlConfig::new_from_toml_path(&path)
            .hardware()
            .unwrap();
        let interrupts = bind_interrupt_default(
            &hardware,
            &syn::parse_quote!(
                mod keyboard {}
            ),
            None,
        )
        .to_string();
        assert!(interrupts.contains("build_sdc"));
        assert!(interrupts.contains("ClockInterruptHandler"));
        assert!(interrupts.contains("vbus_detect"));
        #[cfg(not(feature = "_dfu"))]
        assert!(
            crate::codegen::chip::flash::expand_flash_init(&hardware, None)
                .to_string()
                .contains("nrf_mpsl :: Flash :: take")
        );
    }

    fn parse_fn(src: &str) -> ItemFn {
        syn::parse_str(src).expect("test fn should parse")
    }

    // Selection regression tests (#967 review): validation accepting a marker
    // is meaningless if selection doesn't recognize the same marker.

    #[test]
    fn documented_override_form_is_selected() {
        // The form the stm32h7 example documents. It used to pass validation
        // but never get selected, silently dropping the custom binding.
        assert!(is_bind_interrupt_override(&parse_fn(
            "#[Override(bind_interrupt)]\nfn bind_interrupt() {}"
        )));
    }

    #[test]
    fn overwritten_spelling_is_selected() {
        assert!(is_bind_interrupt_override(&parse_fn(
            "#[Overwritten(bind_interrupt)]\nfn bind_interrupt() {}"
        )));
    }

    #[test]
    fn legacy_bare_marker_is_still_selected() {
        // The original syntax; kept working for existing user code.
        assert!(is_bind_interrupt_override(&parse_fn(
            "#[bind_interrupt]\nfn bind_interrupt() {}"
        )));
    }

    #[test]
    fn doc_comment_does_not_disable_documented_form() {
        assert!(is_bind_interrupt_override(&parse_fn(
            "/// custom irq binding\n#[Override(bind_interrupt)]\nfn bind_interrupt() {}"
        )));
    }

    #[test]
    fn cfg_gated_override_is_not_selected() {
        // Same `cfg` semantics as every other override: the macro cannot
        // evaluate `cfg`, so the function is left unselected.
        assert!(!is_bind_interrupt_override(&parse_fn(
            "#[cfg(feature = \"x\")]\n#[Override(bind_interrupt)]\nfn bind_interrupt() {}"
        )));
    }

    #[test]
    fn other_override_marker_is_not_selected() {
        assert!(!is_bind_interrupt_override(&parse_fn(
            "#[Override(entry)]\nfn run() {}"
        )));
    }
}
