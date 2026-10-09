#!/usr/bin/env bash
# Guest: helper for run-compositor-crash.sh. Usage: guest-compositor-crash.sh crash | read
#
# crash sends SIGSEGV to the compositor and reports how many Gnoblin processes are left. A compositor that stops on its own
# takes every Wayland app with it, and the guardian ends the session. read reports what the next login shows: the compositor
# state, the first line of the recovery marker, its message, and whether the previous compositor log is kept.
set -u
XDG_RUNTIME_DIR="/run/user/$(id -u)"
export XDG_RUNTIME_DIR
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
MARKER="$XDG_RUNTIME_DIR/gnoblin/config-fallback"

case "${1:-}" in
    crash)
        pid=""
        for candidate in $(pgrep -x gnoblin); do
            if tr '\0' ' ' <"/proc/$candidate/cmdline" | grep -q -e '--wayland'; then pid="$candidate"; fi
        done
        echo "compositor_found=$([ -n "$pid" ] && echo yes || echo no)"
        kill -SEGV "$pid"
        for _ in $(seq 1 20); do
            if [ "$(pgrep -x -c gnoblin)" = 0 ]; then break; fi
            sleep 0.5
        done
        echo "processes_after_crash=$(pgrep -x -c gnoblin)"
        ;;
    read)
        echo "compositors=$(pgrep -x -c gnoblin)"
        echo "status=$("$G" status 2>&1 | head -1 | grep -o '"state":"[a-z]*"')"
        echo "marker=$(head -1 "$MARKER" 2>/dev/null)"
        echo "marker_message=$(sed -n 2p "$MARKER" 2>/dev/null)"
        echo "previous_log_bytes=$(stat -c %s "${XDG_STATE_HOME:-$HOME/.local/state}/gnoblin/compositor-previous.log" 2>/dev/null || echo missing)"
        ;;
    *)
        echo "usage: $0 crash | read" >&2
        exit 2
        ;;
esac
