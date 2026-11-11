#!/bin/bash

# Exit on any error
set -euo pipefail

command -v cargo-nextest >/dev/null 2>&1 || {
    echo "cargo-nextest not found. Install with: cargo install cargo-nextest --locked"
    exit 1
}

source "$(dirname "${BASH_SOURCE[0]}")/../.github/ci/_lib.sh"

# Each crate in this repo is its own cargo workspace, so nextest's default
# `<workspace>/.config/nextest.toml` lookup would miss our shared config at
# repo root. Pass it explicitly.
nx=(nextest run --config-file "$repo_root/.config/nextest.toml")

# rmk-types: default-features run + host-feature run (the latter enables
# rynk/_ble/split/steno, which is required to compile the wire-format
# snapshot tests under src/protocol/rynk/snapshots/) + steno-only run.
cargo "${nx[@]}" --manifest-path rmk-types/Cargo.toml
cargo "${nx[@]}" --manifest-path rmk-types/Cargo.toml --features host
cargo "${nx[@]}" --manifest-path rmk-types/Cargo.toml --features steno

for feats in "${RMK_TEST_FEATURESETS[@]}"; do
    if [[ -z "$feats" ]]; then
        cargo "${nx[@]}" --manifest-path rmk/Cargo.toml --no-default-features
    else
        cargo "${nx[@]}" --manifest-path rmk/Cargo.toml --no-default-features --features "$feats"
    fi
done

# Doctests: nextest doesn't run them. rmk/ has `doctest = false` so only
# rmk-types needs a --doc pass.
cargo test --manifest-path rmk-types/Cargo.toml --doc
cargo test --manifest-path rmk-types/Cargo.toml --features host --doc

# Host tooling is a separate workspace; the CI script owns its check/test/
# clippy sequence, so run it as-is rather than duplicating it here.
"$repo_root/.github/ci/host.sh"
