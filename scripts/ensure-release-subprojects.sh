#!/usr/bin/env bash
# Reconcile source submodules with the public GNOME release tags used by Gnoblin.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/retry-command.sh"

projects=(mutter gnome-shell gnome-control-center xdg-desktop-portal-gnome)

for project in "${projects[@]}"; do
    subproject="$ROOT/subprojects/$project"
    tag="$($ROOT/scripts/gnome-versions.py get "$project" version)"

    [ -d "$subproject" ] || {
        echo "subproject $project is not initialised; run 'git submodule update --init --recursive'" >&2
        exit 1
    }

    gnoblin_retry_command git -C "$subproject" fetch --quiet --depth=1 origin "refs/tags/$tag:refs/tags/$tag"
    expected="$(git -C "$subproject" rev-parse "$tag^{commit}")"
    actual="$(git -C "$subproject" rev-parse HEAD)"
    if [ "$actual" = "$expected" ]; then
        continue
    fi

    echo "subproject $project is at $actual, but release $tag names $expected" >&2
    echo "The superproject pin and release tag must agree; no checkout was overwritten." >&2
    exit 1
done
