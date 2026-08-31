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
    "split,vial,storage,async_matrix,_ble,subrating"
    "rynk,_ble,split,storage,async_matrix"
    "rynk,storage"
    "rynk"
    "rynk,_ble,storage"
    "dongle,_ble,storage"
    "dongle,rynk,split,_ble,storage"
    "dongle,rynk,_ble,storage"
    "dongle,vial,split,_ble,storage"
    "dongle,vial,_ble,storage"
)

# Behavioral coverage only; RMK_FEATURESETS remains the compile/clippy matrix.
RMK_TEST_FEATURESETS=(
    ""
    "vial,host_lock,_no_usb,steno,passkey_entry"
    "rynk,_ble,split,async_matrix,storage"
    "dongle,_ble,storage"
    "dongle,vial,_ble,storage"
)

# Examples auto-discovery skiplist. Reasons:
#   - nrf54lm20_ble: Cargo.toml references local path deps that only exist on
#     the author's workstation.
#   - esp32_ble_split: dual-target split example; only builds through the
#     `build-central` / `build-peripheral` cargo aliases.
#   - py32f07x, sf32lb52x_usb: not currently buildable in CI.
#   - sf32lb52x_ble: sifli-radio pins bt-hci 0.8 while rmk needs bt-hci 0.10, so its
#     BleController doesn't satisfy rmk's Controller traits. Document-and-wait (no
#     sifli-rs fork) until sifli-radio ships a bt-hci version rmk can use.
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

# Publish order. A crate's `=X.Y.Z` pins are resolved from crates.io, so
# everything it depends on has to be published before it.
RELEASE_CRATES=(
    rmk-config rmk-types rmk-macro rmk
    rynk rynk-usb rynk-ble rynk-kle rynk-wasm
)

# Every crate carries its own version, so any of them can be released alone.
RELEASE_BUMPABLE=("${RELEASE_CRATES[@]}")

# release-pr.yml opens its PR from this branch, and publish.yml publishes only
# commits that arrived through it.
RELEASE_BRANCH=release/prepare

# The rynk host crates sit under rynk/; every other crate is a top-level
# directory named after itself.
crate_manifest() {
    case "$1" in
        rynk-*) printf 'rynk/%s/Cargo.toml\n' "$1" ;;
        *) printf '%s/Cargo.toml\n' "$1" ;;
    esac
}

# A package's own version is the only `version = "..."` at column 0; dependency
# versions are either indented or inline as `name = { version = ... }`.
crate_version() {
    awk -F'"' '/^version = "/ { print $2; exit }' "$(crate_manifest "$1")"
}

# The version $1 becomes after a major, minor or patch bump.
bump_version() {
    local major minor patch
    IFS=. read -r major minor patch <<< "$1"
    case "$2" in
        major) printf '%s.0.0\n' "$((major + 1))" ;;
        minor) printf '%s.%s.0\n' "$major" "$((minor + 1))" ;;
        patch) printf '%s.%s.%s\n' "$major" "$minor" "$((patch + 1))" ;;
        *) echo "unknown bump level: $2" >&2; return 1 ;;
    esac
}

# The version requirement $2 declares on $1, empty if it does not depend on it.
crate_pin() {
    sed -nE "s/^$1 *= *\{.*version *= *\"([^\"]*)\".*/\1/p" "$(crate_manifest "$2")"
}

# Whether bumping $1 by $2 moves the requirement $3 declares on it. An exact pin
# always moves; a caret pin only when the bump crosses its major.minor.
pin_moves() {
    local pin next
    pin="$(crate_pin "$1" "$3")"
    [ -n "$pin" ] || return 1
    next="$(bump_version "$(crate_version "$1")" "$2")"
    case "$pin" in
        "="*) return 0 ;;
        *) [ "$pin" != "${next%.*}" ] ;;
    esac
}

# The crates named in $2.. plus every dependent a $1 bump would drag along, in
# publish order. A dependent whose requirement moves but whose own version does
# not would publish nothing, leaving crates.io on the version the fix replaced.
release_closure() {
    local level="$1" list name dep grown=1
    shift
    list=" $* "
    for name in "$@"; do
        case " ${RELEASE_BUMPABLE[*]} " in
            *" $name "*) continue ;;
        esac
        echo "unknown crate: $name (releasable: ${RELEASE_BUMPABLE[*]})" >&2
        return 1
    done
    while [ "$grown" = 1 ]; do
        grown=0
        for name in "${RELEASE_BUMPABLE[@]}"; do
            case "$list" in *" $name "*) continue ;; esac
            for dep in $list; do
                if pin_moves "$dep" "$level" "$name"; then
                    list="$list$name "
                    grown=1
                    break
                fi
            done
        done
    done
    for name in "${RELEASE_BUMPABLE[@]}"; do
        case "$list" in *" $name "*) printf '%s\n' "$name" ;; esac
    done
}

# A crate's sparse-index URL. crates.io groups names by length: 1, 2, first
# letter, then the first two pairs of letters.
crate_index_url() {
    local name="$1"
    case "${#name}" in
        1) printf 'https://index.crates.io/1/%s\n' "$name" ;;
        2) printf 'https://index.crates.io/2/%s\n' "$name" ;;
        3) printf 'https://index.crates.io/3/%s/%s\n' "${name:0:1}" "$name" ;;
        *) printf 'https://index.crates.io/%s/%s/%s\n' "${name:0:2}" "${name:2:2}" "$name" ;;
    esac
}

# Succeeds when this exact version is live on crates.io.
crate_published() {
    local body
    body="$(curl -sf --retry 3 --retry-connrefused "$(crate_index_url "$1")")" || return 1
    grep -qF "\"vers\":\"$2\"" <<< "$body"
}

# The index lags a publish by up to a few minutes, and the next crate cannot
# resolve its pin until it catches up.
wait_for_index() {
    local name="$1" version="$2" waited=0
    until crate_published "$name" "$version"; do
        if (( waited >= 600 )); then
            echo "timed out after ${waited}s waiting for $name $version" >&2
            return 1
        fi
        sleep 15
        waited=$(( waited + 15 ))
        printf 'waiting for %s %s on crates.io (%ss)\n' "$name" "$version" "$waited"
    done
}
