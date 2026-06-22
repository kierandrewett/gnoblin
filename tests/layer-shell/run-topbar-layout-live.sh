#!/usr/bin/env bash
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

[ -x "$PREFIX/bin/gnoblin-shell" ] || { echo "SKIP: no dev build at $PREFIX (run 'just dev')"; exit 0; }
[ -x "$PREFIX/bin/gnoblin-topbar" ] || { echo "SKIP: gnoblin-topbar not built"; exit 0; }
python3 -c "import gi; gi.require_version('Gio','2.0')" 2>/dev/null || { echo "SKIP: no python gi"; exit 0; }

echo "== topbar must live-reload widget layout and input geometry =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/topbar-layout-live-test.py" 2>&1 \
  | tee /tmp/gnoblin-topbar-layout-live-test.log
exit "${PIPESTATUS[0]}"
