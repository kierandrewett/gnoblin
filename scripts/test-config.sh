#!/usr/bin/env bash
# Tier-1 logic test: the shared C config parser (src/config/gnoblin-config.c) that
# the mutter protocol overlays compile in. No display, no full build — just cc + run.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CFLAGS="$(pkg-config --cflags --libs glib-2.0)"
BIN="$(mktemp -d /tmp/gnoblin-cfg.XXXXXX)"
trap 'rm -rf "$BIN"' EXIT

cc "$ROOT/tests/config-test.c" "$ROOT/src/config/gnoblin-config.c" \
   "$ROOT/src/config/gnoblin-toml.c" "$ROOT/src/config/tomlc99/toml.c" \
   -I "$ROOT/src/config" $CFLAGS -o "$BIN/config-test"

echo "== gnoblin.conf parser =="
"$BIN/config-test"
echo ">> config parser test PASS"

cc "$ROOT/tests/toml-config-test.c" "$ROOT/src/config/gnoblin-config.c" \
   "$ROOT/src/config/gnoblin-toml.c" "$ROOT/src/config/tomlc99/toml.c" \
   -I "$ROOT/src/config" $CFLAGS -o "$BIN/toml-config-test"
"$BIN/toml-config-test"
