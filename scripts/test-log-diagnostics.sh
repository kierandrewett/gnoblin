#!/usr/bin/env bash
# Verify that shell smoke tests distinguish fatal diagnostics from expected noise.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP="$(mktemp -d /tmp/gnoblin-log-test.XXXXXX)"
trap 'rm -rf "$TMP"' EXIT

source "$ROOT/scripts/gnoblin-test-lib.sh"

cat > "$TMP/benign.log" <<'EOF'
portal is not running: GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown
DeprecationWarning: Gio.DBusConnection.register_object is deprecated
GNOME Shell started
EOF

cat > "$TMP/fatal.log" <<'EOF'
GNOME Shell-CRITICAL **: TypeError: can't access property "join"
EOF

if gnoblin_log_has_fatal "$TMP/benign.log"; then
    echo "FAIL: expected environment diagnostics were treated as fatal" >&2
    exit 1
fi

if ! gnoblin_log_has_fatal "$TMP/fatal.log"; then
    echo "FAIL: GNOME Shell CRITICAL was not treated as fatal" >&2
    exit 1
fi

wait_log="$TMP/wait.log"
: > "$wait_log"
(sleep 0.1; echo "state: ready" >> "$wait_log") &
if ! gnoblin_wait_for_log "$wait_log" '^state: ready$' 2; then
    echo "FAIL: state-based log wait missed the transition" >&2
    exit 1
fi
if gnoblin_wait_for_log "$wait_log" '^state: never$' 1; then
    echo "FAIL: state-based log wait ignored its timeout" >&2
    exit 1
fi

echo "PASS: fatal shell diagnostics detected"
