#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

# Emit "--- <cmd> ..." tuples for rmk (every feature set) plus the other
# workspace crates. Tokens are literal or feature lists with no whitespace,
# so relying on word-splitting at the call site is safe.
emit_batch_args() {
    local cmd="$1"
    local feats
    for feats in "${RMK_FEATURESETS[@]}"; do
        if [[ -z "$feats" ]]; then
            printf -- '--- %s --manifest-path rmk/Cargo.toml --no-default-features\n' "$cmd"
        else
            printf -- '--- %s --manifest-path rmk/Cargo.toml --no-default-features --features %s\n' "$cmd" "$feats"
        fi
    done
    printf -- '--- %s --manifest-path rmk/Cargo.toml\n' "$cmd"
    printf -- '--- %s --manifest-path rmk-config/Cargo.toml\n' "$cmd"
    printf -- '--- %s --manifest-path rmk-macro/Cargo.toml\n' "$cmd"
    printf -- '--- %s --manifest-path rmk-types/Cargo.toml\n' "$cmd"
}

log_section "Stable checks"
# shellcheck disable=SC2046
cargo +stable batch --target-dir "$target_root/check-stable" $(emit_batch_args check)

log_section "Nightly smoke checks"
cargo +nightly batch --target-dir "$target_root/check-nightly" \
    --- check --manifest-path rmk/Cargo.toml --no-default-features \
    --- check --manifest-path rmk/Cargo.toml --no-default-features --features split,vial,storage,passkey_entry

log_section "Clippy"
# cargo-batch only exposes a subset of cargo subcommands (build, check, ...) —
# `clippy` is not supported, so we run it serially. Dedicated target dir
# prevents conflicts with the check-stable fingerprints above.
clippy_target="$target_root/clippy-stable"
clippy_rmk() {
    local feats="$1"
    if [[ -z "$feats" ]]; then
        cargo +stable clippy --target-dir "$clippy_target" \
            --manifest-path rmk/Cargo.toml --no-default-features -- -D warnings
    else
        cargo +stable clippy --target-dir "$clippy_target" \
            --manifest-path rmk/Cargo.toml --no-default-features --features "$feats" -- -D warnings
    fi
}
for feats in "${RMK_FEATURESETS[@]}"; do
    clippy_rmk "$feats"
done
cargo +stable clippy --target-dir "$clippy_target" --manifest-path rmk-config/Cargo.toml -- -D warnings
cargo +stable clippy --target-dir "$clippy_target" --manifest-path rmk-macro/Cargo.toml -- -D warnings
cargo +stable clippy --target-dir "$clippy_target" --manifest-path rmk-types/Cargo.toml -- -D warnings
