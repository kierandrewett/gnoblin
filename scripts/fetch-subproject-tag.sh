#!/usr/bin/env bash
# Make sure a shallow submodule checkout knows its pinned upstream tag.
#
# The submodules under subprojects/ are cloned with `shallow = true`, so
# `git submodule update --init` fetches only the pinned commit and no tags.
# Every reset/patch step resolves the pinned tag by name, so fetch exactly
# that tag (and nothing else) when it is missing. The tag peels to the commit
# already present, so this is a tiny transfer.
set -euo pipefail

PROJ="${1:?usage: fetch-subproject-tag.sh <project> <tag>}"
TAG="${2:?usage: fetch-subproject-tag.sh <project> <tag>}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SM="$ROOT/subprojects/$PROJ"

git -C "$SM" rev-parse --git-dir >/dev/null 2>&1 || {
    echo "submodule $PROJ not initialised; run 'just init'" >&2
    exit 1
}

if git -C "$SM" rev-parse --verify --quiet "refs/tags/$TAG^{commit}" >/dev/null; then
    exit 0
fi

echo ">> fetching pinned tag $TAG for $PROJ"
git -C "$SM" fetch --quiet --no-tags --depth=1 origin "refs/tags/$TAG:refs/tags/$TAG" || {
    echo "failed to fetch tag $TAG for subproject $PROJ" >&2
    exit 1
}

pinned="$(git -C "$ROOT" rev-parse "HEAD:subprojects/$PROJ" 2>/dev/null || true)"
tagged="$(git -C "$SM" rev-parse "refs/tags/$TAG^{commit}")"
if [ -n "$pinned" ] && [ "$pinned" != "$tagged" ]; then
    echo "warning: $PROJ tag $TAG ($tagged) differs from the submodule pin ($pinned)" >&2
fi
