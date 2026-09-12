#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
testdir="$(mktemp -d /tmp/gnoblin-blur-protocol.XXXXXX)"
trap 'rm -rf -- "$testdir"' EXIT
wayland-scanner client-header "$root/src/protocols/blur-fade/gnoblin-blur-fade-v1.xml" "$testdir/blur-fade-client.h"
wayland-scanner private-code "$root/src/protocols/blur-fade/gnoblin-blur-fade-v1.xml" "$testdir/protocol.c"
compiler_flags="$(pkg-config --cflags --libs wayland-client)" || exit 1
read -r -a compiler_args <<<"$compiler_flags"
cc "$root/tests/blur-fade-protocol.c" "$testdir/protocol.c" -I"$testdir" "${compiler_args[@]}" -o "$testdir/client"
if [[ "${GNOBLIN_ACTIVE_MODE:-gnoblin}" != gnoblin ]]; then
    "$testdir/client" absent
else
    for check in valid opacity size coordinate length count; do "$testdir/client" "$check"; done
fi
