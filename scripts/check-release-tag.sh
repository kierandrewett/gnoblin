#!/usr/bin/env bash
# Print the package revision for a tag matching the pinned GNOME release.
set -euo pipefail
export PATH="$PATH"
root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
version="$("$root/scripts/gnome-versions.py" get mutter version)"
tag="${1:?release tag required}"
if [ "$tag" = "v$version" ]; then
    echo 1
elif [[ "$tag" == "v$version-"* ]] && [[ "${tag#"v$version-"}" =~ ^[1-9][0-9]*$ ]]; then
    printf '%s\n' "${tag#"v$version-"}"
else
    echo "Expected v$version or v$version-REVISION, got $tag" >&2
    exit 2
fi
