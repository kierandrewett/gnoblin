#!/usr/bin/env bash
# End-to-end: X11 clients and Wayland clients share the clipboard and the primary selection through Xwayland.
#
#   GNOBLIN_BIN=/path/to/gnoblin bash tests/xwayland-clipboard-e2e.sh [OUTPUT_DIR]
#
# Needs: wl-copy and wl-paste (wl-clipboard), xclip, dbus-daemon, and a gnoblin binary. The compositor runs without a
# display of its own. Xwayland refuses to start on a binary without a -byteswappedclients switch unless the config says
# `allow-byte-swapped-clients = true`, so the script writes that before the first boot.
#
# The first boot runs on an empty last-good store, so the config is committed only after Xwayland has refused to start
# (design/e2e-clipboard-xwayland.md, F2b). The script boots once to commit the config, stops, then boots again for the
# checks.
#
# X11 owners (xclip -i) are started in the background: an owner stays alive to answer selection requests, so running
# it in the foreground would block the script. Every step first kills the owners of the previous step, so each step
# starts from an empty selection.
#
# Each check writes one line to OUTPUT_DIR/results.txt: NAME PASS|FAIL src=<sha256> got=<sha256> bytes=<n>.
# The exit status is the number of failures. The compositor logs are kept in OUTPUT_DIR.
set -u

BIN="${GNOBLIN_BIN:-$HOME/target/debug/gnoblin}"
OUT="${1:-$(mktemp -d /tmp/xwayland-clip.XXXXXX)}"
mkdir -p "$OUT"
R="$OUT/run"
mkdir -p "$R/config/gnoblin" "$R/state"
chmod 700 "$R"
export XDG_RUNTIME_DIR="$R" XDG_CONFIG_HOME="$R/config" XDG_STATE_HOME="$R/state"
export WAYLAND_DISPLAY=gnoblin-wayland.sock
RESULTS="$OUT/results.txt"
: >"$RESULTS"
fail=0
GNOBLIN_PID=""
DBUS_PID=""

result() {
    # result NAME PASS|FAIL DETAIL
    echo "$1 $2 $3" | tee -a "$RESULTS"
    [ "$2" = PASS ] || fail=$((fail + 1))
}

sha() {
    if [ -f "$1" ]; then sha256sum "$1" | cut -c1-16; else echo none; fi
}

compare() {
    # compare NAME SOURCE_FILE RECEIVED_FILE: the received bytes must equal the source bytes exactly.
    local src got
    src="$(sha "$2")"
    got="$(sha "$3")"
    if cmp -s "$2" "$3" 2>/dev/null; then
        result "$1" PASS "src=$src got=$got bytes=$(stat -c%s "$2")"
    else
        result "$1" FAIL "src=$src got=$got bytes_src=$(stat -c%s "$2") bytes_got=$(stat -c%s "$3" 2>/dev/null || echo missing)"
    fi
}

reset_owners() {
    # Stop every X11 and Wayland selection owner from the previous step, so a read cannot see stale data.
    pkill -x xclip 2>/dev/null
    pkill -x wl-copy 2>/dev/null
    sleep 1
}

stop_gnoblin() {
    if [ -n "$GNOBLIN_PID" ]; then
        kill "$GNOBLIN_PID" 2>/dev/null
        for _ in 1 2 3 4 5 6 7 8 9 10; do
            kill -0 "$GNOBLIN_PID" 2>/dev/null || break
            sleep 0.5
        done
        kill -9 "$GNOBLIN_PID" 2>/dev/null
        wait "$GNOBLIN_PID" 2>/dev/null
        GNOBLIN_PID=""
    fi
}

cleanup() {
    stop_gnoblin
    pkill -x xclip 2>/dev/null
    pkill -x wl-copy 2>/dev/null
    [ -n "$DBUS_PID" ] && kill "$DBUS_PID" 2>/dev/null
}
trap cleanup EXIT

cat >"$R/config/gnoblin/init.lua" <<'LUA'
gnoblin.configure {
    xwayland = {
        ["allow-byte-swapped-clients"] = true,
    },
}
LUA

# A private session bus. dbus-daemon prints the address, then the pid.
{
    read -r DBUS_ADDR
    read -r DBUS_PID
} < <(dbus-daemon --session --fork --print-address=1 --print-pid=1 2>"$OUT/dbus.err")
export DBUS_SESSION_BUS_ADDRESS="$DBUS_ADDR"

# boot LOG: starts gnoblin, waits for the Wayland socket and the Xwayland ready line; sets DISPLAY from that line.
# Returns 1 if it does not come up.
boot() {
    "$BIN" >"$1" 2>&1 &
    GNOBLIN_PID=$!
    display=""
    for _ in $(seq 1 120); do
        if [ -S "$R/$WAYLAND_DISPLAY" ]; then
            display="$(sed -n 's/.*area=xwayland display_number=\([0-9]*\).*ready.*/\1/p;s/.*ready.*display_number=\([0-9]*\).*/\1/p' "$1" | head -1)"
            [ -n "$display" ] && break
        fi
        kill -0 "$GNOBLIN_PID" 2>/dev/null || return 1
        sleep 0.5
    done
    [ -n "$display" ] || return 1
    export DISPLAY=":$display"
    return 0
}

# Boot 1: commit the config. Xwayland refuses on this boot (F2b); that is expected.
boot "$OUT/session-1.log"
for _ in $(seq 1 60); do
    grep -q "configuration committed" "$OUT/session-1.log" 2>/dev/null && break
    sleep 0.5
done
stop_gnoblin

# Boot 2: the checks.
if ! boot "$OUT/session.log"; then
    echo "FAIL start (no Wayland socket or Xwayland ready line; see $OUT/session.log)" | tee -a "$RESULTS"
    exit 1
fi
echo "started: WAYLAND_DISPLAY=$WAYLAND_DISPLAY DISPLAY=$DISPLAY" | tee -a "$RESULTS"
sleep 2

printf 'x11-to-wayland-text' >"$OUT/t1-src"
head -c 512 /dev/urandom >"$OUT/bin-src"
printf 'x11-primary' >"$OUT/p1-src"
printf 'wayland-to-x11-text' >"$OUT/t2-src"
printf 'wayland-primary' >"$OUT/p3-src"
printf 'x11-to-clear' >"$OUT/c-src"
: >"$OUT/empty"

# E1: an X11 owner sets the clipboard text, a Wayland paste reads it.
reset_owners
xclip -quiet -selection clipboard -i <"$OUT/t1-src" >/dev/null 2>&1 &
sleep 2
timeout 10 wl-paste -n >"$OUT/E1-got" 2>/dev/null
compare "E1 X11 clipboard text reaches a Wayland paste" "$OUT/t1-src" "$OUT/E1-got"

# E2: an X11 owner sets the clipboard to 512 random bytes, a Wayland paste reads the same bytes.
reset_owners
xclip -quiet -selection clipboard -t application/octet-stream -i <"$OUT/bin-src" >/dev/null 2>&1 &
sleep 2
timeout 10 wl-paste -n -t application/octet-stream >"$OUT/E2-got" 2>/dev/null
compare "E2 X11 binary clipboard reaches a Wayland paste" "$OUT/bin-src" "$OUT/E2-got"

# P1/P2: an X11 primary owner is set while the clipboard holds the binary data. The clipboard must still hold it.
reset_owners
xclip -quiet -selection clipboard -t application/octet-stream -i <"$OUT/bin-src" >/dev/null 2>&1 &
sleep 2
xclip -quiet -selection primary -i <"$OUT/p1-src" >/dev/null 2>&1 &
sleep 2
timeout 10 wl-paste --primary -n >"$OUT/P1-got" 2>/dev/null
compare "P1 X11 primary text reaches a Wayland primary paste" "$OUT/p1-src" "$OUT/P1-got"
timeout 10 wl-paste -n -t application/octet-stream >"$OUT/P2-got" 2>/dev/null
compare "P2 an X11 primary copy leaves the clipboard alone" "$OUT/bin-src" "$OUT/P2-got"

# F1: a Wayland client sets the clipboard text, an X11 read gets it.
reset_owners
wl-copy <"$OUT/t2-src" >/dev/null 2>&1
sleep 2
timeout 10 xclip -selection clipboard -t UTF8_STRING -o >"$OUT/F1-got" 2>/dev/null
compare "F1 Wayland clipboard text reaches an X11 read" "$OUT/t2-src" "$OUT/F1-got"

# F2: a Wayland client sets the clipboard to the 512 random bytes, an X11 read gets the same bytes.
reset_owners
wl-copy -t application/octet-stream <"$OUT/bin-src" >/dev/null 2>&1
sleep 2
timeout 10 xclip -selection clipboard -o -t application/octet-stream >"$OUT/F2-got" 2>/dev/null
compare "F2 Wayland binary clipboard reaches an X11 read" "$OUT/bin-src" "$OUT/F2-got"

# F3: a Wayland client sets the primary selection, an X11 primary read gets it.
reset_owners
wl-copy --primary <"$OUT/p3-src" >/dev/null 2>&1
sleep 2
timeout 10 xclip -selection primary -t UTF8_STRING -o >"$OUT/F3-got" 2>"$OUT/F3-got.err"
compare "F3 Wayland primary text reaches an X11 primary read" "$OUT/p3-src" "$OUT/F3-got"

# C: an X11 owner sets the clipboard, then exits. A Wayland paste must not return its text afterwards.
reset_owners
xclip -quiet -selection clipboard -i <"$OUT/c-src" >/dev/null 2>&1 &
sleep 2
timeout 10 wl-paste -n >"$OUT/C0-got" 2>/dev/null
compare "C0 clipboard holds the X11 text before the clear" "$OUT/c-src" "$OUT/C0-got"
pkill -x xclip 2>/dev/null
sleep 2
timeout 10 wl-paste -n >"$OUT/C1-got" 2>/dev/null
compare "C1 a cleared X11 selection is not pasted from Wayland" "$OUT/empty" "$OUT/C1-got"

echo "failures: $fail" | tee -a "$RESULTS"
exit "$fail"
