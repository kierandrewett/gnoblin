#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-dock" ] || { echo "SKIP: gnoblin-dock not built"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

echo "== dock must live-reload [dock] favorites config =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/dock-live-favorites-test.py" 2>&1 \
  | tee /tmp/gnoblin-dock-live-favorites-test.log
exit "${PIPESTATUS[0]}"
