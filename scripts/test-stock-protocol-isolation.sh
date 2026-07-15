#!/usr/bin/env bash
# Prove Gnoblin-only Wayland globals and control API stay out of stock mode.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

GNOBLIN_TEST_MODE=user \
GNOBLIN_EXPECT_PRIVILEGED_PROTOCOLS=0 \
    "$ROOT/scripts/run-gnome-shell.sh"
