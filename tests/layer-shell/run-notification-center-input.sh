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
if ! command -v notify-send >/dev/null 2>&1; then
  echo "SKIP: notify-send not installed" >&2
  exit 0
fi

echo "== notification history must not block topbar/popout input =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/notification-center-input-test.py" 2>&1 \
  | tee /tmp/gnoblin-notification-center-input-test.log
