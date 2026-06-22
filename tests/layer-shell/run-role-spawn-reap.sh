#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

if [ ! -x "$PREFIX/bin/gnoblin-shell" ]; then
  echo "SKIP: gnoblin-shell not installed in $PREFIX (run just dev)" >&2
  exit 0
fi
if ! command -v foot >/dev/null 2>&1; then
  echo "SKIP: foot not installed" >&2
  exit 0
fi

echo "== short-lived role clients must be reaped =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/role-spawn-reap-test.py" 2>&1 \
  | tee /tmp/gnoblin-role-spawn-reap-test.log
