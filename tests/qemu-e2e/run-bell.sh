#!/usr/bin/env bash
# Host: compositor.visual_bell check. Saves the output in the run directory.
#
# PASS needs: no frame changes brightness with visual_bell off, a frame does with fullscreen-flash, and
# frame-flash changes less of the screen than fullscreen-flash.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"
here="$root/tests/qemu-e2e"
prefix="${GNOBLIN_PREFIX:-$root/build/validation-install}"
run="${GNOBLIN_QEMU_RUN:-$(cat /tmp/gnoblin-run)}"
port="$(cat "$run/ssh-port")"
out="$run/bell-$(date -u +%Y%m%dT%H%M%SZ).txt"
{
    echo "command: tests/qemu-e2e/run-bell.sh"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" "$here/guest-click.py" luna@127.0.0.1:/tmp/
    scripts/qemu-e2e guest "$run" -- env GNOBLIN_PREFIX="$prefix" bash -s < "$here/guest-bell.sh"
    rm -rf "$run/bell"
    mkdir -p "$run/bell"
    scp -q -P "$port" -o UserKnownHostsFile="$run/known_hosts" 'luna@127.0.0.1:/tmp/bell-*.png' "$run/bell/"
    python3 -I - "$run/bell" <<'PY'
import glob
import sys

from PIL import Image, ImageChops, ImageStat

directory = sys.argv[1]


def frames(mode):
    return sorted(glob.glob(f"{directory}/bell-{mode}-*.png"))


def brightness(path):
    return sum(ImageStat.Stat(Image.open(path).convert("RGB")).mean) / 3


def flashed_area(mode):
    """Fraction of the screen that differs between the darkest and brightest frame."""
    paths = frames(mode)
    means = [brightness(path) for path in paths]
    if not paths or max(means) - min(means) < 1:
        return 0.0
    low = Image.open(paths[means.index(min(means))]).convert("RGB")
    high = Image.open(paths[means.index(max(means))]).convert("RGB")
    diff = ImageChops.difference(low, high).convert("L").point(lambda value: 255 if value > 8 else 0)
    return sum(1 for pixel in diff.getdata() if pixel) / (low.size[0] * low.size[1])


area = {mode: flashed_area(mode) for mode in ("off", "on", "frame")}
for mode in area:
    print(f"visual_bell {mode}: frames={len(frames(mode))} flashed screen fraction={area[mode]:.2f}")
failures = 0


def check(name, ok):
    global failures
    print(("PASS " if ok else "FAIL ") + name)
    failures += 0 if ok else 1


check("no flash with visual_bell off", area["off"] == 0)
check("fullscreen-flash covers the screen", area["on"] > 0.9)
check("frame-flash covers only part of the screen", 0.01 < area["frame"] < 0.5)
print(f"failures: {failures}")
sys.exit(failures)
PY
} 2>&1 | tee "$out"
echo "saved: $out"
