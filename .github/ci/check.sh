#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

mode="${1:-all}"
shard="${2:-0}"
shards="${3:-1}"
case "$mode" in all|stable|nightly) ;; *) exit 2 ;; esac
[[ "$shard" =~ ^[0-9]+$ && "$shards" =~ ^[1-9][0-9]*$ && "$shard" -lt "$shards" ]] || exit 2

if [[ "$mode" != nightly ]]; then
    log_section "Stable Clippy (part $((shard + 1)) of $shards)"
    # Clippy also type-checks each configuration; a separate cargo check
    # of these same manifests/features repeats the compilation.
    for i in "${!RMK_FEATURESETS[@]}"; do
        (( i % shards == shard )) || continue
        args=(--no-default-features)
        [[ -z "${RMK_FEATURESETS[$i]}" ]] || args+=(--features "${RMK_FEATURESETS[$i]}")
        log_check "Clippy: rmk, default features disabled, features: ${RMK_FEATURESETS[$i]:-none}"
        cargo +stable clippy --target-dir "$target_root/check" \
            --manifest-path rmk/Cargo.toml "${args[@]}" -- -D warnings
    done
    crates=(rmk rmk-config rmk-macro rmk-types)
    for i in "${!crates[@]}"; do
        (( i % shards == shard )) || continue
        log_check "Clippy: ${crates[$i]}, default features"
        cargo +stable clippy --target-dir "$target_root/check" \
            --manifest-path "${crates[$i]}/Cargo.toml" -- -D warnings
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
