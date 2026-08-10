# shellcheck shell=bash
#
# Shared bootstrap for RMK CI scripts. Source this from other scripts in
# .github/ci/ to pick up common env and example discovery helpers.
#
#     source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"
#
# Expected preamble in the caller:
#
#     #!/bin/bash
#     set -euo pipefail
#
# Toolchain + tool installation (rustup components/targets, cargo-batch,
# cargo-expand, espup) is the workflow's responsibility and lives in
# .github/workflows/ci.yml. Locally the repo's rust-toolchain.toml takes
# care of it, so these scripts stay side-effect-free on your machine.

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$repo_root"

export CARGO_TERM_COLOR=always
export CARGO_TERM_PROGRESS_WHEN=never
export CARGO_NET_GIT_FETCH_WITH_CLI=true
export TERM="${TERM:-dumb}"

# Shared parent for CI target directories. Cargo creates each target directory
# itself so it also writes the CACHEDIR.TAG required by `cargo clean`.
target_root="$repo_root/target/ci"

log_section() {
    printf "\n==> %s\n" "$1"
}

# Broad rmk compile/clippy matrix; empty means only `--no-default-features`.
RMK_FEATURESETS=(
    ""
    "log,std"
    "storage"
    "async_matrix,storage"
    "vial,host_lock,storage"
    "vial,_ble"
    "vial,_ble,_no_usb,steno,passkey_entry"
    "split,async_matrix"
    "split,async_matrix,_ble"
    "split,vial,async_matrix"
    "split,vial,async_matrix,_ble"
    "split,vial,storage"
    "passkey_entry"
    "split,vial,storage,passkey_entry"
    "vial,storage,steno"
    "split,vial,storage,async_matrix,_ble,steno"
    "rynk,_ble,split,storage,async_matrix"
    "rynk,storage"
    "rynk"
    "rynk,_ble,storage"
    "dongle,_ble,storage"
    "dongle,rynk,split,_ble,storage"
    "dongle,vial,_ble,storage"
)

# Behavioral coverage only; RMK_FEATURESETS remains the compile/clippy matrix.
RMK_TEST_FEATURESETS=(
    ""
    "vial,host_lock,_no_usb,steno,passkey_entry"
    "rynk,_ble,split,async_matrix,storage"
    "dongle,_ble,storage"
)

# Examples auto-discovery skiplist. Reasons:
#   - nrf54lm20_ble: Cargo.toml references local path deps that only exist on
#     the author's workstation.
#   - esp32_ble_split: dual-target split example; only builds through the
#     `build-central` / `build-peripheral` cargo aliases.
#   - py32f07x, sf32lb52x_usb: not currently buildable in CI.
#   - sf32lb52x_ble: sifli-radio pins bt-hci 0.8 while rmk needs bt-hci 0.9, so its
#     BleController doesn't satisfy rmk's Controller traits. Document-and-wait (no
#     sifli-rs fork) until sifli-radio ships bt-hci 0.9.
EXAMPLE_SKIPLIST=(
    "examples/use_rust/nrf54lm20_ble"
    "examples/use_config/esp32_ble_split"
    "examples/use_rust/py32f07x"
    "examples/use_rust/sf32lb52x_usb"
    "examples/use_rust/sf32lb52x_ble"
)

# Multi-target examples (several boards in one directory) sit one level
# deeper than the discovery glob; list their crates explicitly.
EXTRA_EXAMPLE_MANIFESTS=(
    "examples/use_rust/nrf_dongle/dongle/Cargo.toml"
    "examples/use_rust/nrf_dongle/central/Cargo.toml"
    "examples/use_rust/nrf_dongle/peripheral/Cargo.toml"
)

# Echoes Cargo.toml paths for every buildable example, one per line.
# A buildable example is a direct child of examples/use_{rust,config}/ that
# has both a src/ dir and a Cargo.toml (filters out placeholders like fix/),
# and is not listed in EXAMPLE_SKIPLIST.
list_example_manifests() {
    local dir stripped skip entry
    for dir in examples/use_rust/*/ examples/use_config/*/; do
        [[ -d "$dir/src" && -f "$dir/Cargo.toml" ]] || continue
        stripped="${dir%/}"
        skip=0
        for entry in "${EXAMPLE_SKIPLIST[@]}"; do
            if [[ "$stripped" == "$entry" ]]; then
                skip=1
                break
            fi
        done
        (( skip == 0 )) && printf '%s\n' "${dir}Cargo.toml"
    done
    local extra
    for extra in "${EXTRA_EXAMPLE_MANIFESTS[@]}"; do
        [[ -f "$extra" ]] && printf '%s\n' "$extra"
    done
}

# Echoes the default build target triple declared in the manifest's sibling
# .cargo/config.toml ([build].target). Only the first uncommented occurrence
# is emitted; returns empty if the file or the key is absent. Trailing
# TOML comments on the value are stripped.
get_example_target() {
    local manifest="$1"
    local dir config
    dir="$(dirname "$manifest")"
    config="$dir/.cargo/config.toml"
    [[ -f "$config" ]] || return 0
    awk '
        /^\[/ { section = $0; next }
        section == "[build]" && /^[[:space:]]*target[[:space:]]*=/ {
            sub(/^[[:space:]]*target[[:space:]]*=[[:space:]]*/, "")
            sub(/[[:space:]]*#.*$/, "")
            sub(/^"/, "")
            sub(/"[[:space:]]*$/, "")
            print
            exit
        }
    ' "$config"
}
