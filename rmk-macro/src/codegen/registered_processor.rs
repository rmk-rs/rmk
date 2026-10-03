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

            match expand_custom_processor(item_fn, attr) {
                Ok((custom_init, executor)) => {
                    initializers.extend(custom_init);
                    executors.push(executor);
                }
                Err(err) => initializers.extend(err.to_compile_error()),
            }
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

fn expand_custom_processor(
    fn_item: &syn::ItemFn,
    attr: &syn::Attribute,
) -> syn::Result<(TokenStream, TokenStream)> {
    let task_name = &fn_item.sig.ident;
    let mut cfg_attrs = Vec::new();
    for attr in &fn_item.attrs {
        if let Some(gate) = cfg_gate(&attr.meta)? {
            cfg_attrs.push(quote! { #[#gate] });
        }
    }

    let content = &fn_item.block.stmts;
    let initializer = quote! {
        #(#cfg_attrs)*
        let mut #task_name = {
            #(#content)*
        };
    };
    let executor = registered_processor_executor(attr, task_name);
    let executor = if cfg_attrs.is_empty() {
        executor
    } else {
        // The disabled branch must not refer to a processor type or value that may not exist.
        quote! {{
            let __rmk_processor_task = ::core::future::pending::<()>();
            #(#cfg_attrs)*
            let __rmk_processor_task = #executor;
            __rmk_processor_task
        }}
    };

    Ok((initializer, executor))
}

/// Keep conditional-compilation gates without copying function-only attributes onto let bindings.
fn cfg_gate(meta: &syn::Meta) -> syn::Result<Option<syn::Meta>> {
    if meta.path().is_ident("cfg") {
        return Ok(Some(meta.clone()));
    }
    if !meta.path().is_ident("cfg_attr") {
        return Ok(None);
    }
    let syn::Meta::List(list) = meta else {
        return Err(syn::Error::new_spanned(
            meta,
            "expected cfg_attr(predicate, attribute)",
        ));
    };
    let args = list.parse_args_with(
        syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
    )?;
    if args.len() < 2 {
        return Err(syn::Error::new_spanned(
            meta,
            "expected cfg_attr(predicate, attribute)",
        ));
    }
    let mut args = args.iter();
    let predicate = args.next().unwrap();
    let mut gates = Vec::new();
    for attr in args {
        if let Some(gate) = cfg_gate(attr)? {
            gates.push(gate);
        }
    }
    Ok(if gates.is_empty() {
        None
    } else {
        Some(syn::parse_quote!(cfg_attr(#predicate, #(#gates),*)))
    })
}

/// Registration always runs the behavior declared by the type.
fn registered_processor_executor(attr: &syn::Attribute, exec: &syn::Ident) -> TokenStream {
    if !matches!(&attr.meta, syn::Meta::Path(_))
        && !matches!(&attr.meta, syn::Meta::List(list) if list.tokens.is_empty())
    {
        return syn::Error::new_spanned(
            attr,
            "#[register_processor] takes no arguments; set `poll_interval` or `deadline` on #[processor]",
        )
        .to_compile_error();
    }

    quote! {
        ::rmk::core_traits::Runnable::run(&mut #exec)
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use quote::quote;

    use super::{expand_custom_processor, registered_processor_executor};

    /// The executor generated for `attr` on a registered function named `leds`.
    fn executor(attr: &str) -> String {
        let item: syn::ItemFn =
            syn::parse_str(&format!("{attr} fn leds() -> Leds {{ Leds::new() }}")).unwrap();
        registered_processor_executor(&item.attrs[0], &item.sig.ident).to_string()
    }

    #[test]
    fn registration_arguments_report_how_to_migrate() {
        for attr in [
            "#[register_processor(event)]",
            "#[register_processor(poll)]",
            "#[register_processor(deadline)]",
            "#[register_processor(event, deadline)]",
            "#[register_processor = true]",
        ] {
            let error = executor(attr);
            assert!(error.contains("compile_error"), "{error}");
            assert!(error.contains("takes no arguments"), "{error}");
            assert!(error.contains("on #[processor]"), "{error}");
        }
    }

    #[test]
    fn bare_registration_runs_the_types_runnable() {
        let expected = quote! { ::rmk::core_traits::Runnable::run(&mut leds) };
        for attr in ["#[register_processor]", "#[register_processor()]"] {
            assert_eq!(executor(attr), expected.to_string());
        }
    }

    #[test]
    fn conditional_registration_gates_initialization_and_execution() {
        let module: syn::ItemMod = syn::parse_quote! {
            mod keyboard {
                #[cfg(all())]
                #[register_processor]
                fn enabled() -> Worker { Worker::new() }

                #[cfg(any())]
                #[register_processor]
                fn disabled() -> Missing { must_not_compile() }

                #[cfg(all())]
                #[cfg(any())]
                #[register_processor]
                fn multiple_gates() -> Missing { must_not_compile() }

                #[cfg_attr(all(), cfg_attr(all(), cfg(any())), inline)]
                #[register_processor]
                fn nested_disabled() -> Missing { must_not_compile() }

                #[cfg_attr(any(), cfg(any()))]
                #[register_processor]
                fn inactive_cfg_attr() -> Worker { Worker::new() }

                #[cfg_attr(all(), inline)]
                #[register_processor]
                fn function_attribute() -> Worker { Worker::new() }

                #[cfg(all())]
                #[cfg_attr(all(), cfg(all()))]
                #[register_processor]
                fn all_enabled() -> Worker { Worker::new() }
            }
        };
        let mut initializers = Vec::new();
        let mut polls = Vec::new();
        for item in &module.content.unwrap().1 {
            let syn::Item::Fn(item) = item else {
                unreachable!()
            };
            let registration = item
                .attrs
                .iter()
                .find(|a| a.path().is_ident("register_processor"))
                .unwrap();
            let (init, exec) = expand_custom_processor(item, registration).unwrap();
            initializers.push(init);
            polls.push(quote! {
                let mut task = ::std::pin::pin!(#exec);
                assert!(::std::future::Future::poll(task.as_mut(), &mut context).is_pending());
            });
        }
        let source = quote! {
            extern crate self as rmk;
            use std::sync::atomic::{AtomicUsize, Ordering};
            static INITIALIZED: AtomicUsize = AtomicUsize::new(0);
            static STARTED: AtomicUsize = AtomicUsize::new(0);
            mod core_traits {
                pub(crate) trait Runnable {
                    async fn run(&mut self) -> !;
                }
            }
            struct Worker;
            impl Worker {
                fn new() -> Self {
                    INITIALIZED.fetch_add(1, Ordering::Relaxed);
                    Self
                }
            }
            impl core_traits::Runnable for Worker {
                async fn run(&mut self) -> ! {
                    STARTED.fetch_add(1, Ordering::Relaxed);
                    core::future::pending().await
                }
            }
            fn main() {
                #(#initializers)*
                assert_eq!(INITIALIZED.load(Ordering::Relaxed), 4);
                assert_eq!(STARTED.load(Ordering::Relaxed), 0);
                let mut context = std::task::Context::from_waker(std::task::Waker::noop());
                #(#polls)*
                assert_eq!(STARTED.load(Ordering::Relaxed), 4);
            }
        };
        let dir = std::env::temp_dir().join(format!(
            "rmk-processor-cfg-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        std::fs::create_dir(&dir).unwrap();
        let input = dir.join("main.rs");
        let binary = dir.join(format!("probe{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&input, source.to_string()).unwrap();
        let compiled = Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
            .args(["--edition=2024", "--crate-name=cfg_registration"])
            .arg(&input)
            .arg("-o")
            .arg(&binary)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let executed = Command::new(&binary).output().unwrap();
        assert!(
            executed.status.success(),
            "{}",
            String::from_utf8_lossy(&executed.stderr)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
