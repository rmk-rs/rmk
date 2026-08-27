#!/usr/bin/env bash
set -euo pipefail
# shellcheck source=_lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/_lib.sh"

# Bump the named crates, update the `=` pins that point at them, refresh the
# example lockfiles, and add rmk's CHANGELOG heading.
#
#     .github/ci/prepare-release.sh <major|minor|patch> [crate...]   # no crate = all

level="${1:?usage: prepare-release.sh <major|minor|patch> [crate...]}"
shift
if [[ $# -eq 0 ]]; then
    crates=("${RELEASE_BUMPABLE[@]}")
else
    crates=("$@")
fi

# Set a crate's own version, then update every `=` pin that names it.
set_version() {
    local crate="$1" version="$2" manifest name
    manifest="$(crate_manifest "$crate")"
    awk -v v="$version" '
        !done && /^version = "/ { print "version = \"" v "\""; done = 1; next }
        { print }
    ' "$manifest" > "$manifest.new"
    mv "$manifest.new" "$manifest"
    for name in "${RELEASE_CRATES[@]}"; do
        manifest="$(crate_manifest "$name")"
        awk -v n="$crate" -v v="$version" '
            $0 ~ "^" n " *= *\\{" {
                gsub(/version = "=[0-9]+\.[0-9]+\.[0-9]+"/, "version = \"=" v "\"")
            }
            { print }
        ' "$manifest" > "$manifest.new"
        mv "$manifest.new" "$manifest"
    done
}

log_section "Bumping versions"
for crate in "${crates[@]}"; do
    current="$(crate_version "$crate")"
    IFS=. read -r major minor patch <<< "$current"
    case "$level" in
        major) next="$((major + 1)).0.0" ;;
        minor) next="$major.$((minor + 1)).0" ;;
        patch) next="$major.$minor.$((patch + 1))" ;;
        *) echo "unknown bump level: $level" >&2; exit 1 ;;
    esac
    set_version "$crate" "$next"
    echo "$crate $current -> $next"
done

# Example lockfiles record rmk's version, so the `fetch-check` CI job fails if
# they are left stale.
log_section "Refreshing example lockfiles"
bash scripts/fetch_all.sh

# The publish workflow turns this section into the GitHub release notes.
# Skipped when it already exists.
log_section "Changelog"
version="$(crate_version rmk)"
if grep -q "^## \[$version\]" rmk/CHANGELOG.md; then
    echo "already has a $version section"
else
    awk -v v="$version" -v d="$(date -u +%F)" '
        $0 == "## [Unreleased]" { print; print ""; print "## [" v "] - " d; next }
        { print }
    ' rmk/CHANGELOG.md > rmk/CHANGELOG.new
    mv rmk/CHANGELOG.new rmk/CHANGELOG.md
    echo "rolled [Unreleased] into [$version]"
fi
