#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

export CARGO_NET_OFFLINE=false
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$target_root/test}"
nx=(nextest run --config-file "$repo_root/.config/nextest.toml" --profile ci)

log_section "Configuration and shared types"
log_section "Tests: rmk-config, default features"
cargo +stable "${nx[@]}" --manifest-path rmk-config/Cargo.toml
log_section "Tests: rmk-types, default features"
cargo +stable "${nx[@]}" --manifest-path rmk-types/Cargo.toml
log_section "Tests: rmk-types, default features + host"
cargo +stable "${nx[@]}" --manifest-path rmk-types/Cargo.toml --features host
log_section "Tests: rmk-types, default features + steno"
cargo +stable "${nx[@]}" --manifest-path rmk-types/Cargo.toml --features steno

# nextest does not run doctests. rmk and rmk-macro disable them.
log_section "Doctests"
log_section "Doctests: rmk-types, default features"
cargo +stable test --manifest-path rmk-types/Cargo.toml --doc
log_section "Doctests: rmk-types, default features + host"
cargo +stable test --manifest-path rmk-types/Cargo.toml --features host --doc
log_section "Doctests: rmk-config, default features"
cargo +stable test --manifest-path rmk-config/Cargo.toml --doc

log_section "Macro tests and expansion snapshots"
log_section "Tests: rmk-macro, default features + _simulator,steno"
cargo +stable "${nx[@]}" --manifest-path rmk-macro/Cargo.toml --features _simulator,steno

log_section "RMK behavior"
for feats in "${RMK_TEST_FEATURESETS[@]}"; do
    args=(--no-default-features)
    [[ -z "$feats" ]] || args+=(--features "$feats")
    log_section "Tests: rmk, default features disabled, features: ${feats:-none}"
    cargo +stable "${nx[@]}" --manifest-path rmk/Cargo.toml "${args[@]}"
done
