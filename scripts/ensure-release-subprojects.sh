#!/usr/bin/env bash
# Reconcile source submodules with the public GNOME release tags used by Gnoblin.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ "$#" -gt 0 ]; then
    projects=("$@")
else
    projects=(mutter gnome-shell xdg-desktop-portal-gnome)
fi

for project in "${projects[@]}"; do
    case "$project" in
        mutter | gnome-shell | xdg-desktop-portal-gnome) ;;
        *)
            echo "Unknown source project: $project" >&2
            exit 2
            ;;
    esac
    subproject="$ROOT/subprojects/$project"
    tag="$($ROOT/scripts/gnome-versions.py get "$project" version)"

    git -C "$subproject" rev-parse --git-dir >/dev/null 2>&1 || {
        echo "subproject $project is not initialised; run 'git submodule update --init --recursive'" >&2
        exit 1
    }

    if ! git -C "$subproject" rev-parse --verify "$tag^{commit}" >/dev/null 2>&1; then
        git -C "$subproject" fetch --quiet --depth=1 origin "refs/tags/$tag:refs/tags/$tag"
    fi
    expected="$(git -C "$subproject" rev-parse "$tag^{commit}")"
    actual="$(git -C "$ROOT" rev-parse "HEAD:subprojects/$project")"
    if [ "$actual" = "$expected" ]; then
        continue
    fi

    echo "subproject $project is pinned at $actual, but release $tag names $expected" >&2
    echo "The superproject pin and release tag must agree; no checkout was overwritten." >&2
    exit 1
done
