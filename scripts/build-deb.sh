#!/usr/bin/env bash
# Run inside a prepared Debian/Ubuntu build container, as the build user.
set -euo pipefail
export PATH="$PATH"
cd -- "$(dirname -- "$(realpath -- "$0")")/.."

prefix=/usr/lib/gnoblin
if [ "$(id -u)" -eq 0 ] || [ ! -w "$prefix" ]; then
    echo 'Run as the build user in a container with /usr/lib/gnoblin owned by that user.' >&2
    echo 'See packaging/deb/README.md for the container build.' >&2
    exit 2
fi
export GIT_AUTHOR_NAME="${GIT_AUTHOR_NAME:-Gnoblin build}"
export GIT_AUTHOR_EMAIL="${GIT_AUTHOR_EMAIL:-build@gnoblin.local}"
export GIT_COMMITTER_NAME="${GIT_COMMITTER_NAME:-$GIT_AUTHOR_NAME}"
export GIT_COMMITTER_EMAIL="${GIT_COMMITTER_EMAIL:-$GIT_AUTHOR_EMAIL}"

mkdir -p build/deb-tools
if [[ ! "${GNOBLIN_BUILD_JOBS:-4}" =~ ^[1-9][0-9]*$ ]]; then
    echo 'GNOBLIN_BUILD_JOBS must be a positive integer.' >&2
    exit 2
fi
ninja_bin=$(command -v ninja)
printf '#!/bin/sh\nexec "%s" -j%s "$@"\n' "$ninja_bin" "${GNOBLIN_BUILD_JOBS:-4}" >build/deb-tools/ninja
chmod +x build/deb-tools/ninja
export PATH="$PWD/build/deb-tools:$PATH"
export CARGO_BUILD_JOBS="${GNOBLIN_BUILD_JOBS:-4}"
python3 - <<'MANIFEST'
import json
import platform
from pathlib import Path
base = json.loads(Path("build-dependencies.json").read_text())
distro = platform.freedesktop_os_release()
if (distro["ID"], distro["VERSION_ID"]) != ("ubuntu", "26.04"):
    position = next(index for index, recipe in enumerate(base) if recipe["name"] == "gjs")
    base[position:position] = json.loads(Path("packaging/deb/build-dependencies.json").read_text())
Path("build/deb-dependencies.json").write_text(json.dumps(base))
MANIFEST
just init
python3 scripts/build-private-deps.py --prefix "$prefix/deps" --manifest build/deb-dependencies.json --jobs "${GNOBLIN_BUILD_JOBS:-4}"
python3 scripts/build-private-deps.py --prefix "$prefix/deps" --run \
    env GNOBLIN_PREFIX="$prefix" GNOBLIN_LIBDIR=lib64 GNOBLIN_DEVKIT=disabled just build-local
python3 scripts/build-private-deps.py --prefix "$prefix/deps" \
    --fix-runtime --runtime-prefix "$prefix"
python3 scripts/package-deb.py "$@"
