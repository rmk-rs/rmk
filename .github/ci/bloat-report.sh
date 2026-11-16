#!/usr/bin/env bash
# Assemble a bloat report from size-data artifact files.
# Input directory structure per example:
#   <safe_name>/size-data.txt       — dir|bin_label|base_total|head_total|text_diff|data_diff|bss_diff
#   <safe_name>/size-head[-bin].txt — full LLVM size output (HEAD)
#   <safe_name>/size-base[-bin].txt — full LLVM size output (BASE)
#   <safe_name>/bloaty[-bin].txt    — bloaty diff output
#
# Writes markdown to stdout and $GITHUB_STEP_SUMMARY.
set -euo pipefail

artifact_dir="${1:?Usage: bloat-report.sh <artifact-dir>}"

# Format bytes as human-readable KiB with one decimal.
fmt_size() {
    awk "BEGIN { printf \"%.1f KiB\", $1 / 1024 }"
}

# Format diff percentage with sign and emoji.
fmt_diff() {
    local base=$1 head=$2
    if (( base == 0 )); then
        echo "new"
        return
    fi
    local diff=$((head - base))
    local pct_x100=$(( diff * 10000 / base ))
    local sign="+" whole frac
    (( pct_x100 < 0 )) && sign="-" && pct_x100=$(( -pct_x100 ))
    whole=$(( pct_x100 / 100 ))
    frac=$(( pct_x100 % 100 ))

    local indicator=""
    (( diff > 0 )) && indicator=" ⬆️"
    (( diff < 0 )) && indicator=" ⬇️"

    printf "%s%d.%02d%%%s" "$sign" "$whole" "$frac" "$indicator"
}

# Format a byte diff with sign (e.g. "+688", "-32", "0").
fmt_bytes_diff() {
    local d=$1
    if (( d > 0 )); then printf "+%d" "$d"
    elif (( d < 0 )); then printf "%d" "$d"
    else printf "0"
    fi
}

# Collect all size-data entries: dir|bin_label|base|head|d_text|d_data|d_bss|safe_name
entries=()
for f in $(find "$artifact_dir" -name 'size-data.txt' -type f | sort); do
    safe_name="$(basename "$(dirname "$f")")"
    while IFS='|' read -r dir bin_label base_size head_size d_text d_data d_bss; do
        entries+=("$dir|$bin_label|$base_size|$head_size|${d_text:-0}|${d_data:-0}|${d_bss:-0}|$safe_name")
    done < "$f"
done

{
    # ── Overview table ──
    echo "## Size Report"
    echo
    echo "| Example | main | PR | Diff | .text | .data | .bss |"
    echo "| :--- | ---: | ---: | ---: | ---: | ---: | ---: |"

    for entry in "${entries[@]}"; do
        IFS='|' read -r dir bin_label base_size head_size d_text d_data d_bss safe_name <<< "$entry"
        label="${dir#examples/}"
        [[ -n "$bin_label" ]] && label="$label ($bin_label)"
        printf "| \`%s\` | %s | %s | %s | %s | %s | %s |\n" \
            "$label" \
            "$(fmt_size "$base_size")" \
            "$(fmt_size "$head_size")" \
            "$(fmt_diff "$base_size" "$head_size")" \
            "$(fmt_bytes_diff "$d_text")" \
            "$(fmt_bytes_diff "$d_data")" \
            "$(fmt_bytes_diff "$d_bss")"
    done

    echo

    # ── Collapsible details per example ──
    prev_safe=""
    for entry in "${entries[@]}"; do
        IFS='|' read -r dir bin_label base_size head_size _d_text _d_data _d_bss safe_name <<< "$entry"
        label="${dir#examples/}"
        [[ -n "$bin_label" ]] && label="$label ($bin_label)"

        bin_suffix=""
        [[ -n "$bin_label" ]] && bin_suffix="-$bin_label"

        echo "<details>"
        echo "<summary><code>$label</code> — $(fmt_size "$base_size") → $(fmt_size "$head_size") ($(fmt_diff "$base_size" "$head_size"))</summary>"
        echo

        # LLVM size (PR)
        size_head="$artifact_dir/$safe_name/size-head${bin_suffix}.txt"
        if [[ -f "$size_head" ]]; then
            echo "**LLVM size (PR):**"
            echo '```'
            cat "$size_head"
            echo '```'
            echo
        fi

        # LLVM size (main)
        size_base="$artifact_dir/$safe_name/size-base${bin_suffix}.txt"
        if [[ -f "$size_base" ]]; then
            echo "**LLVM size (main):**"
            echo '```'
            cat "$size_base"
            echo '```'
            echo
        fi

        # bloaty diff
        bloaty_file="$artifact_dir/$safe_name/bloaty${bin_suffix}.txt"
        if [[ -f "$bloaty_file" ]]; then
            echo "**Bloaty diff (PR vs main):**"
            echo '```'
            cat "$bloaty_file"
            echo '```'
            echo
        fi

        echo "</details>"
        echo
    done
}
