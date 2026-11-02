#!/usr/bin/env bash
# Discover buildable examples and emit GitHub Actions matrix JSON.
# Outputs:
#   stable  — target-specific groups of examples sharing one build directory
#   esp     — JSON array of {dir, target} for xtensa/ESP examples
set -euo pipefail

# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"


# Examples tracked for binary-size regression (bloat) reports on PRs.
BLOAT_DIRS=(
    # use_config — TOML-driven path
    "examples/use_config/nrf52832_ble"
    "examples/use_config/nrf52840_ble"
    "examples/use_config/nrf52840_ble_split"
    "examples/use_config/rp2040"
    "examples/use_config/rp2040_split"
    "examples/use_config/pi_pico_w_ble"
    "examples/use_config/stm32f1"
    "examples/use_config/stm32h7"
    # use_rust — pure-Rust API path (mirrors use_config set)
    "examples/use_rust/nrf52832_ble"
    "examples/use_rust/nrf52840_ble"
    "examples/use_rust/nrf52840_ble_split"
    "examples/use_rust/rp2040"
    "examples/use_rust/rp2040_split"
    "examples/use_rust/pi_pico_w_ble"
    "examples/use_rust/stm32f1"
    "examples/use_rust/stm32h7"
)

# nrf52840_ble_split / rp2040_split produce two binaries; list them comma-separated.
bloat_bins_for() {
    case "$1" in
        */nrf52840_ble_split|*/rp2040_split) echo "central,peripheral" ;;
        *) echo "" ;;
    esac
}

is_bloat_dir() {
    local d
    for d in "${BLOAT_DIRS[@]}"; do
        [[ "$1" == "$d" ]] && return 0
    done
    return 1
}

stable='['
esp='['
first_stable=1
first_esp=1

while IFS= read -r manifest; do
    dir="$(dirname "$manifest")"
    target="$(get_example_target "$manifest")"
    [[ -z "$target" ]] && continue

    case "$dir" in
        */esp32s3_ble)
            entry="{\"dir\":\"$dir\",\"target\":\"$target\"}"
            (( first_esp )) && first_esp=0 || esp+=','
            esp+="$entry"
            ;;
        *)
            bloat=false
            bins=""
            if is_bloat_dir "$dir"; then
                bloat=true
                bins="$(bloat_bins_for "$dir")"
            fi
            entry="{\"dir\":\"$dir\",\"target\":\"$target\",\"bloat\":$bloat,\"bloat_bins\":\"$bins\"}"
            (( first_stable )) && first_stable=0 || stable+=','
            stable+="$entry"
            ;;
    esac
done < <(list_example_manifests)

stable+=']'
esp+=']'

# Group small builds to share dependencies and runner setup. Bloat entries
# compile both HEAD and BASE, so give them twice the scheduling weight.
stable=$(EXAMPLES="$stable" python3 - <<'PY'
import collections
import json
import math
import os

targets = collections.defaultdict(list)
for example in json.loads(os.environ["EXAMPLES"]):
    targets[example["target"]].append(example)
groups = []
for target, examples in targets.items():
    count = math.ceil(sum(2 if e["bloat"] else 1 for e in examples) / 4)
    buckets = [[] for _ in range(count)]
    weights = [0] * count
    for example in sorted(examples, key=lambda e: (not e["bloat"], e["dir"])):
        index = min(range(count), key=lambda i: weights[i])
        buckets[index].append(example)
        weights[index] += 2 if example["bloat"] else 1
    for index, bucket in enumerate(buckets):
        groups.append({
            "name": f"{target}-{index}",
            "target": target,
            "examples": bucket,
            "arm_gcc": any("nrf" in e["dir"] for e in bucket),
            "bloat": any(e["bloat"] for e in bucket),
            "workspaces": "\n".join(
                e["dir"] + " -> " + os.path.relpath("target/examples", e["dir"])
                for e in bucket
            ),
        })
print(json.dumps(groups, separators=(",", ":")))
PY
)
echo "stable=$stable" >> "$GITHUB_OUTPUT"
echo "esp=$esp" >> "$GITHUB_OUTPUT"

# Debug: show what was discovered
echo "Stable examples: $(echo "$stable" | grep -o '"dir"' | wc -l | tr -d ' ')"
echo "ESP examples: $(echo "$esp" | grep -o '"dir"' | wc -l | tr -d ' ')"
echo "Bloat examples: $(jq '[.[].examples[] | select(.bloat)] | length' <<< "$stable")"
