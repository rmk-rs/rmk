#[path = "./build_common.rs"]
mod common;

use std::process::Command;

fn main() {
    let mut cfgs = common::CfgSet::new();
    common::set_target_cfgs(&mut cfgs);

    // `RMK_COMMIT` and `RMK_FEATURES` go into the storage schema hash.
    // When either changes, the storage is erased.
    let commit = Command::new("git")
        .args(["log", "-1", "--format=%H", "--", "."])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=RMK_COMMIT={commit}");

    let mut features: Vec<String> = std::env::vars()
        .filter_map(|(key, _)| key.strip_prefix("CARGO_FEATURE_").map(str::to_lowercase))
        .collect();
    features.sort();
    println!("cargo:rustc-env=RMK_FEATURES={}", features.join(","));
}
