#!/usr/bin/env bash
# Build the pinned private GNOME closure that legacy RPM targets cannot supply.
set -euo pipefail

if [[ ${EUID} -eq 0 ]]; then
    echo "Run the compatibility runtime build as the gnoblin-build user." >&2
    exit 1
fi

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
PREFIX=/usr/lib/gnoblin/deps
MANIFEST="$ROOT/build/rpm-compat-runtime.json"

if [[ ! -w /usr/lib/gnoblin ]]; then
    echo "The private Gnoblin prefix must be owned by the build user." >&2
    exit 2
fi

python3 "$ROOT/packaging/rpm/compose-compat-runtime-manifest.py" --output "$MANIFEST"
python3 "$ROOT/scripts/build-private-deps.py" \
    --prefix "$PREFIX" \
    --cache "$ROOT/build/rpm-compat-cache" \
    --manifest "$MANIFEST" \
    --jobs "${GNOBLIN_BUILD_JOBS:-2}"

test -f "$PREFIX/lib64/pkgconfig/girepository-2.0.pc"
test -f "$PREFIX/lib64/pkgconfig/gjs-1.0.pc"
test -f "$PREFIX/lib64/pkgconfig/glycin-2.pc"
test -f "$PREFIX/lib64/pkgconfig/hyprcursor.pc"
