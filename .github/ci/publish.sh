#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

# Publish every crate whose version is not on crates.io yet, in dependency
# order, then tag them. Skipping published versions makes a re-run safe.

log_section "Git dependencies"
# crates.io rejects a crate with a git dependency, and nothing else checks.
for name in "${RELEASE_CRATES[@]}"; do
    manifest="$(crate_manifest "$name")"
    if grep -nE '^[^#]*[[:space:]]git[[:space:]]*=[[:space:]]*"' "$manifest"; then
        echo "::error file=$manifest::$name has a git dependency and cannot be published"
        exit 1
    fi
done
echo "none"

for name in "${RELEASE_CRATES[@]}"; do
    version="$(crate_version "$name")"
    if crate_published "$name" "$version"; then
        log_section "$name $version is already published, skipping"
        continue
    fi
    log_section "Publishing $name $version"
    # rynk/Cargo.toml is a workspace root, so without `-p` cargo would publish
    # every member of it at once, out of order and with no index wait.
    cargo publish -p "$name" --registry crates-io --manifest-path "$(crate_manifest "$name")"
    wait_for_index "$name" "$version"
done

log_section "Tagging"
git config user.name "github-actions[bot]"
git config user.email "41898282+github-actions[bot]@users.noreply.github.com"
for name in "${RELEASE_CRATES[@]}"; do
    version="$(crate_version "$name")"
    tag="$name-v$version"
    if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then continue; fi
    # A crate that never made it to crates.io must not get a tag saying it did.
    crate_published "$name" "$version" || continue
    git tag -a "$tag" -m "$name $version"
    echo "created $tag"
done
git push origin --tags

log_section "GitHub release"
version="$(crate_version rmk)"
if ! crate_published rmk "$version"; then
    echo "rmk $version is not on crates.io, nothing to release"
elif gh release view "rmk-v$version" >/dev/null 2>&1; then
    echo "release rmk-v$version already exists"
else
    gh release create "rmk-v$version" --title "RMK v$version" --notes "$(
        awk -v heading="## [$version]" '
            index($0, heading) == 1 { inside = 1; next }
            inside && /^## \[/ { exit }
            inside { print }
        ' rmk/CHANGELOG.md
    )"
fi
