//! Unified event macro implementation.
//!
//! Generates static channels and trait implementations for events.
//! Supports both MPSC (Channel) and PubSub (PubSubChannel) modes.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

use crate::event_macros::utils::{AttributeParser, has_derive};
use crate::utils::to_upper_snake_case;

/// Configuration for the unified `#[event]` macro.
pub struct EventConfig {
    /// Buffer size of the channel
    pub channel_size: Option<TokenStream>,
    /// Max subscribers (pubsub only)
    pub subs: Option<TokenStream>,
    /// Max publishers (pubsub only)
    pub pubs: Option<TokenStream>,
}

/// Channel type for event macro.
/// Automatically inferred: if `subs` or `pubs` is specified, PubSub is used; otherwise MPSC.
enum ChannelType {
    /// MPSC channel (embassy_sync::channel::Channel) - single consumer
    Mpsc,
    /// PubSub channel (embassy_sync::pubsub::PubSubChannel) - broadcast
    PubSub,
}

/// Parse event config from attribute tokens.
///
/// Channel type is automatically inferred: if `subs` or `pubs` is specified, PubSub is used;
/// otherwise MPSC.
///
/// Returns an error if unknown attribute keys are found.
pub fn parse_event_config(tokens: impl Into<TokenStream>) -> Result<EventConfig, TokenStream> {
    let parser = AttributeParser::new_validated(tokens, &["channel_size", "subs", "pubs"])?;

    Ok(EventConfig {
        channel_size: parser.get_expr_tokens("channel_size"),
        subs: parser.get_expr_tokens("subs"),
        pubs: parser.get_expr_tokens("pubs"),
    })
}

/// Generate MPSC channel static and trait implementations.
fn generate_mpsc_channel(
    type_name: &syn::Ident,
    ty_generics: &syn::TypeGenerics,
    impl_generics: &syn::ImplGenerics,
    where_clause: Option<&syn::WhereClause>,
    channel_size: Option<TokenStream>,
) -> (TokenStream, TokenStream) {
    let channel_name = syn::Ident::new(
        &format!(
            "{}_EVENT_CHANNEL",
            to_upper_snake_case(&type_name.to_string())
        ),
        type_name.span(),
    );

    let cap = channel_size.unwrap_or_else(|| quote! { 8 });

    let channel_static = quote! {
        #[doc(hidden)]
        static #channel_name: ::embassy_sync::channel::Channel<
            ::rmk::RawMutex,
            #type_name #ty_generics,
            { #cap }
        > = ::embassy_sync::channel::Channel::new();
    };

    let trait_impls = quote! {
        impl #impl_generics ::rmk::event::PublishableEvent for #type_name #ty_generics #where_clause {
            type Publisher = ::embassy_sync::channel::Sender<
                'static,
                ::rmk::RawMutex,
                #type_name #ty_generics,
                { #cap }
            >;
            // A MPSC channel has a single consumer by definition.
            const PUBLISH_IS_NOOP: bool = false;

            fn publisher() -> Self::Publisher {
                #channel_name.sender()
            }
        }

        impl #impl_generics ::rmk::event::SubscribableEvent for #type_name #ty_generics #where_clause {
            type Subscriber = ::embassy_sync::channel::Receiver<
                'static,
                ::rmk::RawMutex,
                #type_name #ty_generics,
                { #cap }
            >;

            fn subscriber() -> Self::Subscriber {
                #channel_name.receiver()
            }
        }

        impl #impl_generics ::rmk::event::AsyncPublishableEvent for #type_name #ty_generics #where_clause {
            type AsyncPublisher = ::embassy_sync::channel::Sender<
                'static,
                ::rmk::RawMutex,
                #type_name #ty_generics,
                { #cap }
            >;

            fn publisher_async() -> Result<Self::AsyncPublisher, ::embassy_sync::pubsub::Error> {
                Ok(#channel_name.sender())
            }
        }
    };

    (channel_static, trait_impls)
}

/// Generate PubSub channel static and trait implementations.
fn generate_pubsub_channel(
    type_name: &syn::Ident,
    ty_generics: &syn::TypeGenerics,
    impl_generics: &syn::ImplGenerics,
    where_clause: Option<&syn::WhereClause>,
    config: &EventConfig,
) -> (TokenStream, TokenStream) {
    let channel_name = syn::Ident::new(
        &format!(
            "{}_EVENT_CHANNEL",
            to_upper_snake_case(&type_name.to_string())
        ),
        type_name.span(),
    );

    let cap = config.channel_size.clone().unwrap_or_else(|| quote! { 1 });
    let subs_val = config.subs.clone().unwrap_or_else(|| quote! { 4 });
    let pubs_val = config.pubs.clone().unwrap_or_else(|| quote! { 1 });

    let channel_static = quote! {
        #[doc(hidden)]
        static #channel_name: ::embassy_sync::pubsub::PubSubChannel<
            ::rmk::RawMutex,
            #type_name #ty_generics,
            { #cap },
            { #subs_val },
            { #pubs_val }
        > = ::embassy_sync::pubsub::PubSubChannel::new();
    };

    let trait_impls = quote! {
        impl #impl_generics ::rmk::event::PublishableEvent for #type_name #ty_generics #where_clause {
            type Publisher = ::embassy_sync::pubsub::ImmediatePublisher<
                'static,
                ::rmk::RawMutex,
                #type_name #ty_generics,
                { #cap },
                { #subs_val },
                { #pubs_val }
            >;
            const PUBLISH_IS_NOOP: bool = #subs_val == 0;

            fn publisher() -> Self::Publisher {
                #channel_name.immediate_publisher()
            }
        }

        impl #impl_generics ::rmk::event::SubscribableEvent for #type_name #ty_generics #where_clause {
            type Subscriber = ::embassy_sync::pubsub::Subscriber<
                'static,
                ::rmk::RawMutex,
                #type_name #ty_generics,
                { #cap },
                { #subs_val },
                { #pubs_val }
            >;

            fn subscriber() -> Self::Subscriber {
                #channel_name.subscriber().expect(
                    concat!(
                        "Failed to create subscriber for ",
                        stringify!(#type_name),
                        ". The 'subs' limit has been exceeded. Increase 'subs' for this event in keyboard.toml [event] (or #[event(subs = N)] for custom events)."
                    )
                )
            }
        }

        impl #impl_generics ::rmk::event::AsyncPublishableEvent for #type_name #ty_generics #where_clause {
            type AsyncPublisher = ::embassy_sync::pubsub::Publisher<
                'static,
                ::rmk::RawMutex,
                #type_name #ty_generics,
                { #cap },
                { #subs_val },
                { #pubs_val }
            >;

            fn publisher_async() -> Result<Self::AsyncPublisher, ::embassy_sync::pubsub::Error> {
                #channel_name.publisher()
            }
        }
    };

    (channel_static, trait_impls)
}

/// Implementation of the unified `#[event]` macro.
pub fn event_impl(
    attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let input = parse_macro_input!(item as DeriveInput);

    // Parse config
    let config = match parse_event_config(proc_macro2::TokenStream::from(attr)) {
        Ok(config) => config,
        Err(err) => return err.into(),
    };

    // Validate event type
    if let Some(error) = validate_event_type(&input, "event") {
        return error.into();
    }

    let type_name = input.ident.clone();
    let generics = input.generics.clone();
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // Infer channel type: if subs or pubs is specified, use PubSub; otherwise MPSC
    let channel_type = if config.subs.is_some() || config.pubs.is_some() {
        ChannelType::PubSub
    } else {
        ChannelType::Mpsc
    };

    // Generate channel and trait impls based on channel type
    let (channel_static, trait_impls) = match channel_type {
        ChannelType::Mpsc => generate_mpsc_channel(
            &type_name,
            &ty_generics,
            &impl_generics,
            where_clause,
            config.channel_size.clone(),
        ),
        ChannelType::PubSub => generate_pubsub_channel(
            &type_name,
            &ty_generics,
            &impl_generics,
            where_clause,
            &config,
        ),
    };

    let expanded = quote! {
        #input

        #channel_static
        #trait_impls
    };

    expanded.into()
}

/// Validate event type for event macros.
///
/// Returns an error TokenStream if validation fails, None if valid.
pub fn validate_event_type(input: &syn::DeriveInput, macro_name: &str) -> Option<TokenStream> {
    // Validate input is a struct or enum
    if !matches!(input.data, syn::Data::Struct(_) | syn::Data::Enum(_)) {
        return Some(
            syn::Error::new_spanned(
                input,
                format!("#[{}] can only be applied to structs or enums", macro_name),
            )
            .to_compile_error(),
        );
    }

    // Reject generic types - static channels cannot be generic
    if !input.generics.params.is_empty() {
        return Some(
            syn::Error::new_spanned(
                &input.generics,
                format!(
                    "#[{}] does not support generic types. Static channels cannot be generic.",
                    macro_name
                ),
            )
            .to_compile_error(),
        );
    }

    // Verify Clone derive
    if !has_derive(&input.attrs, "Clone") {
        return Some(
            syn::Error::new_spanned(
                input,
                format!("#[{}] requires the struct to derive Clone", macro_name),
            )
            .to_compile_error(),
        );
    }

    None
}
