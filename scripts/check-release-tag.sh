#!/usr/bin/env bash
# Print the package revision for the current Gnoblin SemVer release tag.
set -euo pipefail
export PATH="$PATH"
root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
version="$("$root/scripts/gnoblin-version.py" get version)"
tag="${1:?release tag required}"
if [ "$tag" = "gnoblin-v$version" ]; then
    # 0.1.7-1 is already immutable in the APT repositories. Keep the SemVer
    # tag while publishing the corrected source build as Debian revision 2.
    if [ "$version" = "0.1.7" ]; then
        echo 2
    else
        echo 1
    fi
else
    echo "Expected gnoblin-v$version, got $tag" >&2
    exit 2
fi
