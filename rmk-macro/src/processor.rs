//! Unified processor macro implementation.
//!
//! Generates `Processor` trait implementations for event-driven processors.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Meta, parse_macro_input};

use crate::event_macros::runnable::{generate_event_enum_and_dispatch, generate_runnable};
use crate::event_macros::utils::{AttributeParser, attr_matches_name, has_runnable_marker};

/// Processor subscription config.
#[derive(Default)]
pub struct ProcessorConfig {
    pub event_types: Vec<syn::Path>,
    pub poll_interval_ms: Option<u64>,
    /// Enable dynamic deadlines alongside event handling and optional polling.
    pub deadline: bool,
}

impl ProcessorConfig {
    pub fn validate_runnable(
        &self,
        struct_name: &syn::Ident,
        has_marker: bool,
    ) -> Result<(), TokenStream> {
        if !has_marker
            && self.event_types.is_empty()
            && self.poll_interval_ms.is_none()
            && !self.deadline
        {
            return Err(syn::Error::new_spanned(
                struct_name,
                "#[processor] needs a non-empty `subscribe`, `poll_interval`, or `deadline` to generate \
                 Runnable; use #[rmk::macros::runnable_generated] when implementing Runnable yourself",
            ).to_compile_error());
        }
        Ok(())
    }
}

/// Parse processor config from attribute tokens.
pub fn parse_processor_config(
    tokens: impl Into<TokenStream>,
) -> Result<ProcessorConfig, TokenStream> {
    let parser = AttributeParser::new_validated_with_flags(
        tokens,
        &["subscribe", "poll_interval"],
        &["deadline"],
    )?;

    let poll_interval_ms = parser
        .get_int::<std::num::NonZeroU64>("poll_interval")?
        .map(std::num::NonZeroU64::get);

    Ok(ProcessorConfig {
        event_types: parser.get_path_array("subscribe")?,
        poll_interval_ms,
        deadline: parser.has_flag("deadline"),
    })
}

/// Merge all processor attributes before either macro generates a shared Runnable.
pub fn merge_processor_attrs(
    mut config: ProcessorConfig,
    attrs: &[syn::Attribute],
) -> Result<ProcessorConfig, TokenStream> {
    for attr in attrs
        .iter()
        .filter(|attr| attr_matches_name(attr, "processor"))
    {
        if matches!(attr.meta, Meta::Path(_)) {
            continue;
        }
        let Meta::List(meta) = &attr.meta else {
            return Err(
                syn::Error::new_spanned(attr, "#[processor] requires parameters")
                    .to_compile_error(),
            );
        };
        let sibling = parse_processor_config(meta.tokens.clone())?;
        config.event_types.extend(sibling.event_types);
        config.deadline |= sibling.deadline;
        if sibling.poll_interval_ms.is_some() {
            if config.poll_interval_ms.is_some() {
                return Err(syn::Error::new_spanned(
                    attr,
                    "Conflicting poll_interval in multiple #[processor] attributes",
                )
                .to_compile_error());
            }
            config.poll_interval_ms = sibling.poll_interval_ms;
        }
    }
    config.event_types.retain({
        let mut seen = std::collections::HashSet::new();
        move |path| seen.insert(quote!(#path).to_string())
    });
    Ok(config)
}

/// Implementation of the unified `#[processor]` macro.
pub fn processor_impl(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut input = parse_macro_input!(item as DeriveInput);
    let mut config = match parse_processor_config(proc_macro2::TokenStream::from(attr)) {
        Ok(config) => config,
        Err(err) => return err.into(),
    };

    config = match merge_processor_attrs(config, &input.attrs) {
        Ok(config) => config,
        Err(err) => return err.into(),
    };

    let struct_name = &input.ident;
    let vis = &input.vis;
    let generics = &input.generics;
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let deduped_ty_generics = crate::event_macros::utils::deduplicate_type_generics(generics);

    let has_marker = has_runnable_marker(&input.attrs);
    if let Err(err) = config.validate_runnable(struct_name, has_marker) {
        return quote! { #input #err }.into();
    }

    // Check for sibling #[input_device] attribute.
    // Support both simple form (#[input_device]) and qualified form (#[rmk_macro::input_device])
    let has_input_device = input
        .attrs
        .iter()
        .any(|attr| attr_matches_name(attr, "input_device"));

    // Generate event enum, subscriber, and dispatch body
    let (event_type_tokens, event_enum_def, event_subscriber_impl, process_body) =
        generate_event_enum_and_dispatch(
            struct_name,
            vis,
            &config.event_types,
            "Processor",
            quote! { ::rmk::event::SubscribableEvent },
            quote! { subscriber },
        );

    let subscriber_body = if config.event_types.is_empty() {
        quote! { ::core::future::pending::<Self::Event>() }
    } else {
        quote! { <#event_type_tokens as ::rmk::event::SubscribableEvent>::subscriber() }
    };

    // PollingProcessor impl when poll_interval is set
    let polling_processor_impl = if let Some(interval_ms) = config.poll_interval_ms {
        quote! {
            impl #impl_generics ::rmk::processor::PollingProcessor for #struct_name #deduped_ty_generics #where_clause {
                fn interval(&self) -> ::embassy_time::Duration {
                    ::embassy_time::Duration::from_millis(#interval_ms)
                }

                async fn update(&mut self) {
                    self.poll().await;
                }
            }
        }
    } else {
        quote! {}
    };

    let deadline_processor_impl = if config.deadline {
        quote! {
            impl #impl_generics ::rmk::processor::DeadlineProcessor for #struct_name #deduped_ty_generics #where_clause {
                fn next_deadline(&self) -> Option<::embassy_time::Instant> {
                    Self::deadline(self)
                }

                async fn handle_deadline(&mut self) {
                    Self::on_deadline(self).await;
                }
            }
        }
    } else {
        quote! {}
    };

    // Generate Runnable implementation
    // Logic: If marker exists, Runnable was already generated by another macro, so skip.
    // Otherwise, we're the first macro to run, so generate Runnable.

    // Parse sibling input_device config if present (for combined Runnable generation)
    let input_device_config = if has_input_device {
        let attr = input
            .attrs
            .iter()
            .find(|attr| attr_matches_name(attr, "input_device"))
            .unwrap(); // Safe because has_input_device is true

        if let Meta::List(meta_list) = &attr.meta {
            use crate::event_macros::parser::parse_input_device_config;
            match parse_input_device_config(meta_list.tokens.clone()) {
                Ok(cfg) => Some(cfg),
                Err(err) => return err.into(),
            }
        } else {
            return syn::Error::new_spanned(
                attr,
                "#[input_device] requires parameters. Use `#[input_device(publish = EventType)]`",
            )
            .to_compile_error()
            .into();
        }
    } else {
        None
    };

    let runnable_impl = if has_marker {
        // Runnable was already generated by another macro
        quote! {}
    } else {
        // We're the first macro to run, generate Runnable.
        // If sibling #[input_device] exists, this becomes a combined Runnable.
        generate_runnable(
            struct_name,
            generics,
            where_clause,
            input_device_config.as_ref(),
            Some(&config),
        )
    };

    // Remove only processor attribute to allow sibling macro expansion.
    input
        .attrs
        .retain(|attr| !attr_matches_name(attr, "processor"));

    // Add marker only when sibling macro needs to know Runnable is already generated.
    if has_input_device && !has_marker {
        input
            .attrs
            .push(syn::parse_quote!(#[::rmk::macros::runnable_generated]));
    }

    let expanded = quote! {
        #input

        #event_enum_def
        #event_subscriber_impl

        impl #impl_generics ::rmk::processor::Processor for #struct_name #deduped_ty_generics #where_clause {
            type Event = #event_type_tokens;

            fn subscriber() -> impl ::rmk::event::EventSubscriber<Event = Self::Event> {
                #subscriber_body
            }

            async fn process(&mut self, event: Self::Event) {
                #process_body
            }
        }

        #polling_processor_impl

        #deadline_processor_impl

        #runnable_impl
    };

    expanded.into()
}

#[cfg(test)]
mod tests {
    use quote::quote;

    use super::parse_processor_config;

    fn error(tokens: proc_macro2::TokenStream) -> String {
        match parse_processor_config(tokens) {
            Ok(_) => panic!("expected an error"),
            Err(err) => err.to_string(),
        }
    }

    #[test]
    fn deadline_flag_is_parsed() {
        let config = parse_processor_config(quote! { subscribe = [KeyEvent], deadline }).unwrap();
        assert!(config.deadline);
        assert!(config.poll_interval_ms.is_none());
        assert!(
            !parse_processor_config(quote! { subscribe = [KeyEvent] })
                .unwrap()
                .deadline
        );
    }

    #[test]
    fn misspelled_flag_lists_what_is_accepted() {
        let message = error(quote! { subscribe = [KeyEvent], dedline });
        assert!(
            message.contains(
                "unknown attribute `dedline`. Expected one of: `subscribe = ...`, `poll_interval = ...`, `deadline`"
            ),
            "{message}"
        );
    }

    #[test]
    fn a_path_is_not_a_flag() {
        // Validation and `has_flag` must agree: `::deadline` is not the flag, so it is an error
        // rather than silently ignored.
        let message = error(quote! { subscribe = [KeyEvent], ::deadline });
        assert!(
            message.contains("unknown attribute `::deadline`"),
            "{message}"
        );
    }

    #[test]
    fn key_without_value_is_explained() {
        let message = error(quote! { subscribe = [KeyEvent], poll_interval });
        assert!(
            message.contains("`poll_interval` needs a value: `poll_interval = ...`"),
            "{message}"
        );
    }

    #[test]
    fn flag_with_value_is_explained() {
        let message = error(quote! { subscribe = [KeyEvent], deadline = true });
        assert!(
            message.contains("`deadline` is a flag and takes no value: write `deadline`"),
            "{message}"
        );
    }
}
