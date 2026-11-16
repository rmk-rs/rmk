//! Naming helpers for generated event code.

use quote::format_ident;
use syn::Path;

use crate::utils::to_snake_case;

/// Convert an event type to `on_xxx_event`.
pub fn event_type_to_handler_method_name(path: &Path) -> syn::Ident {
    let type_name = path.segments.last().unwrap().ident.to_string();
    let base_name = type_name.strip_suffix("Event").unwrap_or(&type_name);
    format_ident!("on_{}_event", to_snake_case(base_name))
}

/// Generate unique variant names, adding numeric suffixes on collisions.
pub fn generate_unique_variant_names(event_types: &[Path]) -> Vec<syn::Ident> {
    use std::collections::HashMap;

    let base_names: Vec<String> = event_types
        .iter()
        .map(|path| {
            let type_name = path.segments.last().unwrap().ident.to_string();
            type_name
                .strip_suffix("Event")
                .unwrap_or(&type_name)
                .to_string()
        })
        .collect();

    let mut counts: HashMap<&str, usize> = HashMap::new();
    for name in &base_names {
        *counts.entry(name.as_str()).or_insert(0) += 1;
    }

    let mut seen: HashMap<&str, usize> = HashMap::new();
    base_names
        .iter()
        .map(|name| {
            let name_str = name.as_str();
            if counts[name_str] > 1 {
                let idx = seen.entry(name_str).or_insert(0);
                *idx += 1;
                format_ident!("{}{}", name, idx)
            } else {
                format_ident!("{}", name)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use syn::parse_quote;

    use super::*;

    #[test]
    fn test_generate_unique_variant_names() {
        let paths: Vec<Path> = vec![parse_quote!(BatteryEvent), parse_quote!(ChargingEvent)];
        let names = generate_unique_variant_names(&paths);
        assert_eq!(names[0].to_string(), "Battery");
        assert_eq!(names[1].to_string(), "Charging");

        let paths: Vec<Path> = vec![parse_quote!(FooEvent), parse_quote!(FooEvent)];
        let names = generate_unique_variant_names(&paths);
        assert_eq!(names[0].to_string(), "Foo1");
        assert_eq!(names[1].to_string(), "Foo2");

        let paths: Vec<Path> = vec![parse_quote!(KeyPress)];
        let names = generate_unique_variant_names(&paths);
        assert_eq!(names[0].to_string(), "KeyPress");
    }
}
