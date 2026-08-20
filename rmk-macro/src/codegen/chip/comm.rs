//! Initialize communication boilerplate of RMK, including USB or BLE
//!

use proc_macro2::TokenStream as TokenStream2;
use quote::{ToTokens, format_ident, quote};
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::ChipSeries;
use syn::{ItemFn, ItemMod};

use crate::codegen::override_helper::{Overwritten, find_overwritten};

pub(crate) fn expand_usb_init(hardware: &Hardware, item_mod: &ItemMod) -> TokenStream2 {
    // If there is a function with `#[Overwritten(usb)]`, override the chip initialization
    if let Some((_, items)) = &item_mod.content {
        items
            .iter()
            .find_map(|item| {
                if let syn::Item::Fn(item_fn) = &item
                    && let Some(Ok(Overwritten::Usb)) = find_overwritten(item_fn)
                {
                    return Some(override_usb_init(item_fn));
                }
                None
            })
            .unwrap_or(usb_config_default(hardware))
    } else {
        usb_config_default(hardware)
    }
}

/// Default implementation of usb initialization
pub(crate) fn usb_config_default(hardware: &Hardware) -> TokenStream2 {
    if let Some(usb_info) = hardware.communication.get_usb_info() {
        let peripheral_name = format_ident!("{}", usb_info.peripheral_name);
        match hardware.chip.series {
            ChipSeries::Stm32 => {
                let dp = format_ident!("{}", usb_info.dp);
                let dm = format_ident!("{}", usb_info.dm);
                if usb_info.peripheral_name.contains("OTG") {
                    quote! {
                        static EP_OUT_BUFFER: ::static_cell::StaticCell<[u8; 1024]> = ::static_cell::StaticCell::new();
                        let mut usb_config = ::embassy_stm32::usb::Config::default();
                        usb_config.vbus_detection = false;
                        let driver = ::embassy_stm32::usb::Driver::new_fs(
                            p.#peripheral_name,
                            Irqs,
                            p.#dp,
                            p.#dm,
                            &mut EP_OUT_BUFFER.init([0; 1024])[..],
                            usb_config,
                        );
                    }
                } else {
                    quote! {
                        {
                            let _dp = ::embassy_stm32::gpio::Output::new(p.#dp.reborrow(), ::embassy_stm32::gpio::Level::Low, ::embassy_stm32::gpio::Speed::Low);
                            ::embassy_time::Timer::after_millis(10).await;
                        }
                        // Usb driver
                        let driver = ::embassy_stm32::usb::Driver::new(p.#peripheral_name, Irqs, p.#dp, p.#dm);
                    }
                }
            }
            ChipSeries::Nrf52 => quote! {
                // use hardware vbus
                let driver = ::embassy_nrf::usb::Driver::new(p.#peripheral_name, Irqs, ::embassy_nrf::usb::vbus_detect::HardwareVbusDetect::new(Irqs));
            },
            ChipSeries::Rp2040 => quote! {
                let driver = ::embassy_rp::usb::Driver::new(p.#peripheral_name, Irqs);
            },
            ChipSeries::Esp32 => {
                let dp = format_ident!("{}", usb_info.dp);
                let dm = format_ident!("{}", usb_info.dm);
                quote! {
                    static mut EP_MEMORY: [u8; 1024] = [0; 1024];
                    let usb = ::esp_hal::usb::otg::Usb::new_fs(p.#peripheral_name, p.#dp, p.#dm);
                    let usb_config = ::esp_hal::usb::otg::embassy_usb_device::Config::default();
                    let driver = ::esp_hal::usb::otg::embassy_usb_device::Driver::new(usb, unsafe { &mut *core::ptr::addr_of_mut!(EP_MEMORY) }, usb_config);
                }
            }
        }
    } else {
        quote! {}
    }
}

fn override_usb_init(item_fn: &ItemFn) -> TokenStream2 {
    let initialization = item_fn.block.to_token_stream();
    quote! {
        let driver = #initialization;
    }
}
