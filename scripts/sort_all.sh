#!/usr/bin/env bash
set -euo pipefail

# Run cargo-sort on every tracked Cargo.toml.
#
# --grouped sorts within each blank-line-separated block without moving the
# blocks, so curated ordering like `rmk` first in the examples is preserved.
#
# Usage:
#   scripts/sort_all.sh           # rewrite manifests in place
#   scripts/sort_all.sh --check   # fail if any manifest is unsorted

source "$(dirname "${BASH_SOURCE[0]}")/../.github/ci/_lib.sh"

usage() {
    echo "Usage: $0 [--check]"
    echo "Sort Cargo.toml dependency tables in every tracked manifest."
    echo ""
    echo "Options:"
    echo "  --check    Fail if a manifest is unsorted instead of rewriting it"
    echo "  --help     Show this help message and exit"
}

case "${1:-}" in
    --check) mode=check ;;
    --help | -h)
        usage
        exit 0
        ;;
    "") mode=sort ;;
    *)
        usage >&2
        exit 2
        ;;
esac

mapfile -t manifests < <(list_all_manifests)

if [ "$mode" = check ]; then
    log_section "Checking Cargo.toml sort order"
    cargo sort --check --check-format --grouped "${manifests[@]}"
else
    log_section "Sorting Cargo.toml files"
    cargo sort --grouped "${manifests[@]}"
fi
