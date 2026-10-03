#!/usr/bin/env bash
# Prepare a submodule checkout with gnoblin's changes for building.
#
# The submodules under subprojects/ are always kept at their pinned upstream
# tag. gnoblin changes come from two places, applied here at build time:
#   1. overlay source files (large new files) copied in via copy-overlay.sh
#   2. patches (edits to existing files / small additions) applied with git am
# This script resets the submodule to its tag first, so it is idempotent and
# never accumulates state.
set -euo pipefail

PROJ="${1:?usage: apply-patches.sh <mutter|xdg-desktop-portal-gnome>}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
case "$PROJ" in
    mutter | xdg-desktop-portal-gnome) ;;
    *)
        echo "unsupported source project: $PROJ" >&2
        exit 2
        ;;
esac
SM="$ROOT/subprojects/$PROJ"

TAG="$($ROOT/scripts/gnome-versions.py get "$PROJ" version)"

git -C "$SM" rev-parse --git-dir >/dev/null 2>&1 ||
    {
        echo "submodule $PROJ not initialised; run 'just init'" >&2
        exit 1
    }

"$ROOT/scripts/subproject-state.sh" check "$PROJ" "$TAG"
"$ROOT/scripts/manage-patches.py" check
"$ROOT/scripts/copy-overlay.sh" "$PROJ" "$SM" --remove-destinations

echo ">> resetting $PROJ to pristine tag $TAG"
git -C "$SM" am --abort >/dev/null 2>&1 || true
git -C "$SM" checkout -qf "$TAG"
git -C "$SM" reset -q --hard "$TAG"
git -C "$SM" clean -qfd

# 1. Copy overlay source files (new files we author) into the submodule.
"$ROOT/scripts/copy-overlay.sh" "$PROJ" "$SM"

# 2. Apply the patch series (edits to existing files).
mapfile -t PATCHES < <(find "$ROOT/patches/$PROJ" -name '*.patch' | sort)
echo ">> applying ${#PATCHES[@]} patch(es) to $PROJ"
if [ "${#PATCHES[@]}" -gt 0 ]; then
    printf '   %s\n' "${PATCHES[@]#$ROOT/}"
    # Preserve the patch author from each file's From: header. A clean build
    # environment may not have a configured committer, so supply a stable build
    # identity only when Git cannot resolve one from its ambient config.
    # The .patch files under patches/ are the source of truth; these commits are
    # only a staging step before `git archive`/tarball and are never pushed.
    if git -C "$SM" var GIT_COMMITTER_IDENT >/dev/null 2>&1; then
        git -C "$SM" am "${PATCHES[@]}"
    else
        git -C "$SM" -c user.name='Gnoblin Build' \
            -c user.email='builds@gnoblin.invalid' am "${PATCHES[@]}"
    fi
fi

"$ROOT/scripts/subproject-state.sh" record "$PROJ" "$TAG"

echo ">> $PROJ now at $(git -C "$SM" rev-parse --short HEAD) ($(git -C "$SM" log --oneline "$TAG..HEAD" | wc -l) patches on top of $TAG)"
