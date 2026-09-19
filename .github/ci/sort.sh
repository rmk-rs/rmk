#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

log_section "Checking Cargo.toml sort order"
# --check-format also flags formatting drift. --grouped sorts within each
# blank-line-separated block without moving the blocks, so curated ordering
# like `rmk` first in the examples is preserved.
mapfile -t manifests < <(list_all_manifests)
cargo sort --check --check-format --grouped "${manifests[@]}"
