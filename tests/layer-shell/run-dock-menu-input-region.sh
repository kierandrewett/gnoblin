#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

if [ ! -x "$PREFIX/bin/gnoblin-shell" ]; then
  echo "SKIP: gnoblin-shell not installed in $PREFIX (run just dev)" >&2
  exit 0
fi
if ! command -v gst-launch-1.0 >/dev/null 2>&1; then
  echo "SKIP: gst-launch-1.0 not installed (needed for RemoteDesktop pointer injection)" >&2
  exit 0
fi

echo "== dock context menu daemon must map via D-Bus and dismiss via scrim =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/dock-menu-input-region-test.py" 2>&1 \
  | tee /tmp/gnoblin-dock-menu-input-region-test.log
