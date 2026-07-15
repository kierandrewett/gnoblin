#!/usr/bin/env bash
# Prove Gnoblin-only protocols, control APIs, and Shell policy stay out of stock mode.
set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

GNOBLIN_EXPECT_EXTENSION_SCOPE=1 \
GNOBLIN_EXPECT_NOTIFICATION_OWNER=1 \
GNOBLIN_TEST_EXTENSION_ROOT="$ROOT/tests/shell-extensions" \
GNOBLIN_TEST_DBUS_CLIENT="$ROOT/scripts/test-shell-security-policy.py" \
GNOBLIN_TEST_GSETTINGS_BACKEND=keyfile \
GNOBLIN_TEST_DISABLE_NOTIFICATIONS=1 \
GNOBLIN_TEST_ENV_MODE=gnoblin \
GNOBLIN_TEST_MODE=user \
GNOBLIN_EXPECT_PRIVILEGED_PROTOCOLS=0 \
    "$ROOT/scripts/run-gnome-shell.sh"
