#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PREFIX="${GNOBLIN_PREFIX:-$ROOT/install}"

if [ ! -x "$PREFIX/bin/gnoblin-shell" ]; then
  echo "SKIP: gnoblin-shell not installed in $PREFIX (run just dev)" >&2
  exit 0
fi

echo "== explicit -- COMMAND children must be reaped =="
GNOBLIN_PREFIX="$PREFIX" python3 "$ROOT/tests/layer-shell/explicit-command-reap-test.py" 2>&1 \
  | tee /tmp/gnoblin-explicit-command-reap-test.log
