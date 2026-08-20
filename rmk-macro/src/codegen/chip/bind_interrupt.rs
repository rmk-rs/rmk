//! Add `bind_interrupts!` boilerplate of RMK, including USB or BLE
//!

use std::collections::HashSet;

use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::{BoardConfig, InputDeviceConfig, UniBodyConfig};
use syn::{ItemFn, ItemMod};

use crate::codegen::display::expand_display_interrupt;
use crate::codegen::feature::{get_rmk_features, is_feature_enabled};
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
pub(crate) fn expand_bind_interrupt(hardware: &Hardware, item_mod: &ItemMod) -> TokenStream2 {
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
            .unwrap_or(bind_interrupt_default(hardware, item_mod))
    } else {
        bind_interrupt_default(hardware, item_mod)
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

/// Expand default `bind_interrupt!` for different chips and nrf-sdc config for nRF52
pub(crate) fn bind_interrupt_default(hardware: &Hardware, item_mod: &ItemMod) -> TokenStream2 {
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

    match chip.series {
        rmk_config::resolved::hardware::ChipSeries::Stm32 => {
            // For stm32, bind USB interrupt and EXTI interrupts (if async_matrix is enabled)
            let rmk_features = get_rmk_features();
            let async_matrix = is_feature_enabled(&rmk_features, "async_matrix");

            // Generate EXTI interrupt bindings for async_matrix
            let exti_interrupts = if async_matrix {
                generate_stm32_exti_interrupts(board)
            } else {
                quote! {}
            };

            if let Some(usb_info) = communication.get_usb_info() {
                let interrupt_name = format_ident!("{}", usb_info.interrupt_name);
                let peripheral_name = format_ident!("{}", usb_info.peripheral_name);
                quote! {
                    use ::embassy_stm32::bind_interrupts;
                    bind_interrupts!(struct Irqs {
                        #interrupt_name => ::embassy_stm32::usb::InterruptHandler<::embassy_stm32::peripherals::#peripheral_name>;
                        #exti_interrupts
                        #display_interrupt
                        #extern_irqs
                    });
                }
            } else if async_matrix {
                quote! {
                    use ::embassy_stm32::bind_interrupts;
                    bind_interrupts!(struct Irqs {
                        #exti_interrupts
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
            let support_subrating = if is_feature_enabled(&get_rmk_features(), "subrating") {
                quote! { .support_connection_subrating_central() }
            } else {
                quote! {}
            };
            let tx_power = if let Some(pwr) = ble_config.default_tx_power {
                quote! { .default_tx_power(#pwr)?  }
            } else {
                quote! {}
            };
            let use_2m_phy = if ble_config.use_2m_phy.unwrap_or(true) {
                quote! { .support_le_2m_phy() }
            } else {
                quote! {}
            };

            // nrf-sdc interrupt config
            let nrf_sdc_config = match &board {
                BoardConfig::Split(_) => {
                    let num_peri = board.get_num_peripheral() as u8;
                    quote! {
                        ::nrf_sdc::Builder::new()?
                        .support_scan()
                        .support_central()
                        .support_adv()
                        .support_peripheral()
                        .support_dle_peripheral()
                        .support_dle_central()
                        .support_phy_update_central()
                        .support_phy_update_peripheral()
                        #support_subrating
                        #use_2m_phy
                        #tx_power
                        .central_count(#num_peri)?
                        .peripheral_count(1)?
                        .buffer_cfg(L2CAP_MTU as u16, L2CAP_MTU as u16, L2CAP_TXQ, L2CAP_RXQ)?
                        .build(p, rng, mpsl, mem)
                    }
                }
                BoardConfig::UniBody(_) => quote! {
                    ::nrf_sdc::Builder::new()?
                    .support_adv()
                    .support_peripheral()
                    .support_dle_peripheral()
                    .support_phy_update_peripheral()
                    #support_subrating
                    #use_2m_phy
                    #tx_power
                    .peripheral_count(1)?
                    .buffer_cfg(L2CAP_MTU as u16, L2CAP_MTU as u16, L2CAP_TXQ, L2CAP_RXQ)?
                    .build(p, rng, mpsl, mem)
                },
            };

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

            quote! {
                use ::embassy_nrf::bind_interrupts;
                bind_interrupts!(struct Irqs {
                    #usb_and_clock_interrupt
                    RNG => ::embassy_nrf::rng::InterruptHandler<::embassy_nrf::peripherals::RNG>;
                    EGU0_SWI0 => ::nrf_sdc::mpsl::LowPrioInterruptHandler;
                    RADIO => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    TIMER0 => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    RTC0 => ::nrf_sdc::mpsl::HighPrioInterruptHandler;
                    #pmw33xx_spi_interrupts
                    #iqs5xx_interrupt
                    #display_interrupt
                    #extern_irqs
                });

                #[::embassy_executor::task]
                async fn mpsl_task(mpsl: &'static ::nrf_sdc::mpsl::MultiprotocolServiceLayer<'static>) -> ! {
                    mpsl.run().await
                }
                /// How many outgoing L2CAP buffers per link
                const L2CAP_TXQ: u8 = 3;

                /// How many incoming L2CAP buffers per link
                const L2CAP_RXQ: u8 = 3;

                /// Size of L2CAP packets
                const L2CAP_MTU: usize = 251;
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
            let dma_irq_0 = quote! {
                DMA_IRQ_0 => ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::DMA_CH0>, ::embassy_rp::dma::InterruptHandler<::embassy_rp::peripherals::DMA_CH1> #dma_ch2;
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
                    #iqs5xx_interrupt
                    #display_interrupt
                });
                #ble_task
            }
        }
        rmk_config::resolved::hardware::ChipSeries::Esp32 => quote! {},
    }
}

/// Generate STM32 EXTI interrupt bindings based on row pins
/// STM32 EXTI lines:
/// - EXTI0 - EXTI4: each has its own interrupt
/// - EXTI5 - EXTI9: share EXTI9_5 interrupt
/// - EXTI10 - EXTI15: share EXTI15_10 interrupt
fn generate_stm32_exti_interrupts(board: &BoardConfig) -> TokenStream2 {
    // Collect all row pins from the matrix configuration
    let row_pins: Vec<String> = match board {
        BoardConfig::UniBody(unibody) => unibody.matrix.row_pins.clone().unwrap_or_default(),
        BoardConfig::Split(split) => split.central.matrix.row_pins.clone().unwrap_or_default(),
    };

    // Extract pin numbers and determine required EXTI interrupts
    let mut required_interrupts: HashSet<String> = HashSet::new();

    for pin in &row_pins {
        if let Some(pin_num_str) = get_pin_num_stm32(pin)
            && let Ok(pin_num) = pin_num_str.parse::<u8>()
        {
            let interrupt_name = match pin_num {
                0 => "EXTI0",
                1 => "EXTI1",
                2 => "EXTI2",
                3 => "EXTI3",
                4 => "EXTI4",
                5..=9 => "EXTI9_5",
                10..=15 => "EXTI15_10",
                _ => continue,
            };
            required_interrupts.insert(interrupt_name.to_string());
        }
    }

    // Generate interrupt bindings
    let interrupt_bindings: Vec<TokenStream2> = required_interrupts
        .iter()
        .map(|irq_name| {
            let irq_ident = format_ident!("{}", irq_name);
            quote! {
                #irq_ident => ::embassy_stm32::exti::InterruptHandler<::embassy_stm32::interrupt::typelevel::#irq_ident>;
            }
        })
        .collect();

    quote! {
        #(#interrupt_bindings)*
    }
}

/// Get pin number from pin str.
/// For example, if the pin str is "PD13", this function will return "13".
fn get_pin_num_stm32(gpio_name: &str) -> Option<String> {
    if gpio_name.len() < 3 {
        None
    } else {
        Some(gpio_name[2..].to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
