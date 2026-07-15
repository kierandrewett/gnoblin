#!/usr/bin/env bash
# Refuse destructive subproject resets unless the checkout is pristine or matches
# the exact state recorded by the last successful gnoblin workflow.
set -euo pipefail

ACTION="${1:?usage: subproject-state.sh <check|record> <project> <tag>}"
PROJECT="${2:?usage: subproject-state.sh <check|record> <project> <tag>}"
TAG="${3:?usage: subproject-state.sh <check|record> <project> <tag>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SUBPROJECT="$ROOT/subprojects/$PROJECT"
STATE_DIR="$ROOT/build/subproject-state"
STATE_FILE="$STATE_DIR/$PROJECT.sha256"

[ -d "$SUBPROJECT" ] || { echo "subproject not found: $PROJECT" >&2; exit 1; }

git -C "$SUBPROJECT" rev-parse --verify "$TAG^{commit}" >/dev/null 2>&1 || {
    echo "subproject $PROJECT has no tag $TAG; run 'just init'" >&2
    exit 1
}

snapshot() {
    {
        git -C "$SUBPROJECT" rev-parse HEAD
        git -C "$SUBPROJECT" diff --binary --no-ext-diff HEAD --
        while IFS= read -r -d '' path; do
            printf '%s\0' "$path"
            git -C "$SUBPROJECT" hash-object -- "$path"
        done < <(git -C "$SUBPROJECT" ls-files --others --exclude-standard -z -- \
            . ':(exclude)subprojects/.wraplock')
    } | sha256sum | cut -d ' ' -f 1
}

worktree_status() {
    git -C "$SUBPROJECT" status --porcelain --untracked-files=all -- \
        . ':(exclude)subprojects/.wraplock'
}

is_pristine() {
    [ "$(git -C "$SUBPROJECT" rev-parse HEAD)" = "$(git -C "$SUBPROJECT" rev-parse "$TAG^{commit}")" ] &&
        [ -z "$(worktree_status)" ]
}

case "$ACTION" in
    check)
        if [ "${GNOBLIN_FORCE_RESET:-0}" = 1 ]; then
            echo ">> WARNING: forcing destructive reset of $PROJECT" >&2
            exit 0
        fi

        current="$(snapshot)"
        if is_pristine || { [ -f "$STATE_FILE" ] && [ "$(cat "$STATE_FILE")" = "$current" ]; }; then
            exit 0
        fi

        echo "refusing to reset subproject $PROJECT: checkout contains unrecognised work" >&2
        worktree_status >&2
        echo "Move source changes into gnoblin overlays/patches, or review them and rerun with" >&2
        echo "GNOBLIN_FORCE_RESET=1 only when discarding this checkout is intentional." >&2
        exit 1
        ;;
    record)
        mkdir -p "$STATE_DIR"
        temporary="$STATE_FILE.tmp.$$"
        snapshot > "$temporary"
        mv -f "$temporary" "$STATE_FILE"
        ;;
    *)
        echo "unknown subproject state action: $ACTION" >&2
        exit 1
        ;;
esac
