#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

group="${1:-all}"
shard="${2:-0}"
shards="${3:-1}"
case "$group" in all|types|macro|rmk) ;; *) exit 2 ;; esac
[[ "$shard" =~ ^[0-9]+$ && "$shards" =~ ^[1-9][0-9]*$ && "$shard" -lt "$shards" ]] || exit 2

export CARGO_NET_OFFLINE=false
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$target_root/test}"
nx=(nextest run --config-file "$repo_root/.config/nextest.toml" --profile ci)

if [[ "$group" == all || "$group" == types ]]; then
    log_section "Configuration and shared types"
    log_check "Tests: rmk-config, default features"
    cargo +stable "${nx[@]}" --manifest-path rmk-config/Cargo.toml
    log_check "Tests: rmk-types, default features"
    cargo +stable "${nx[@]}" --manifest-path rmk-types/Cargo.toml
    log_check "Tests: rmk-types, default features + host"
    cargo +stable "${nx[@]}" --manifest-path rmk-types/Cargo.toml --features host
    log_check "Tests: rmk-types, default features + steno"
    cargo +stable "${nx[@]}" --manifest-path rmk-types/Cargo.toml --features steno

    # nextest does not run doctests. rmk and rmk-macro disable them.
    log_section "Doctests"
    log_check "Doctests: rmk-types, default features"
    cargo +stable test --manifest-path rmk-types/Cargo.toml --doc
    log_check "Doctests: rmk-types, default features + host"
    cargo +stable test --manifest-path rmk-types/Cargo.toml --features host --doc
    log_check "Doctests: rmk-config, default features"
    cargo +stable test --manifest-path rmk-config/Cargo.toml --doc
fi

if [[ "$group" == all || "$group" == macro ]]; then
    log_section "Macro tests and expansion snapshots"
    log_check "Tests: rmk-macro, default features + _simulator,steno"
    cargo +stable "${nx[@]}" --manifest-path rmk-macro/Cargo.toml --features _simulator,steno
fi

if [[ "$group" == all || "$group" == rmk ]]; then
    log_section "RMK behavior (part $((shard + 1)) of $shards)"
    for i in "${!RMK_TEST_FEATURESETS[@]}"; do
        (( i % shards == shard )) || continue
        args=(--no-default-features)
        [[ -z "${RMK_TEST_FEATURESETS[$i]}" ]] || args+=(--features "${RMK_TEST_FEATURESETS[$i]}")
        log_check "Tests: rmk, default features disabled, features: ${RMK_TEST_FEATURESETS[$i]:-none}"
        cargo +stable "${nx[@]}" --manifest-path rmk/Cargo.toml "${args[@]}"
    done
fi
