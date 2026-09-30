#!/usr/bin/env bash
# The standalone devkit already gives every run disposable XDG directories.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec "$root/scripts/run-gnoblin-devkit.sh" "$@"
