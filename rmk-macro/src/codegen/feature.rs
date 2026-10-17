//! Utilities of check cargo feature
//!

/// Get enabled RMK features list
pub(crate) fn get_rmk_features() -> Option<Vec<String>> {
    // Use an absolute path. `cargo_toml::Manifest::from_path` resolves the
    // workspace root by walking ancestors of the given path; passing a
    // relative `"./Cargo.toml"` makes its fallback canonicalize an empty
    // path and fail with ENOENT inside workspace members.
    let manifest_path = std::path::Path::new(
        &std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is not set"),
    )
    .join("Cargo.toml");
    let manifest = cargo_toml::Manifest::from_path(&manifest_path).ok()?;
    let dep = manifest.dependencies.get("rmk")?;
    let mut feature_set = dep.req_features().to_vec();

    // Add default features to the feature list.
    // Must mirror `default = [...]` in rmk/Cargo.toml.
    if dep.detail().is_none_or(|d| d.default_features) {
        feature_set.push("defmt".to_string());
        feature_set.push("storage".to_string());
        feature_set.push("vial".to_string());
        feature_set.push("host_lock".to_string());
        feature_set.push("watchdog".to_string());
    }
    Some(feature_set)
}

/// Check whether the given feature is enabled
pub(crate) fn is_feature_enabled(feature_list: &Option<Vec<String>>, feature: &str) -> bool {
    feature_list
        .as_ref()
        .is_some_and(|features| features.iter().any(|f| f == feature))
}
