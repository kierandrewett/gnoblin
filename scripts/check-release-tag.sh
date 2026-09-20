#!/usr/bin/env bash
# Print the package revision for the current Gnoblin SemVer release tag.
set -euo pipefail
export PATH="$PATH"
root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
version="$("$root/scripts/gnoblin-version.py" get version)"
tag="${1:?release tag required}"
if [ "$tag" = "gnoblin-v$version" ]; then
    echo 1
else
    echo "Expected gnoblin-v$version, got $tag" >&2
    exit 2
fi
