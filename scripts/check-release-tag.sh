#!/usr/bin/env bash
# Print the package revision for the current Gnoblin SemVer release tag.
set -euo pipefail
export PATH="$PATH"
root="$(cd -- "$(dirname -- "$0")/.." && pwd)"
version="$("$root/scripts/gnoblin-version.py" get version)"
tag="${1:?release tag required}"
if [ "$tag" = "gnoblin-v$version" ]; then
    # Debian revisions 1, 2, and 3 are already published and immutable.
    # Keep the SemVer tag while publishing this corrected build as revision 4.
    if [ "$version" = "0.1.7" ]; then
        echo 4
    else
        echo 1
    fi
else
    echo "Expected gnoblin-v$version, got $tag" >&2
    exit 2
fi
