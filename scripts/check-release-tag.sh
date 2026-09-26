#!/usr/bin/env bash
# Print the package revision for the current Gnoblin SemVer release tag.
set -euo pipefail
export PATH="$PATH"
root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
version="$("$root/scripts/gnoblin-version.py" get version)"
tag="${1:?release tag required}"
if [ "$tag" = "gnoblin-v$version" ]; then
    # Debian revisions 1 through 4 are already published and immutable.
    # Keep the SemVer tag while publishing this corrected build as revision 5.
    if [ "$version" = "0.1.7" ]; then
        echo 5
    else
        echo 1
    fi
else
    echo "Expected gnoblin-v$version, got $tag" >&2
    exit 2
fi
