#!/usr/bin/env bash
# Host: cursor theme and size check. Saves the output in the run directory.
#
# Needs no hyprcursor. It checks: a custom Xcursor theme changes the sprite, a missing theme falls back to
# the default sprite, a larger size gives a larger sprite, and sizes outside 1 to 256 are rejected.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/cursor-themes-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-cursor-themes.sh"
    guest_out="$(scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-cursor-themes.sh")"
    printf '%s\n' "$guest_out"
    rm -f "$run"/cur-*.png
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" 'luna@127.0.0.1:/tmp/cur-*.png' "$run/"
    GUEST_OUT="$guest_out" python3 -I - "$run" <<'PY'
import os
import sys

from PIL import Image, ImageChops

run = sys.argv[1]
guest = dict(line.split("=", 1) for line in os.environ["GUEST_OUT"].splitlines() if "=" in line)


def crop(name):
    return Image.open(f"{run}/cur-{name}.png").convert("RGB").crop((280, 180, 560, 460))


def width(name):
    base = Image.new("RGB", crop(name).size, (128, 128, 128))
    box = ImageChops.difference(crop(name), base).getbbox()
    return (box[2] - box[0]) if box else 0


def same(a, b):
    return ImageChops.difference(crop(a), crop(b)).getbbox() is None


failures = 0


def check(name, ok):
    global failures
    print(("PASS " if ok else "FAIL ") + name)
    failures += 0 if ok else 1


check("default cursor is drawn", width("default") > 0)
check("a custom Xcursor theme changes the sprite", not same("default", "custom"))
check("a missing theme falls back to the default sprite", same("default", "missing"))
check("size 192 draws a larger sprite than size 128", width("large") > width("default"))
for bad in ("0", "257", "-5"):
    check(f"size {bad} is rejected", guest.get(f"reject-size-{bad}") == "rejected")
check("compositor is still running", guest.get("compositor-alive") == "1")
print(f"failures: {failures}")
sys.exit(failures)
PY
} 2>&1 | tee "$out"
echo "saved: $out"
