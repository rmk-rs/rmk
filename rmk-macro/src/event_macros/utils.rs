//! Event system utility functions for rmk-macro.
//!
//! Contains utilities used by event system macros.
//! Case conversion utilities remain in the root utils module.

use std::collections::HashSet;

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::Parser;
use syn::{Attribute, GenericParam, Meta};

/// Generic attribute parser for extracting values from macro attributes.
///
/// Parses attribute tokens in the form of `name = value` or `name = [value1, value2]`.
pub struct AttributeParser {
    metas: Vec<Meta>,
}

impl AttributeParser {
    /// Create a new parser from attribute tokens.
    pub fn new(tokens: impl Into<TokenStream>) -> Result<Self, syn::Error> {
        use syn::Token;
        use syn::punctuated::Punctuated;

        let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
        let tokens: TokenStream = tokens.into();
        let metas = parser.parse2(tokens)?;
        Ok(Self {
            metas: metas.into_iter().collect(),
        })
    }

    /// Create a new parser and validate keys in one step.
    ///
    /// This is more ergonomic than calling `new()` followed by `validate_keys()`.
    pub fn new_validated(
        tokens: impl Into<TokenStream>,
        allowed_keys: &[&str],
    ) -> Result<Self, TokenStream> {
        let parser = Self::new(tokens).map_err(|e| e.to_compile_error())?;
        parser.validate_keys(allowed_keys)?;
        Ok(parser)
    }

    /// Get an integer value for `name = N`.
    ///
    /// Returns an error when the key exists but is not an integer literal,
    /// or cannot be parsed into the requested integer type.
    pub fn get_int<T>(&self, name: &str) -> Result<Option<T>, TokenStream>
    where
        T: std::str::FromStr,
        T::Err: std::fmt::Display,
    {
        let Some(meta) = self
            .metas
            .iter()
            .find(|meta| matches!(meta, Meta::NameValue(nv) if nv.path.is_ident(name)))
        else {
            return Ok(None);
        };

        let Meta::NameValue(nv) = meta else {
            return Ok(None);
        };

        let syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(lit),
            ..
        }) = &nv.value
        else {
            return Err(syn::Error::new_spanned(
                &nv.value,
                format!("`{name}` must be an integer literal"),
            )
            .to_compile_error());
        };

        lit.base10_parse().map(Some).map_err(|err| {
            syn::Error::new_spanned(lit, format!("invalid `{name}` value: {err}"))
                .to_compile_error()
        })
    }

    /// Get an array of paths for `name = [Type1, Type2]`.
    ///
    /// Returns an error when the key exists but the value is not an array,
    /// or when any array element is not a path.
    pub fn get_path_array(&self, name: &str) -> Result<Vec<syn::Path>, TokenStream> {
        let Some(meta) = self
            .metas
            .iter()
            .find(|meta| matches!(meta, Meta::NameValue(nv) if nv.path.is_ident(name)))
        else {
            return Ok(vec![]);
        };

        let Meta::NameValue(nv) = meta else {
            return Ok(vec![]);
        };

        let syn::Expr::Array(arr) = &nv.value else {
            return Err(syn::Error::new_spanned(
                &nv.value,
                format!("`{name}` must be an array of type paths, e.g. `[EventA, EventB]`"),
            )
            .to_compile_error());
        };

        let mut result = Vec::with_capacity(arr.elems.len());
        for elem in &arr.elems {
            if let syn::Expr::Path(path_expr) = elem {
                result.push(path_expr.path.clone());
            } else {
                return Err(syn::Error::new_spanned(
                    elem,
                    format!("invalid `{name}` element: expected a type path"),
                )
                .to_compile_error());
            }
        }

        Ok(result)
    }

    /// Get a single path for `name = Type`.
    pub fn get_path(&self, name: &str) -> Option<syn::Path> {
        self.metas.iter().find_map(|meta| {
            if let Meta::NameValue(nv) = meta
                && nv.path.is_ident(name)
                && let syn::Expr::Path(p) = &nv.value
            {
                Some(p.path.clone())
            } else {
                None
            }
        })
    }

    /// Get an expression as TokenStream for `name = expr`.
    /// Useful for values that need to be embedded as-is (like channel_size).
    pub fn get_expr_tokens(&self, name: &str) -> Option<TokenStream> {
        self.metas.iter().find_map(|meta| {
            if let Meta::NameValue(nv) = meta
                && nv.path.is_ident(name)
            {
                let expr = &nv.value;
                Some(quote! { #expr })
            } else {
                None
            }
        })
    }

    /// Create a new parser that also accepts the bare `flags`, e.g. `deadline`.
    pub fn new_validated_with_flags(
        tokens: impl Into<TokenStream>,
        allowed_keys: &[&str],
        allowed_flags: &[&str],
    ) -> Result<Self, TokenStream> {
        let parser = Self::new(tokens).map_err(|e| e.to_compile_error())?;
        parser.validate(allowed_keys, allowed_flags)?;
        Ok(parser)
    }

    /// Whether the bare flag `name` is present.
    pub fn has_flag(&self, name: &str) -> bool {
        self.metas
            .iter()
            .any(|meta| matches!(meta, Meta::Path(path) if path.is_ident(name)))
    }

    /// Validate attribute key/value pairs against the allowed set.
    ///
    /// Enforces `key = value` syntax and rejects unknown keys.
    pub fn validate_keys(&self, allowed: &[&str]) -> Result<(), TokenStream> {
        self.validate(allowed, &[])
    }

    /// [`Self::validate_keys`], plus the bare `flags`.
    ///
    /// Without flags the errors are exactly [`Self::validate_keys`]'s. With flags, every error
    /// lists what the attribute accepts, and a key or flag written the wrong way says so.
    fn validate(&self, allowed: &[&str], flags: &[&str]) -> Result<(), TokenStream> {
        let expected = || {
            allowed
                .iter()
                .map(|key| format!("`{key} = ...`"))
                .chain(flags.iter().map(|flag| format!("`{flag}`")))
                .collect::<Vec<_>>()
                .join(", ")
        };

        for meta in &self.metas {
            // A bare word is a flag, but only where flags exist; elsewhere it falls through to
            // the `key = value` error below, unchanged.
            if let Meta::Path(path) = meta
                && !flags.is_empty()
            {
                if flags.iter().any(|flag| path.is_ident(flag)) {
                    continue;
                }
                let name = quote!(#path).to_string().replace(' ', "");
                let message = if allowed.contains(&name.as_str()) {
                    format!("`{name}` needs a value: `{name} = ...`")
                } else {
                    format!(
                        "unknown attribute `{name}`. Expected one of: {}",
                        expected()
                    )
                };
                return Err(syn::Error::new_spanned(path, message).to_compile_error());
            }

            let Meta::NameValue(nv) = meta else {
                let message = if flags.is_empty() {
                    "invalid attribute syntax. Expected `key = value`".to_string()
                } else {
                    format!("invalid attribute syntax. Expected one of: {}", expected())
                };
                return Err(syn::Error::new_spanned(meta, message).to_compile_error());
            };

            let Some(key_ident) = nv.path.get_ident() else {
                return Err(syn::Error::new_spanned(
                    &nv.path,
                    "invalid attribute key. Expected a simple identifier",
                )
                .to_compile_error());
            };

            let key = key_ident.to_string();
            if !allowed.contains(&key.as_str()) {
                let message = if flags.contains(&key.as_str()) {
                    format!("`{key}` is a flag and takes no value: write `{key}`")
                } else if flags.is_empty() {
                    format!(
                        "unknown attribute `{key}`. Expected one of: {}",
                        allowed.join(", ")
                    )
                } else {
                    format!("unknown attribute `{key}`. Expected one of: {}", expected())
                };
                return Err(syn::Error::new_spanned(&nv.path, message).to_compile_error());
            }
        }
        Ok(())
    }
}

/// Deduplicate generic parameters by name.
/// Handles cfg-conditional generics that repeat the same name.
pub fn deduplicate_type_generics(generics: &syn::Generics) -> TokenStream {
    let mut seen = HashSet::new();
    let mut unique_params = Vec::new();

    for param in &generics.params {
        let name = match param {
            GenericParam::Type(t) => t.ident.to_string(),
            GenericParam::Lifetime(l) => l.lifetime.to_string(),
            GenericParam::Const(c) => c.ident.to_string(),
        };

        if seen.insert(name) {
            // First occurrence.
            match param {
                GenericParam::Type(t) => {
                    let ident = &t.ident;
                    unique_params.push(quote! { #ident });
                }
                GenericParam::Lifetime(l) => {
                    let lifetime = &l.lifetime;
                    unique_params.push(quote! { #lifetime });
                }
                GenericParam::Const(c) => {
                    let ident = &c.ident;
                    unique_params.push(quote! { #ident });
                }
            }
        }
    }

    if unique_params.is_empty() {
        quote! {}
    } else {
        quote! { < #(#unique_params),* > }
    }
}

/// Check if a type derives a trait (e.g., Clone).
///
/// This function parses the derive attribute properly to avoid false positives.
/// For example, searching for "Clone" won't match "CloneInto" or "DeepClone".
pub fn has_derive(attrs: &[Attribute], derive_name: &str) -> bool {
    use syn::punctuated::Punctuated;
    use syn::{Path, Token};

    attrs.iter().any(|attr| {
        if !attr.path().is_ident("derive") {
            return false;
        }

        let Meta::List(meta_list) = &attr.meta else {
            return false;
        };

        // Parse the derive macro's token list as comma-separated paths
        let parser = Punctuated::<Path, Token![,]>::parse_terminated;
        let Ok(paths) = parser.parse2(meta_list.tokens.clone()) else {
            return false;
        };

        // Check if any path's last segment matches the derive name exactly
        paths.iter().any(|path| {
            path.segments
                .last()
                .map(|seg| seg.ident == derive_name)
                .unwrap_or(false)
        })
    })
}

/// Check for the runnable_generated marker.
/// Prevents duplicate Runnable impls when macros combine.
pub fn has_runnable_marker(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr_matches_name(attr, "runnable_generated"))
}

/// Check if an attribute matches a given name, supporting both simple and qualified paths.
///
/// Examples:
/// - `#[processor]` matches "processor"
/// - `#[rmk_macro::processor]` matches "processor"
/// - `#[some::other::path]` does not match "processor"
pub fn attr_matches_name(attr: &Attribute, name: &str) -> bool {
    attr.path()
        .segments
        .last()
        .map(|seg| seg.ident == name)
        .unwrap_or(false)
}
