#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/retry-command.sh"

# Optional arguments are the submodule paths to fetch. With none, every submodule is fetched.
gnoblin_retry_command git submodule update --init --recursive "$@"
