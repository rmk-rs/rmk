//! Unified Runnable trait implementation generator.
//!
//! Generates `Runnable` implementations for structs that combine
//! input_device and processor behaviors.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};

use crate::event_macros::config::InputDeviceConfig;
use crate::event_macros::utils::deduplicate_type_generics;
use crate::processor::ProcessorConfig;

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
        let publish_ident = publish_type.segments.last().map(|s| &s.ident);
        if proc_cfg
            .event_types
            .iter()
            .any(|path| path.segments.last().map(|s| &s.ident) == publish_ident)
        {
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
                ::rmk::processor::DeadlineProcessor::deadline_loop(self).await
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

    let (Some(device_config), Some(processor_config)) = (input_device_config, processor_config)
    else {
        unreachable!("Runnable generation requires an input device or processor");
    };
    let enum_name = format_ident!("__RmkSelectEvent{}", struct_name);
    let input_type = &device_config.event_type;
    let proc_type = match processor_config.event_types.as_slice() {
        [] => quote! { ::core::convert::Infallible },
        [event_type] => quote! { #event_type },
        _ => {
            let proc_enum_name = format_ident!("{}ProcessorEventEnum", struct_name);
            quote! { #proc_enum_name }
        }
    };
    let mut select_arms = vec![
        quote! { event = self.read_event().fuse() => #enum_name::Input(event) },
        quote! { proc_event = proc_sub.next_event().fuse() => #enum_name::Processor(proc_event) },
    ];
    let mut match_arms = vec![
        quote! { #enum_name::Input(event) => { publish_event_async(event).await; } },
        quote! { #enum_name::Processor(event) => { <Self as ::rmk::processor::Processor>::process(self, event).await; } },
    ];
    let timer_variant = has_polling.then(|| quote! { Timer, });
    let timer_init = processor_config.poll_interval_ms.map(|interval_ms| {
        quote! {
            let mut ticker = ::embassy_time::Ticker::every(
                ::embassy_time::Duration::from_millis(#interval_ms)
            );
        }
    });
    if has_polling {
        select_arms.insert(0, quote! { _ = ticker.next().fuse() => #enum_name::Timer });
        match_arms.push(quote! {
            #enum_name::Timer => { <Self as ::rmk::processor::PollingProcessor>::update(self).await; }
        });
    }
    wrap_runnable(quote! {
        use ::rmk::event::publish_event_async;
        use ::rmk::input_device::InputDevice;
        use ::rmk::event::EventSubscriber;
        use ::rmk::futures::FutureExt;
        enum #enum_name {
            Input(#input_type),
            Processor(#proc_type),
            #timer_variant
        }
        let mut proc_sub = <Self as ::rmk::processor::Processor>::subscriber();
        #timer_init
        loop {
            let select_result = { ::rmk::futures::select_biased! { #(#select_arms),* } };
            match select_result { #(#match_arms)* }
        }
    })
}
