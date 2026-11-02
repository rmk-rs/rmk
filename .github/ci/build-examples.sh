#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

export CARGO_TARGET_DIR="$repo_root/target/examples"
export RUSTFLAGS="-D warnings --remap-path-prefix=$repo_root=/rmk-build --remap-path-prefix=$CARGO_TARGET_DIR=/rmk-target"

if [[ "${COMPARE_BASE:-false}" == true ]]; then
    # Each runner owns one base checkout and reuses its dependencies across
    # the size comparisons in this group.
    base_worktree="$RUNNER_TEMP/rmk-base"
    git worktree add --detach "$base_worktree" "$BASE_SHA"
    trap 'git worktree remove --force "$base_worktree"' EXIT
    llvm_size="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-size"
fi

while IFS=$'\t' read -r dir bloat bloat_bins; do
    log_section "Build $dir"
    (cd "$dir" && cargo build --release)
    [[ "${COMPARE_BASE:-false}" == true && "$bloat" == true ]] || continue

    base_dir="$base_worktree/$dir"
    if [[ -d "$base_dir" ]]; then
        (cd "$base_dir" && CARGO_TARGET_DIR="$repo_root/target/base" \
            RUSTFLAGS="--remap-path-prefix=$base_worktree=/rmk-build --remap-path-prefix=$repo_root/target/base=/rmk-target" cargo build --release)
    fi

    outdir="$repo_root/bloat-data/${dir//\//_}"
    mkdir -p "$outdir"
    if [[ -n "$bloat_bins" ]]; then
        IFS=',' read -ra bins <<< "$bloat_bins"
    else
        bins=("$(cargo metadata --no-deps --format-version 1 --manifest-path "$dir/Cargo.toml" |
            jq -r --arg manifest "$repo_root/$dir/Cargo.toml" \
                '.packages[] | select(.manifest_path == $manifest) | .targets[] | select(.kind | index("bin")) | .name')")
    fi
    for bin in "${bins[@]}"; do
        label=""
        suffix=""
        if [[ -n "$bloat_bins" ]]; then label="$bin"; suffix="-$bin"; fi
        head_elf="$CARGO_TARGET_DIR/$TARGET/release/$bin"
        "$llvm_size" "$head_elf" > "$outdir/size-head$suffix.txt"
        read -r h_text h_data h_bss head_size < <(awk 'NR==2 {print $1, $2, $3, $4}' "$outdir/size-head$suffix.txt")
        if [[ -d "$base_dir" ]]; then
            base_elf="$repo_root/target/base/$TARGET/release/$bin"
            "$llvm_size" "$base_elf" > "$outdir/size-base$suffix.txt"
            read -r b_text b_data b_bss base_size < <(awk 'NR==2 {print $1, $2, $3, $4}' "$outdir/size-base$suffix.txt")
            d_text=$((h_text - b_text))
            d_data=$((h_data - b_data))
            d_bss=$((h_bss - b_bss))
            bloaty "$head_elf" -- "$base_elf" > "$outdir/bloaty$suffix.txt" 2>&1 || true
        else
            base_size=0; d_text=0; d_data=0; d_bss=0
            echo "(base not available)" > "$outdir/size-base$suffix.txt"
        fi
        echo "$dir|$label|$base_size|$head_size|$d_text|$d_data|$d_bss" >> "$outdir/size-data.txt"
    done
    cat "$outdir/size-data.txt"
done < <(jq -r '.[] | [.dir, .bloat, .bloat_bins] | @tsv' <<< "$EXAMPLES")
