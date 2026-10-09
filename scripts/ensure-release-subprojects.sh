#!/usr/bin/env bash
# Reconcile source submodules with the public GNOME release tags used by Gnoblin.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [ "$#" -gt 0 ]; then
    projects=("$@")
else
    projects=(mutter xdg-desktop-portal-gnome)
fi

for project in "${projects[@]}"; do
    case "$project" in
        mutter | xdg-desktop-portal-gnome) ;;
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

    # Checkout builds materialize the reviewed overlay and patch series in
    # this worktree. Accept that exact recorded state on later build stages;
    # subproject-state.sh rejects any edits outside the patch pipeline.
    if "$ROOT/scripts/subproject-state.sh" check "$project" "$tag" >/dev/null 2>&1; then
        continue
    fi

    if ! git -C "$subproject" rev-parse --verify "$tag^{commit}" >/dev/null 2>&1; then
        git -C "$subproject" fetch --quiet --depth=1 origin "refs/tags/$tag:refs/tags/$tag"
    fi
    expected="$(git -C "$subproject" rev-parse "$tag^{commit}")"
    actual="$(git -C "$ROOT" rev-parse "HEAD:subprojects/$project")"
    if [ "$actual" = "$expected" ]; then
        continue
    fi

    if [ "$project" = mutter ]; then
        expected_url="$("$ROOT/scripts/gnome-versions.py" get mutter source-url)"
        configured_url="$(git -C "$ROOT" config -f "$ROOT/.gitmodules" --get submodule.subprojects/mutter.url)"
        origin_url="$(git -C "$subproject" remote get-url origin)"
        checkout="$(git -C "$subproject" rev-parse HEAD)"
        status="$(git -C "$subproject" status --porcelain --untracked-files=all -- \
            . ':(exclude)subprojects/.wraplock')"
        if [ "$configured_url" = "$expected_url" ] &&
            [ "$origin_url" = "$expected_url" ] &&
            [ "$checkout" = "$actual" ] && [ -z "$status" ]; then
            continue
        fi
    fi

    echo "subproject $project is pinned at $actual, but release $tag names $expected" >&2
    if [ "$project" = mutter ]; then
        echo "Mutter must use either the GNOME release tag or the clean Gnoblin fork revision pinned by this checkout." >&2
    else
        echo "The superproject pin and release tag must agree; no checkout was overwritten." >&2
    fi
    exit 1
done
