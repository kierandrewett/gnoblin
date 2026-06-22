#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-dock" ] || { echo "SKIP: gnoblin-dock not built"; exit 0; }
command -v foot >/dev/null || { echo "SKIP: no foot"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

echo "== dock launch must resolve desktop ids and real icon clicks (headless) =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/dock-launch-test.py" 2>&1 \
  | tee /tmp/gnoblin-dock-launch-test.log
