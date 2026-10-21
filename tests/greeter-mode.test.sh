#!/usr/bin/env bash
# Greeter mode: the command line and the fresh-session guard work outside a login session.
#
# usage: tests/greeter-mode.test.sh GNOBLIN_BINARY GREETER_LUA
#
# GNOBLIN_BINARY is the built gnoblin guardian (for example build/stage/usr/bin/gnoblin).
# GREETER_LUA is the shipped src/data/session/greeter.lua. The test runs outside a login
# session, so greeter mode must refuse to start and say why, without crashing.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
gnoblin="${1:?usage: greeter-mode.test.sh GNOBLIN_BINARY GREETER_LUA}"
greeter_lua="${2:?usage: greeter-mode.test.sh GNOBLIN_BINARY GREETER_LUA}"
tmp="$(mktemp -d)"
trap 'rm -rf -- "$tmp"' EXIT

fail() {
    printf 'greeter-mode: %s\n' "$*" >&2
    exit 1
}

# The usage text lists the greeter flag.
usage_text="$("$gnoblin" --bogus-flag 2>&1 || true)"
grep -q -- '\[--greeter\]' <<<"$usage_text" || fail "usage does not list --greeter"

# Outside a fresh Wayland login the guardian refuses, exits non-zero, and does not crash.
# Clear display variables so the guard, not the host session, decides the outcome.
set +e
env -u WAYLAND_DISPLAY -u DISPLAY -u XDG_SESSION_ID -u XDG_SESSION_TYPE \
    XDG_RUNTIME_DIR="$tmp/runtime" HOME="$tmp/home" XDG_STATE_HOME="$tmp/state" \
    timeout 20 "$gnoblin" --greeter --config "$tmp/does-not-exist.lua" \
    >"$tmp/out" 2>"$tmp/err"
status=$?
set -e
[[ "$status" -ne 0 ]] || fail "--greeter outside a login session must exit non-zero"
[[ "$status" -lt 124 ]] || fail "--greeter hung or timed out (status $status)"
grep -q 'start it from a fresh Wayland login session' "$tmp/err" ||
    fail "missing fresh-session error; stderr was: $(cat "$tmp/err")"

# The guard runs before any user configuration is read or created.
[[ ! -e "$tmp/home/.config/gnoblin" ]] || fail "greeter mode must not create user config"

# The shipped default is only a fallback, so it must carry no autostart entries.
grep -Eq '^return[[:space:]]*\{\}[[:space:]]*$' "$greeter_lua" ||
    fail "$greeter_lua must return an empty configuration table"

printf 'greeter-mode: ok\n'
