#!/usr/bin/env bash
# Host: the display must show the locker surface while the session is locked. The other lock checks read state or
# capture refusal; this one reads the pixels the guest scans out. swaylock paints one colour (red) over a green desktop.
# Needs vncdo (pip install vncdotool) and PIL on the host. A black screen means the compositor shows only its cover.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
out="$run/session-lock-pixels-$(date -u +%Y%m%dT%H%M%SZ).txt"
vnc="127.0.0.1::$(cat "$run/vnc-port")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
fail=0

guest() { scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s <<<"$1"; }

# Prints the share of pixels near each reference colour, as "red green black".
shares() {
    vncdo -s "$vnc" capture "$work/shot.png" >/dev/null 2>&1
    python3 - "$work/shot.png" <<'PY'
import sys
from PIL import Image
im = Image.open(sys.argv[1]).convert("RGB")
px = list(im.getdata())
def share(c):
    return sum(1 for p in px if max(abs(a - b) for a, b in zip(p, c)) <= 8) / len(px)
print("%.3f %.3f %.3f" % (share((255, 0, 0)), share((0, 170, 0)), share((0, 0, 0))))
PY
}

check() {
    # check NAME ACTUAL_SHARE MINIMUM_SHARE
    if awk -v a="$2" -v m="$3" 'BEGIN { exit !(a >= m) }'; then
        echo "PASS $1 ($2)"
    else
        echo "FAIL $1 (share $2, expected at least $3)"
        fail=$((fail + 1))
    fi
}

{
    echo "command: tests/qemu-e2e/run-session-lock-pixels.sh"
    guest 'export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0
(setsid swaybg -c 00aa00 >/dev/null 2>&1 &)
sleep 3'
    read -r red green _ <<<"$(shares)"
    check "the unlocked desktop shows the green background" "$green" 0.99

    guest 'export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0
(setsid swaylock -f -c ff0000 >/dev/null 2>&1 &)
sleep 4
"$GNOBLIN_PREFIX/bin/gnoblinctl" status | head -1'
    read -r red green _ <<<"$(shares)"
    check "the locked screen shows the locker colour" "$red" 0.99

    guest 'export XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0
pkill -USR1 -x swaylock
sleep 3
pkill -x swaybg || true'
    echo "failures: $fail"
} 2>&1 | tee "$out"
echo "saved: $out"
exit "$(grep -c '^FAIL' "$out")"
