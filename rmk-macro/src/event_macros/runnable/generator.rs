//! Unified Runnable trait implementation generator.
//!
//! Generates `Runnable` implementations for structs that combine
//! input_device and processor behaviors.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::event_macros::config::InputDeviceConfig;
use crate::event_macros::utils::deduplicate_type_generics;
use crate::processor::ProcessorConfig;

/// Build the loop body for select with optional match.
fn build_select_loop_body(
    select_arms: &[TokenStream],
    match_arms: Option<&[TokenStream]>,
) -> TokenStream {
    let select_block = quote! {
        ::rmk::futures::select_biased! {
            #(#select_arms),*
        }
    };

    match match_arms {
        Some(arms) => quote! {
            let select_result = { #select_block };
            match select_result {
                #(#arms)*
            }
        },
        None => select_block,
    }
}

/// Check if a path matches any path in a list (by last segment ident).
fn path_in_list(needle: &syn::Path, haystack: &[syn::Path]) -> bool {
    let needle_ident = needle.segments.last().map(|s| &s.ident);
    haystack
        .iter()
        .any(|p| p.segments.last().map(|s| &s.ident) == needle_ident)
}

/// Generate a unified `Runnable` impl for input_device and/or processor.
///
/// Handles:
/// - InputDevice: read_event + publish
/// - Processor: subscribe + process + optional polling
///
/// Uses select_biased! when multiplexing multiple sources.
///
/// # Panics
/// Panics at compile time if the same event type is both published and subscribed,
/// which would cause a self-deadlock.
pub fn generate_runnable(
    struct_name: &syn::Ident,
    generics: &syn::Generics,
    where_clause: Option<&syn::WhereClause>,
    input_device_config: Option<&InputDeviceConfig>,
    processor_config: Option<&ProcessorConfig>,
) -> TokenStream {
    // Check for self-deadlock: published event type must not be in subscribe list of the same struct
    if let (Some(input_cfg), Some(proc_cfg)) = (input_device_config, processor_config) {
        let publish_type = &input_cfg.event_type;
        if path_in_list(publish_type, &proc_cfg.event_types) {
            panic!(
                "Self-deadlock detected on `{}`: cannot publish and subscribe the same event type `{}`. \
                The task would block on publish_event_async() while the only consumer is in the same blocked task.",
                struct_name,
                quote! { #publish_type }
            );
        }
    }

    let has_polling = processor_config.is_some_and(|c| c.poll_interval_ms.is_some());
    let has_deadline = processor_config.is_some_and(|c| c.deadline);
    if has_deadline && input_device_config.is_some() {
        return syn::Error::new_spanned(
            struct_name,
            "a `deadline` processor cannot also be an #[input_device]",
        )
        .to_compile_error();
    }

    let (impl_generics, _, _) = generics.split_for_impl();
    let ty_generics = deduplicate_type_generics(generics);

    // Helper to wrap body in Runnable impl
    let wrap_runnable = |body: TokenStream| {
        quote! {
            impl #impl_generics ::rmk::core_traits::Runnable for #struct_name #ty_generics #where_clause {
                async fn run(&mut self) -> ! {
                    #body
                }
            }
        }
    };

    // Standalone processor
    if input_device_config.is_none() && processor_config.is_some() {
        return wrap_runnable(if has_deadline && has_polling {
            quote! {
                ::rmk::processor::DeadlineProcessor::polling_deadline_loop(self).await
            }
        } else if has_deadline {
            quote! {
                use ::rmk::processor::DeadlineProcessor;
                self.deadline_loop().await
            }
        } else if has_polling {
            quote! {
                use ::rmk::processor::PollingProcessor;
                self.polling_loop().await
            }
        } else {
            quote! {
                use ::rmk::processor::Processor;
                self.process_loop().await
            }
        });
    }

    // Standalone input_device
    if input_device_config.is_some() && processor_config.is_none() {
        return wrap_runnable(quote! {
            use ::rmk::event::publish_event_async;
            use ::rmk::input_device::InputDevice;
            loop {
                let event = self.read_event().await;
                publish_event_async(event).await;
            }
        });
    }

    // Collect select arms and subscriber definitions.
    let mut sub_defs: Vec<TokenStream> = Vec::new();
    let mut select_arms: Vec<TokenStream> = Vec::new();
    let mut select_match_arms: Vec<TokenStream> = Vec::new();
    let mut use_statements: Vec<TokenStream> = Vec::new();

    let needs_split_select = input_device_config.is_some() && processor_config.is_some();
    let select_enum_name =
        needs_split_select.then(|| format_ident!("__RmkSelectEvent{}", struct_name));
    let mut input_event_type: Option<syn::Path> = None;

    // Handle input_device.
    if let Some(device_config) = input_device_config {
        input_event_type = Some(device_config.event_type.clone());
        use_statements.push(quote! { use ::rmk::event::publish_event_async; });
        use_statements.push(quote! { use ::rmk::input_device::InputDevice; });

        if let Some(ref enum_name) = select_enum_name {
            select_arms.push(quote! {
                event = self.read_event().fuse() => #enum_name::Input(event)
            });
            select_match_arms.push(quote! {
                #enum_name::Input(event) => {
                    publish_event_async(event).await;
                }
            });
        } else {
            select_arms.push(quote! {
                event = self.read_event().fuse() => {
                    publish_event_async(event).await;
                }
            });
        }
    }

    if processor_config.is_some() {
        use_statements.push(quote! { use ::rmk::event::SubscribableEvent; });
        use_statements.push(quote! { use ::rmk::processor::Processor; });

        // Use the Processor trait's subscriber method
        // The processor macro has already generated the event enum and Processor trait impl
        sub_defs.push(quote! {
            let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
        });

        if let Some(ref enum_name) = select_enum_name {
            select_arms.push(quote! {
                proc_event = proc_sub.next_event().fuse() => #enum_name::Processor(proc_event)
            });
        } else {
            select_arms.push(quote! {
                proc_event = proc_sub.next_event().fuse() => {
                    <Self as ::rmk::processor::Processor>::process(self, proc_event).await;
                }
            });
        }

        if let Some(ref enum_name) = select_enum_name {
            select_match_arms.push(quote! {
                #enum_name::Processor(event) => {
                    <Self as ::rmk::processor::Processor>::process(self, event).await;
                }
            });
        }
    }

    // === Combined cases ===

    // Add common use statements
    if !sub_defs.is_empty() {
        use_statements.push(quote! { use ::rmk::event::EventSubscriber; });
    }
    use_statements.push(quote! { use ::rmk::futures::FutureExt; });

    // Build select enum definition if needed
    let select_enum_def = select_enum_name.as_ref().and_then(|enum_name| {
        let input_type = input_event_type.as_ref()?;
        // Determine the processor event type
        // If processor has only one event type, use it directly (no enum generated)
        // Otherwise, use the generated enum name {StructName}ProcessorEventEnum
        let proc_type = if let Some(proc_cfg) = processor_config {
            if proc_cfg.event_types.is_empty() {
                quote! { ::core::convert::Infallible }
            } else if proc_cfg.event_types.len() == 1 {
                let event_type = &proc_cfg.event_types[0];
                quote! { #event_type }
            } else {
                let proc_enum_name = format_ident!("{}ProcessorEventEnum", struct_name);
                quote! { #proc_enum_name }
            }
        } else {
            // This shouldn't happen in combined mode, but provide a fallback
            quote! { () }
        };

        Some(if has_polling {
            quote! {
                enum #enum_name {
                    Input(#input_type),
                    Processor(#proc_type),
                    Timer,
                }
            }
        } else {
            quote! {
                enum #enum_name {
                    Input(#input_type),
                    Processor(#proc_type),
                }
            }
        })
    });

    if has_polling {
        use_statements.push(quote! { use ::rmk::processor::PollingProcessor; });

        let timer_arm = if let Some(ref enum_name) = select_enum_name {
            select_match_arms.push(quote! {
                #enum_name::Timer => {
                    <Self as ::rmk::processor::PollingProcessor>::update(self).await;
                }
            });
            quote! { _ = ticker.next().fuse() => #enum_name::Timer }
        } else {
            quote! {
                _ = ticker.next().fuse() => {
                    <Self as ::rmk::processor::PollingProcessor>::update(self).await;
                }
            }
        };
        // Keep polling ahead of input and processor events when several sources are ready.
        select_arms.insert(0, timer_arm);
    }
    let loop_body = build_select_loop_body(
        &select_arms,
        needs_split_select.then_some(select_match_arms.as_slice()),
    );

    // Build timer init if polling
    let timer_init = processor_config.as_ref().and_then(|config| {
        config.poll_interval_ms.map(|interval_ms| {
            quote! {
                let mut ticker = ::embassy_time::Ticker::every(
                    ::embassy_time::Duration::from_millis(#interval_ms)
                );
            }
        })
    });

    // Assemble final output
    wrap_runnable(quote! {
        #(#use_statements)*
        #select_enum_def
        #(#sub_defs)*
        #timer_init
        loop {
            #loop_body
        }
    })
}
