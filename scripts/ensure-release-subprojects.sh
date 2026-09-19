#!/usr/bin/env bash
# Reconcile source submodules with the public GNOME release tags used by Gnoblin.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

declare -A tags=(
    [mutter]=49.5
    [gnome-shell]=49.6
    [gnome-control-center]=49.6
    [xdg-desktop-portal-gnome]=49.0
)

for project in "${!tags[@]}"; do
    subproject="$ROOT/subprojects/$project"
    tag="${tags[$project]}"

    [ -d "$subproject" ] || {
        echo "subproject $project is not initialised; run 'git submodule update --init --recursive'" >&2
        exit 1
    }

    git -C "$subproject" fetch --quiet origin "refs/tags/$tag:refs/tags/$tag"
    expected="$(git -C "$subproject" rev-parse "$tag^{commit}")"
    actual="$(git -C "$subproject" rev-parse HEAD)"
    if [ "$actual" = "$expected" ]; then
        continue
    fi

    echo "subproject $project is at $actual, but release $tag names $expected" >&2
    echo "The superproject pin and release tag must agree; no checkout was overwritten." >&2
    exit 1
done
