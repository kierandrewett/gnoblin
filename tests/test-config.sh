#!/usr/bin/env bash
# Native config tests need no display or full Mutter build.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CFLAGS="$(pkg-config --cflags --libs glib-2.0 lua)"
BIN="$(mktemp -d /tmp/gnoblin-cfg.XXXXXX)"
trap 'rm -rf "$BIN"' EXIT

sources=(
    "$ROOT/src/config/gnoblin-config.c"
    "$ROOT/src/config/gnoblin-lua.c"
    "$ROOT/src/config/gnoblin-glob.c"
)
for test in lua-config glob-config lua-console; do
    cc "$ROOT/tests/$test-test.c" "${sources[@]}" \
       -I "$ROOT/src/config" $CFLAGS -o "$BIN/$test-test"
    timeout 20 "$BIN/$test-test"
done
