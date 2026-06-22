#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
command -v gst-launch-1.0 >/dev/null || { echo "SKIP: no gst-launch-1.0"; exit 0; }

echo "== per-output autostart must not respawn after output removal =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/autostart-output-removal-test.py" 2>&1 \
  | tee /tmp/gnoblin-autostart-output-removal-test.log
