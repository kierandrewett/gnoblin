#!/usr/bin/env bash
# Build and check the smallest private runtime closure needed before RPM packaging.
set -euo pipefail

if [[ ${EUID} -eq 0 ]]; then
    echo "Run this bootstrap build as the gnoblin-build user." >&2
    exit 1
fi

source /etc/os-release
case "${ID}:${VERSION_ID}" in
    rocky:9 | rocky:9.* | rhel:9 | rhel:9.* | almalinux:9 | almalinux:9.* | \
        rocky:10 | rocky:10.* | rhel:10 | rhel:10.* | almalinux:10 | almalinux:10.*)
        export PATH="/opt/gnoblin-rpm-compat-tools/bin:${PATH}"
        ;;
    opensuse-leap:16.0)
        ;;
    *)
        echo "The private GLib/GI bootstrap gate currently supports EL 9/10 and openSUSE Leap 16.0 only; got ${ID}:${VERSION_ID}." >&2
        exit 2
        ;;
esac

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
cd "$repo_root"
prefix="$repo_root/build/rpm-compat/deps"
python3 scripts/build-private-deps.py \
    --prefix "$prefix" \
    --cache "$repo_root/build/rpm-compat/cache" \
    --manifest packaging/rpm/compat-bootstrap.json \
    --only glib-final \
    --jobs 2

test -f "$prefix/lib64/pkgconfig/girepository-2.0.pc"
test -e "$prefix/lib64/libgirepository-2.0.so"
test -x "$prefix.build-tools/bin/g-ir-scanner"
test ! -e "$prefix/bin/g-ir-scanner"
"$prefix/bin/patchelf" --print-rpath "$prefix/lib64/libgirepository-2.0.so" | grep -Fx "$prefix/lib64"
