//! EventSubscriber generation for aggregated event handling.
//!
//! Used by the `#[processor]` macro.

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Path;

use super::naming::{event_type_to_handler_method_name, generate_unique_variant_names};

/// Generate EventSubscriber struct and its implementation.
///
/// This is a unified generator for the `#[processor]` macro.
/// It generates:
/// - A subscriber struct that holds individual event subscribers
/// - `EventSubscriber` impl with `select_biased!` for event aggregation
/// - The corresponding `SubscribableEvent` trait impl
///
/// # Parameters
/// - `subscribe_trait_path`: The trait path (e.g., `::rmk::event::SubscribableEvent`)
/// - `subscriber_method`: The method name to call (e.g., `subscriber`)
pub fn generate_event_subscriber(
    subscriber_name: &syn::Ident,
    event_types: &[Path],
    variant_names: &[syn::Ident],
    enum_name: &syn::Ident,
    vis: &syn::Visibility,
    subscribe_trait_path: TokenStream,
    subscriber_method: TokenStream,
) -> TokenStream {
    let num_events = event_types.len();

    // Subscriber field names
    let sub_fields: Vec<_> = (0..num_events).map(|i| format_ident!("sub{}", i)).collect();

    // Struct field definitions with types
    let field_defs: Vec<_> = event_types
        .iter()
        .zip(&sub_fields)
        .map(|(event_type, field_name)| {
            quote! {
                #field_name: <#event_type as #subscribe_trait_path>::Subscriber
            }
        })
        .collect();

    // Field initializations in new()
    let field_inits: Vec<_> = event_types
        .iter()
        .zip(&sub_fields)
        .map(|(event_type, field_name)| {
            quote! {
                #field_name: <#event_type as #subscribe_trait_path>::#subscriber_method()
            }
        })
        .collect();

    // select_biased! arms for next_event
    let select_arms: Vec<_> = sub_fields
        .iter()
        .zip(variant_names)
        .map(|(field_name, variant_name)| {
            quote! {
                event = self.#field_name.next_event().fuse() => #enum_name::#variant_name(event),
            }
        })
        .collect();

    quote! {
        /// Event subscriber for aggregated events
        #vis struct #subscriber_name {
            #(#field_defs),*
        }

        impl #subscriber_name {
            /// Create a new event subscriber
            pub fn new() -> Self {
                Self {
                    #(#field_inits),*
                }
            }
        }

        impl ::rmk::event::EventSubscriber for #subscriber_name {
            type Event = #enum_name;

            async fn next_event(&mut self) -> Self::Event {
                use ::rmk::event::EventSubscriber;
                use ::rmk::futures::FutureExt;

                ::rmk::futures::select_biased! {
                    #(#select_arms)*
                }
            }
        }

        impl #subscribe_trait_path for #enum_name {
            type Subscriber = #subscriber_name;

            fn #subscriber_method() -> Self::Subscriber {
                #subscriber_name::new()
            }
        }
    }
}

/// Generate process match arms.
///
/// This is used by the `#[processor]` macro.
pub fn generate_event_match_arms(
    event_types: &[Path],
    variant_names: &[syn::Ident],
    enum_name: &syn::Ident,
) -> Vec<TokenStream> {
    event_types
        .iter()
        .zip(variant_names)
        .map(|(event_type, variant_name)| {
            let method_name = event_type_to_handler_method_name(event_type);
            quote! {
                #enum_name::#variant_name(event) => self.#method_name(event).await
            }
        })
        .collect()
}

/// Generate event enum, subscriber, and dispatch body for a subscriber-based macro.
///
/// This is the unified generator used by the `#[processor]` macro.
/// Without subscriptions, uses `Infallible` and emits no subscriber or event enum.
/// For single-event subscriptions, no enum is generated and the event type is used directly.
/// For multiple events, generates an aggregated enum, subscriber struct, and match-based dispatch.
///
/// # Parameters
/// - `kind`: Used for naming the generated enum/subscriber (e.g., `"Processor"`)
/// - `subscribe_trait_path`: e.g. `::rmk::event::SubscribableEvent`
/// - `subscriber_method`: e.g. `subscriber`
///
/// # Returns
/// `(event_type_tokens, event_enum_def, event_subscriber_impl, dispatch_body)`
pub fn generate_event_enum_and_dispatch(
    struct_name: &syn::Ident,
    vis: &syn::Visibility,
    event_types: &[Path],
    kind: &str,
    subscribe_trait_path: TokenStream,
    subscriber_method: TokenStream,
) -> (TokenStream, TokenStream, TokenStream, TokenStream) {
    if event_types.is_empty() {
        (
            quote! { ::core::convert::Infallible },
            quote! {},
            quote! {},
            quote! { match event {} },
        )
    } else if event_types.len() == 1 {
        // Single event: use the event type directly, no enum needed
        let event_type = &event_types[0];
        let method_name = event_type_to_handler_method_name(event_type);

        (
            quote! { #event_type },
            quote! {},
            quote! {},
            quote! { self.#method_name(event).await },
        )
    } else {
        // Multiple events: generate aggregated enum
        let enum_name = format_ident!("{}{}EventEnum", struct_name, kind);
        let subscriber_name = format_ident!("{}{}EventSubscriber", struct_name, kind);
        let variant_names = generate_unique_variant_names(event_types);

        let enum_variants: Vec<TokenStream> = event_types
            .iter()
            .zip(&variant_names)
            .map(|(event_type, variant_name)| {
                quote! { #variant_name(#event_type) }
            })
            .collect();

        let match_arms = generate_event_match_arms(event_types, &variant_names, &enum_name);

        let subscriber_impl = generate_event_subscriber(
            &subscriber_name,
            event_types,
            &variant_names,
            &enum_name,
            vis,
            subscribe_trait_path,
            subscriber_method,
        );

        let enum_def = quote! {
            #[derive(Clone)]
            #vis enum #enum_name {
                #(#enum_variants),*
            }
        };

        (
            quote! { #enum_name },
            enum_def,
            subscriber_impl,
            quote! {
                match event {
                    #(#match_arms),*
                }
            },
        )
    }
}
