#!/usr/bin/env bash
# End to end: real X11 clients under the real session, with a shell holding the window tree.
#
# What runs is the `gnoblin` binary as a session (guardian, headless backend, software renderer, Xwayland) with a shell that
# holds the window tree: GNOBLIN_E2E_SHELL names its library (the std prelude example), and none runs the default shell.
# The clients are real X11 programs: `xterm`, which maps at a placeholder 1x1 and then asks for its real size and
# waits to be told it got it (an Xt client), `xeyes`, `xclock`, and `xmessage`, a dialog whose size hints name its size.
# Each one must reach the size it asked for, which the script reads back from the X server (`xwininfo`) and checks against
# the `WM_NORMAL_HINTS` the client set (`xprop`). The picture is taken through the compositor's screencopy path with grim.
# Nothing is drawn by this script.
#
# Usage: tests/x11-clients-e2e.sh OUT_DIR
# GNOBLIN_BIN_DIR names where `gnoblin` and `gnoblinctl` are (default: target/release, as scripts/js-shell-screenshots.sh).
# Needs: what scripts/js-shell-screenshots.sh needs, and xterm, xeyes, xclock, xmessage, xwininfo and xprop.
#
# OUT_DIR gets `result.txt` (one line per client), `<client>.xwininfo`, `<client>.xprop`, `<client>.png`, and the
# session's log. The exit status is 0 only when every client reached its size.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
out_dir="${1:?usage: tests/x11-clients-e2e.sh OUT_DIR}"
mkdir -p "$out_dir"
out_dir="$(cd "$out_dir" && pwd)"

for tool in xterm xeyes xclock xmessage xwininfo xprop; do
    command -v "$tool" >/dev/null || {
        echo "x11-clients-e2e: $tool is not installed" >&2
        exit 1
    }
done

# The helpers of the screenshot script: the session, `ctl`, `shoot`, `stop_session`.
# shellcheck source=../scripts/js-shell-screenshots.sh
source "$here/../scripts/js-shell-screenshots.sh" "$out_dir"

display=""
result="$out_dir/result.txt"
: >"$result"
failures=0

# boot_guardian: the screenshot script's, except that GNOBLIN_E2E_SHELL, when set, names the shell library the session
# runs (the std prelude example, `libstd_prelude.so`, is the one this test is meant for), and the session is ready when
# the guardian says so: a native shell commits no configuration document to wait for.
# shellcheck disable=SC2154,SC2034 # session_home, run_dir and bin_dir are set, and session_pid is read, by the sourced script
boot_guardian() {
    : >"$session_home/session.log"
    setsid env -u WAYLAND_DISPLAY -u DISPLAY -u GNOBLIN_JS_STOCK_DIR -u GNOBLIN_SHELL \
        XDG_RUNTIME_DIR="$run_dir" \
        XDG_CONFIG_HOME="$session_home/config" \
        XDG_STATE_HOME="$session_home/state" \
        XDG_DATA_HOME="$session_home/data" \
        XDG_CACHE_HOME="$session_home/cache" \
        DBUS_SESSION_BUS_ADDRESS="$(cat "$session_home/bus-address")" \
        GNOBLIN_IPC_SOCKET="$run_dir/ipc.sock" \
        "$bin_dir/gnoblin" ${GNOBLIN_E2E_SHELL:+"$GNOBLIN_E2E_SHELL"} >>"$session_home/session.log" 2>&1 &
    session_pid=$!
    local waited=0
    until grep -q 'session ready' "$session_home/session.log" 2>/dev/null; do
        sleep 0.2
        waited=$((waited + 1))
        if [ "$waited" -gt 600 ]; then
            echo "x11-clients-e2e: the session never became ready; its log:" >&2
            cat "$session_home/session.log" >&2
            exit 1
        fi
    done
}

# find_display: waits for the session's X server, which is the one whose root window carries the window manager's
# `_NET_SUPPORTED`, and sets `display`.
find_display() {
    local tries=0 socket number
    while [ "$tries" -lt 300 ]; do
        for socket in /tmp/.X11-unix/X*; do
            [ -S "$socket" ] || continue
            number="${socket##*/X}"
            if xprop -display ":$number" -root _NET_SUPPORTED 2>/dev/null | grep -q '_NET_WM_STATE'; then
                display=":$number"
                return 0
            fi
        done
        sleep 0.2
        tries=$((tries + 1))
    done
    echo "x11-clients-e2e: the session never started an X server; its log:" >&2
    cat "$session_home/session.log" >&2
    exit 1
}

# client_line CLASS: the `xwininfo -root -tree` line of the client window whose class is CLASS, or nothing.
client_line() {
    xwininfo -display "$display" -root -tree 2>/dev/null | grep -F "\"$1\")" | head -n 1 || true
}

# hint NAME FILE: the pair `W H` of a `WM_NORMAL_HINTS` entry ("minimum size", "base size", "resize increment"), or nothing.
hint() {
    sed -n "s/^.*$1: \([0-9]*\) by \([0-9]*\).*$/\1 \2/p" "$2" | head -n 1
}

# check NAME CLASS EXPECTED COMMAND...: starts COMMAND, waits for its window to settle, and requires its size to be EXPECTED:
# `WxH` exactly, `cells:COLSxROWS` as the hints' base size plus the resize increment per cell (what a terminal's
# `-geometry` means), or `min` for the hints' minimum size, or the size the client names there when it names no minimum. A size of 1x1 is never accepted.
check() {
    local name="$1" class="$2" expected="$3"
    shift 3
    DISPLAY="$display" "$@" >"$out_dir/$name.stdout" 2>&1 &
    client_pids+=("$!")
    local tries=0 line="" width=0 height=0 id="" want="" min base inc cols rows
    while [ "$tries" -lt 150 ]; do
        line="$(client_line "$class")"
        if [ -n "$line" ]; then
            id="$(printf '%s' "$line" | awk '{print $1}')"
            size="$(printf '%s' "$line" | grep -Eo '[0-9]+x[0-9]+\+' | head -n 1)"
            width="${size%%x*}"
            height="${size#*x}"
            height="${height%+}"
            DISPLAY="$display" xprop -id "$id" WM_NORMAL_HINTS >"$out_dir/$name.xprop" 2>&1 || true
            min="$(hint 'minimum size' "$out_dir/$name.xprop")"
            [ -n "$min" ] || min="$(hint 'program specified size' "$out_dir/$name.xprop")"
            case "$expected" in
                cells:*)
                    base="$(hint 'base size' "$out_dir/$name.xprop")"
                    inc="$(hint 'resize increment' "$out_dir/$name.xprop")"
                    cols="${expected#cells:}"
                    rows="${cols#*x}"
                    cols="${cols%x*}"
                    if [ -n "$base" ] && [ -n "$inc" ]; then
                        want="$((${base% *} + cols * ${inc% *}))x$((${base#* } + rows * ${inc#* }))"
                    fi
                    ;;
                min) want="${min// /x}" ;;
                *) want="$expected" ;;
            esac
            if [ -n "$want" ] && [ "${width}x${height}" = "$want" ] && [ "${width}x${height}" != "1x1" ]; then
                break
            fi
        fi
        sleep 0.2
        tries=$((tries + 1))
    done
    xwininfo -display "$display" -root -tree >"$out_dir/$name.xwininfo" 2>&1 || true
    local verdict=ok
    if [ "${width}x${height}" != "$want" ] || [ -z "$want" ] || [ "${width}x${height}" = "1x1" ]; then
        verdict=FAIL
        failures=$((failures + 1))
    fi
    printf '%s %s: is %sx%s, wants %s\n' "$verdict" "$name" "$width" "$height" "${want:-<no size hints read>}" | tee -a "$result"
}

main() {
    start_session
    find_display
    # xterm: the Xt client. `-geometry 90x24` is a size in cells; the placeholder it maps at is 1x1.
    check xterm XTerm "cells:90x24" xterm -geometry 90x24+0+0 -e sleep 3600
    check xeyes XEyes "300x200" xeyes -geometry 300x200+50+50
    check xclock XClock "200x200" xclock -geometry 200x200+100+100
    # A dialog: its hints name the size it needs, and it must not stay smaller than that.
    check xmessage Xmessage min xmessage -geometry +150+150 "a dialog with size hints"
    shoot x11-clients
    stop_session
    printf 'failures: %s\n' "$failures" | tee -a "$result"
    [ "$failures" -eq 0 ]
}

main
