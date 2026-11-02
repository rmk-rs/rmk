#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

mode="${1:-all}"
case "$mode" in all|stable|nightly) ;; *) exit 2 ;; esac

if [[ "$mode" != nightly ]]; then
    log_section "Stable Clippy"
    # Clippy also type-checks each configuration; a separate cargo check
    # of these same manifests/features repeats the compilation.
    for feats in "${RMK_FEATURESETS[@]}"; do
        args=(--no-default-features)
        [[ -z "$feats" ]] || args+=(--features "$feats")
        log_check "Clippy: rmk, default features disabled, features: ${feats:-none}"
        cargo +stable clippy --target-dir "$target_root/check" \
            --manifest-path rmk/Cargo.toml "${args[@]}" -- -D warnings
    done
    for crate in rmk rmk-config rmk-macro rmk-types; do
        log_check "Clippy: $crate, default features"
        cargo +stable clippy --target-dir "$target_root/check" \
            --manifest-path "$crate/Cargo.toml" -- -D warnings
    done
fi

if [[ "$mode" != stable ]]; then
    log_section "Nightly smoke checks"
    log_check "Nightly: rmk, default features disabled, features: none"
    cargo +nightly check --target-dir "$target_root/nightly" \
        --manifest-path rmk/Cargo.toml --no-default-features
    log_check "Nightly: rmk, default features disabled, features: split,vial,storage,passkey_entry"
    cargo +nightly check --target-dir "$target_root/nightly" \
        --manifest-path rmk/Cargo.toml --no-default-features --features split,vial,storage,passkey_entry
fi
