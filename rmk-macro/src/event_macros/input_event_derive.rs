use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, parse_macro_input};

/// Derive `PublishableEvent`/`AsyncPublishableEvent` for wrapper enums.
///
/// Generates:
/// - publisher type for the enum (routes to individual event channels)
/// - `PublishableEvent`/`AsyncPublishableEvent` impls
/// - `From<VariantType>` impls for each variant
///
/// **Note**: Wrapper enums only implement publish traits, not subscribe traits.
/// This is because wrapper enums route events to their concrete type channels,
/// and you should subscribe to the individual event types instead.
///
/// # Example
///
/// ```rust,ignore
/// #[derive(Event)]
/// pub enum MultiSensorEvent {
///     Battery(BatteryEvent),
///     Pointing(PointingEvent),
/// }
///
/// // Usage:
/// publish_event_async(MultiSensorEvent::Battery(event)).await;
/// ```
pub fn event_derive_impl(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    // Ensure input is an enum.
    let data_enum = match &input.data {
        syn::Data::Enum(e) => e,
        _ => {
            return syn::Error::new_spanned(input, "#[derive(Event)] can only be applied to enums")
                .to_compile_error()
                .into();
        }
    };

    let enum_name = &input.ident;
    let publisher_name = format_ident!("{}Publisher", enum_name);
    let vis = &input.vis;

    // Split generics for impl blocks.
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Collect variant info.
    let mut async_publish_arms = Vec::new();
    let mut publish_arms = Vec::new();
    let mut from_impls = Vec::new();
    let mut publish_is_noop_expr = quote!(true);

    for variant in &data_enum.variants {
        let variant_name = &variant.ident;

        // Require a single-field tuple variant.
        let inner_type = match &variant.fields {
            syn::Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                &fields.unnamed.first().unwrap().ty
            }
            _ => {
                return syn::Error::new_spanned(
                    variant,
                    "Each variant must be a tuple variant with exactly one field, e.g., `Battery(BatteryEvent)`",
                )
                .to_compile_error()
                .into();
            }
        };

        // Sync publish arm.
        publish_arms.push(quote! {
            #enum_name::#variant_name(e) => ::rmk::event::publish_event(e)
        });

        // Async publish arm.
        async_publish_arms.push(quote! {
            #enum_name::#variant_name(e) => ::rmk::event::publish_event_async(e).await
        });

        // From impls (with generics).
        from_impls.push(quote! {
            impl #impl_generics From<#inner_type> for #enum_name #ty_generics #where_clause {
                fn from(e: #inner_type) -> Self {
                    #enum_name::#variant_name(e)
                }
            }
        });

        publish_is_noop_expr = quote! {
            #publish_is_noop_expr && (<#inner_type as ::rmk::event::PublishableEvent>::PUBLISH_IS_NOOP)
        };
    }

    let has_generics = !input.generics.params.is_empty();

    let publisher_def = if has_generics {
        quote! {
            #vis struct #publisher_name #impl_generics
                (::core::marker::PhantomData<#enum_name #ty_generics>)
                #where_clause;
        }
    } else {
        quote! {
            #vis struct #publisher_name;
        }
    };

    let publisher_ctor = if has_generics {
        quote! { #publisher_name(::core::marker::PhantomData) }
    } else {
        quote! { #publisher_name }
    };

    let expanded = quote! {
        /// Publisher for the wrapper enum.
        /// Routes each variant to its event channel.
        #publisher_def

        impl #impl_generics ::rmk::event::AsyncEventPublisher for #publisher_name #ty_generics #where_clause {
            type Event = #enum_name #ty_generics;

            async fn publish_async(&self, event: #enum_name #ty_generics) {
                match event {
                    #(#async_publish_arms),*
                }
            }
        }

        impl #impl_generics ::rmk::event::EventPublisher for #publisher_name #ty_generics #where_clause {
            type Event = #enum_name #ty_generics;

            fn publish(&self, event: #enum_name #ty_generics) {
                match event {
                    #(#publish_arms),*
                }
            }
        }

        impl #impl_generics ::rmk::event::PublishableEvent for #enum_name #ty_generics #where_clause {
            type Publisher = #publisher_name #ty_generics;
            const PUBLISH_IS_NOOP: bool = #publish_is_noop_expr;

            fn publisher() -> Self::Publisher {
                #publisher_ctor
            }
        }

        impl #impl_generics ::rmk::event::AsyncPublishableEvent for #enum_name #ty_generics #where_clause {
            type AsyncPublisher = #publisher_name #ty_generics;

            fn publisher_async() -> Result<Self::AsyncPublisher, ::embassy_sync::pubsub::Error> {
                Ok(#publisher_ctor)
            }
        }

        #(#from_impls)*
    };

    expanded.into()
}
