use darling::FromMeta;
use proc_macro2::TokenStream as TokenStream2;
use syn::{ItemFn, ItemMod};

/// List of functions that can be overwritten
#[derive(Debug, Clone, Copy, FromMeta)]
pub enum Overwritten {
    Usb,
    ChipConfig,
    ChipInit,
    Entry,
    /// `#[Override(bind_interrupt)]` — the form the stm32h7 example documents. Selected by
    /// `bind_interrupt.rs` through this shared matcher; the legacy bare `#[bind_interrupt]`
    /// marker is also still accepted there for backward compatibility.
    BindInterrupt,
}

/// Find the override attribute on a function and parse it. Both documented
/// spellings are accepted: `#[Override(...)]` (docs, examples) and
/// `#[Overwritten(...)]` (this enum's name; the previous matching ignored the
/// attribute path entirely, so both were in use).
///
/// A `#[cfg(...)]` / `#[cfg_attr(...)]` on the function makes it ineligible: an
/// attribute macro receives its module's items *before* `cfg` stripping, so a
/// disabled item still reaches us with its `cfg` intact and we cannot tell here
/// whether it is compiled. Such a function is skipped entirely — neither
/// selected nor validated — so a disabled override can't replace the generated
/// default and a disabled typo can't emit an error (#967 review). Inert
/// attributes (doc comments, `#[allow(...)]`, ...) don't gate compilation and
/// may sit alongside the marker.
///
/// Returns:
/// - `None`: no override marker, or the marker is `cfg`/`cfg_attr`-gated
/// - `Some(Ok(_))`: a valid override marker (inert attributes may accompany it)
/// - `Some(Err(_))`: an override attribute that failed to parse
///   (unknown or miscased variant, missing argument, ...)
pub(crate) fn find_overwritten(item_fn: &ItemFn) -> Option<darling::Result<Overwritten>> {
    let marker = item_fn
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("Override") || attr.path().is_ident("Overwritten"))?;
    // `cfg`/`cfg_attr` gate whether the item is compiled; we can't evaluate
    // that here, so leave the function untouched rather than mis-select it.
    if item_fn
        .attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
    {
        return None;
    }
    Some(Overwritten::from_meta(&marker.meta))
}

/// Validate every `#[Overwritten(...)]` attribute in the module.
///
/// Returns darling's diagnostics rendered as `compile_error!` tokens, or
/// `None` if all attributes are valid. This turns a typo like
/// `#[Overwritten(Entry)]` into a compile error instead of a silent fallback
/// to the generated default (#966).
pub(crate) fn validate_overwritten_attrs(item_mod: &ItemMod) -> Option<TokenStream2> {
    let mut errors: Vec<darling::Error> = Vec::new();
    if let Some((_, items)) = &item_mod.content {
        for item in items {
            if let syn::Item::Fn(item_fn) = item
                && let Some(Err(e)) = find_overwritten(item_fn)
            {
                errors.push(e);
            }
        }
    }
    if errors.is_empty() {
        None
    } else {
        Some(darling::Error::multiple(errors).write_errors())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fn(src: &str) -> ItemFn {
        syn::parse_str(src).expect("test fn should parse")
    }

    #[test]
    fn no_overwritten_attribute_is_none() {
        assert!(find_overwritten(&parse_fn("fn f() {}")).is_none());
        assert!(find_overwritten(&parse_fn("#[inline]\nfn f() {}")).is_none());
    }

    #[test]
    fn valid_variant_parses() {
        let res = find_overwritten(&parse_fn("#[Overwritten(entry)]\nfn f() {}"));
        assert!(matches!(res, Some(Ok(Overwritten::Entry))));
    }

    #[test]
    fn override_spelling_is_accepted() {
        // The documented spelling (docs/examples) is `#[Override(...)]`.
        let res = find_overwritten(&parse_fn("#[Override(chip_config)]\nfn f() {}"));
        assert!(matches!(res, Some(Ok(Overwritten::ChipConfig))));
    }

    #[test]
    fn doc_comment_does_not_disable_override() {
        // Doc comments are attributes; they used to make the override
        // silently ineligible via the old `attrs.len() == 1` check.
        let res = find_overwritten(&parse_fn("/// my entry\n#[Overwritten(entry)]\nfn f() {}"));
        assert!(matches!(res, Some(Ok(Overwritten::Entry))));
    }

    #[test]
    fn cfg_gated_override_is_ignored() {
        // A `cfg`-gated override must not be selected: the attribute macro sees
        // the item before `cfg` stripping and cannot tell whether it is active,
        // so it is left untouched (the generated default is used). Both a valid
        // variant and a typo behind `cfg` resolve to `None` — no selection and
        // no spurious compile error for a disabled function (#967 review).
        assert!(
            find_overwritten(&parse_fn(
                "#[cfg(feature = \"x\")]\n#[Override(chip_config)]\nfn f() {}"
            ))
            .is_none()
        );
        assert!(
            find_overwritten(&parse_fn(
                "#[cfg(feature = \"x\")]\n#[Override(Entry)]\nfn f() {}"
            ))
            .is_none()
        );
        // `cfg_attr` is gated the same way.
        assert!(
            find_overwritten(&parse_fn(
                "#[cfg_attr(test, inline)]\n#[Override(entry)]\nfn f() {}"
            ))
            .is_none()
        );
    }

    #[test]
    fn inert_attribute_does_not_disable_override() {
        // `#[allow(...)]` (like doc comments) does not gate compilation, so it
        // must not make the override ineligible — only `cfg`/`cfg_attr` do.
        let res = find_overwritten(&parse_fn(
            "#[allow(dead_code)]\n#[Override(entry)]\nfn f() {}",
        ));
        assert!(matches!(res, Some(Ok(Overwritten::Entry))));
        // A typo alongside an inert attribute is still caught (no cfg present).
        let res = find_overwritten(&parse_fn(
            "#[allow(dead_code)]\n#[Override(Entry)]\nfn f() {}",
        ));
        assert!(matches!(res, Some(Err(_))));
    }

    #[test]
    fn cfg_gated_override_produces_no_module_error() {
        // The disabled typo above must not turn into a compile error.
        let item_mod: ItemMod =
            syn::parse_str("mod kb { #[cfg(feature = \"x\")] #[Overwritten(Entry)] fn run() {} }")
                .unwrap();
        assert!(validate_overwritten_attrs(&item_mod).is_none());
    }

    #[test]
    fn miscased_variant_is_an_error() {
        let res = find_overwritten(&parse_fn("#[Overwritten(Entry)]\nfn f() {}"));
        assert!(matches!(res, Some(Err(_))));
        let res = find_overwritten(&parse_fn("#[Override(Entry)]\nfn f() {}"));
        let err = match res {
            Some(Err(e)) => e,
            other => panic!("expected Some(Err(_)), got {other:?}"),
        };
        // darling's diagnostic suggests the snake_case spelling
        assert!(
            err.to_string().contains("entry"),
            "unexpected message: {err}"
        );
    }

    #[test]
    fn invalid_attr_in_module_becomes_compile_error() {
        let item_mod: ItemMod =
            syn::parse_str("mod kb { #[Overwritten(Entry)] fn run() {} }").unwrap();
        let tokens = validate_overwritten_attrs(&item_mod).expect("should produce errors");
        assert!(tokens.to_string().contains("compile_error"));
    }

    #[test]
    fn valid_module_produces_no_errors() {
        let item_mod: ItemMod =
            syn::parse_str("mod kb { #[Overwritten(entry)] fn run() {} fn helper() {} }").unwrap();
        assert!(validate_overwritten_attrs(&item_mod).is_none());
    }
}
