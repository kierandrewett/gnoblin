#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

if [ ! -x "$PREFIX/bin/gnoblin-shell" ]; then
  echo "SKIP: gnoblin-shell not installed in $PREFIX (run just dev)" >&2
  exit 0
fi

echo "== failed autostart entries must retry after config reload =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/autostart-retry-test.py" 2>&1 \
  | tee /tmp/gnoblin-autostart-retry-test.log
