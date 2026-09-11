use proc_macro2::TokenStream;
use quote::quote;
use rmk_config::resolved::Hardware;
use rmk_config::resolved::hardware::{ChipModel, PinConfig};
use syn::ItemMod;

use super::chip::gpio::convert_gpio_str_to_output_pin;

/// Expand processor init/exec blocks from keyboard config.
/// Returns (initializers, executors).
pub(crate) fn expand_registered_processor_init(
    hardware: &Hardware,
    item_mod: &ItemMod,
) -> (TokenStream, Vec<TokenStream>) {
    let mut initializers = TokenStream::new();
    let mut executors = vec![];

    let (i, e) = expand_light_indicator_processors(hardware);
    initializers.extend(i);
    executors.extend(e);

    if cfg!(feature = "_dfu") {
        create_dfu_led_processor(hardware, &mut initializers, &mut executors);
    }

    // Custom processors declared in the module.
    if let Some((_, items)) = &item_mod.content {
        for item in items {
            let syn::Item::Fn(item_fn) = item else {
                continue;
            };
            let Some(attr) = item_fn
                .attrs
                .iter()
                .find(|a| a.path().is_ident("register_processor"))
            else {
                continue;
            };

            let (custom_init, custom_exec) = expand_custom_processor(item_fn);
            let mut mode: Option<bool> = None; // Some(true) = event, Some(false) = poll

            attr.parse_nested_meta(|meta| {
                let is_event = meta.path.is_ident("event");
                let is_poll = meta.path.is_ident("poll");

                if !is_event && !is_poll {
                    return Err(meta.error("expected `event` or `poll`"));
                }
                if mode.is_some() {
                    return Err(meta.error("cannot specify multiple modes"));
                }
                mode = Some(is_event);
                Ok(())
            })
            .unwrap_or_else(|e| panic!("#[register_processor] {e}"));

            let executor = match mode {
                Some(true) => quote! {
                    async { use ::rmk::processor::Processor; #custom_exec.process_loop().await }
                },
                Some(false) => quote! {
                    async { use ::rmk::processor::PollingProcessor; #custom_exec.polling_loop().await }
                },
                None => panic!("#[register_processor] requires `event` or `poll` argument"),
            };

            initializers.extend(custom_init);
            executors.push(executor);
        }
    }

    (initializers, executors)
}

fn expand_light_indicator_processors(hardware: &Hardware) -> (TokenStream, Vec<TokenStream>) {
    let chip = &hardware.chip;
    let light_config = &hardware.light;

    let mut initializers = TokenStream::new();
    let mut executors = vec![];

    create_keyboard_indicator_processor(
        chip,
        &light_config.numslock,
        quote! { numlock_processor },
        quote! { NumLock },
        &mut initializers,
        &mut executors,
    );

    create_keyboard_indicator_processor(
        chip,
        &light_config.scrolllock,
        quote! { scrolllock_processor },
        quote! { ScrollLock },
        &mut initializers,
        &mut executors,
    );

    create_keyboard_indicator_processor(
        chip,
        &light_config.capslock,
        quote! { capslock_processor },
        quote! { CapsLock },
        &mut initializers,
        &mut executors,
    );

    (initializers, executors)
}

fn create_keyboard_indicator_processor(
    chip: &ChipModel,
    pin_config: &Option<PinConfig>,
    processor_ident: TokenStream,
    led_indicator_variant: TokenStream,
    initializers: &mut TokenStream,
    executors: &mut Vec<TokenStream>,
) {
    if let Some(c) = pin_config {
        let p = convert_gpio_str_to_output_pin(chip, c.pin.clone(), c.low_active);
        let low_active = c.low_active;
        let processor_init = quote! {
            let mut #processor_ident = ::rmk::processor::builtin::led_indicator::KeyboardIndicatorProcessor::new(
                #p,
                #low_active,
                ::rmk::types::led_indicator::LedIndicatorType::#led_indicator_variant,
            );
        };
        initializers.extend(processor_init);
        executors.push(quote! { #processor_ident.run() });
    }
}

fn create_dfu_led_processor(
    hardware: &Hardware,
    initializers: &mut TokenStream,
    executors: &mut Vec<TokenStream>,
) {
    use rmk_config::resolved::hardware::ChipSeries;
    let chip = &hardware.chip;
    if let Some(dfu) = &hardware.dfu {
        let pin_str = match &dfu.led {
            Some(c) if c.pin == "none" => return,
            Some(c) => c.pin.clone(),
            None => match chip.series {
                ChipSeries::Nrf52 => "P0_15".to_string(),
                ChipSeries::Rp2040 => "PIN_25".to_string(),
                _ => return,
            },
        };
        let p = convert_gpio_str_to_output_pin(chip, pin_str, false);
        let processor_init = quote! {
            let mut dfu_led_processor = ::rmk::processor::builtin::dfu_led::DfuLedProcessor::new(
                #p,
                false,
            );
        };
        initializers.extend(processor_init);
        executors.push(quote! { dfu_led_processor.run() });
    }
}

fn expand_custom_processor(fn_item: &syn::ItemFn) -> (TokenStream, &syn::Ident) {
    let task_name = &fn_item.sig.ident;

    let content = &fn_item.block.stmts;
    let initializer = quote! {
        let mut #task_name = {
            #(#content)*
        };
    };

    (initializer, task_name)
}
