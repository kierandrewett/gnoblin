#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-dock" ] || { echo "SKIP: gnoblin stack not installed; run just dev"; exit 0; }
command -v firefox >/dev/null || { echo "SKIP: no firefox"; exit 0; }
command -v gtk-launch >/dev/null || { echo "SKIP: no gtk-launch"; exit 0; }

echo "== Firefox desktop launch through dock must not crash compositor =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/firefox-launch-test.py" 2>&1 \
  | tee /tmp/gnoblin-firefox-launch-test.log
