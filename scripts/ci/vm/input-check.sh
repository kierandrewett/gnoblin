#!/usr/bin/env bash
# Input through the real path: QMP device events -> the guest's evdev -> libinput -> gnoblin on its DRM backend -> a Wayland client.
#
#   scripts/ci/vm/input-check.sh QMP_SOCKET SHELL_PATH OUTDIR
#
# QMP_SOCKET  the QEMU QMP socket of the guest, which has a virtio keyboard, mouse and tablet, the tablet added last
# SHELL_PATH  the shell library gnoblin loads, as a path inside the guest
# OUTDIR      where the logs and the results go (input-results.txt: `check|pass|detail` per line)
# GUEST_CMD   (environment) the words that run one shell command line in the guest, such as `ssh -p 2242 user@127.0.0.1`
# GUEST_SESSION (environment) the guest's copy of guest-session.sh, `./guest-session.sh` by default
#
# Two sessions run wev, which prints what a client receives, and each is a check of its own so one device never masks the
# other. The relative mouse moves into the window, a button is pressed (QEMU gives it to the tablet, see qmp.py), and two keys
# are typed. The absolute tablet is sent to a point inside wev's window (the top left of the output), then a little to the
# right, clicks, and two keys are typed; the compositor runs with `backend:debug` so the
# positions it placed the tablet at are in its log, and that position is checked against where the tablet was sent. The
# second one separates a wrong mapping (the pointer lands elsewhere than it was sent) from a lost event (wev hears nothing).
# Exit status is 1 when a check failed.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
qmp=${1:?usage: input-check.sh QMP_SOCKET SHELL_PATH OUTDIR}
shell_path=${2:?SHELL_PATH}
out=$(mkdir -p "${3:?OUTDIR}" && cd "$3" && pwd)
session=${GUEST_SESSION:-./guest-session.sh}
: "${GUEST_CMD:?GUEST_CMD names how a command reaches the guest}"
results=$out/input-results.txt
: >"$results"
failed=0

guest() { $GUEST_CMD "$@"; }
# check NAME pass|fail DETAIL
check() {
    echo "$1|$2|$3" >>"$results"
    echo "[input] $1: $2 $3"
    [ "$2" = pass ] || failed=1
}
# start_session LOGSPEC NAME: a session whose only client is wev, and what it and gnoblin logged under $out/NAME.*.
start_session() {
    guest "GNOBLIN_LOG=$1 $session $shell_path 'stdbuf -oL wev'" >"$out/$2.session.txt" 2>&1 || true
    sleep 5
}
collect() {
    guest 'cat ~/session/client.log' >"$out/$1.wev.log" 2>&1 || true
    guest 'cat ~/session/gnoblin.log' >"$out/$1.gnoblin.log" 2>&1 || true
}
# keyboard_check NAME PREFIX: wev got keyboard focus and the two keys.
keyboard_check() {
    if grep -q 'wl_keyboard\] enter' "$out/$1.wev.log" && grep -qE 'wl_keyboard\] key: .*state: 1' "$out/$1.wev.log"; then
        check "$2" pass "wev got keyboard focus and the keys"
    else
        check "$2" fail "wev got no keyboard enter or keys (see $1.wev.log)"
    fi
}

# The relative mouse.
start_session info input
"$here/qmp.py" "$qmp" move 200 150
"$here/qmp.py" "$qmp" click
sleep 1
"$here/qmp.py" "$qmp" type ab
sleep 2
collect input
if ! grep -q 'the shell .* serves' "$out/input.gnoblin.log"; then
    check input-shell fail "the shell did not start, so the input checks test the fallback (see input.gnoblin.log)"
fi
if grep -q 'wl_pointer\] enter' "$out/input.wev.log" && grep -q 'wl_pointer\] button' "$out/input.wev.log"; then
    check pointer pass "wev got pointer enter and the click"
else
    check pointer fail "wev got no pointer enter or click (see input.wev.log)"
fi
keyboard_check input keyboard-focus

# The absolute tablet: a point inside wev's window, which sits at the top left of the output, then a little to the right of it.
start_session backend:debug tablet
"$here/qmp.py" "$qmp" abs 0.15 0.2
sleep 1
"$here/qmp.py" "$qmp" abs 0.25 0.2
sleep 1
"$here/qmp.py" "$qmp" click
sleep 1
"$here/qmp.py" "$qmp" type ab
sleep 2
collect tablet
# The output's size is in the log as the mode gnoblin set; the points the mapper placed are `PointerMotion` events.
placed=$(
    python3 - "$out/tablet.gnoblin.log" <<'PY'
import re
import sys

text = open(sys.argv[1], errors="replace").read()
size = re.search(r"size: \((\d+), (\d+)\)", text)
points = [(int(x), int(y)) for x, y in re.findall(r"PointerMotion \{[^}]*?position: Point \{ x: (-?\d+), y: (-?\d+) \}", text)]
if not size:
    print("no output size in the log")
elif not points:
    print("no PointerMotion event reached the seat")
else:
    w, h = int(size.group(1)), int(size.group(2))
    wanted = [(round(0.15 * w), round(0.2 * h)), (round(0.25 * w), round(0.2 * h))]
    near = lambda p, q: abs(p[0] - q[0]) <= 2 and abs(p[1] - q[1]) <= 2
    if any(near(p, wanted[0]) for p in points) and any(near(p, wanted[1]) for p in points):
        print("ok " + f"{wanted[0]} and {wanted[1]} of {w}x{h}")
    else:
        print(f"the tablet was sent to {wanted} of {w}x{h}; the seat got {sorted(set(points))[:6]}")
PY
)
if [[ $placed == ok* ]]; then
    check tablet-position pass "the seat placed the tablet where it was sent (${placed#ok })"
else
    check tablet-position fail "$placed (see tablet.gnoblin.log)"
fi
if grep -q 'wl_pointer\] enter' "$out/tablet.wev.log" && grep -q 'wl_pointer\] motion' "$out/tablet.wev.log" &&
    grep -q 'wl_pointer\] button' "$out/tablet.wev.log"; then
    check tablet-pointer pass "wev got pointer enter, motion and the click"
else
    check tablet-pointer fail "wev got no pointer enter, motion or click (see tablet.wev.log)"
fi
keyboard_check tablet tablet-keyboard
exit "$failed"
