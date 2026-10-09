#!/usr/bin/env bash
# Guest: measure how much of a striped wallpaper shows through a translucent window with and without a blur rule.
set -u
export XDG_RUNTIME_DIR="/run/user/$(id -u)"
export WAYLAND_DISPLAY=wayland-0
G="${GNOBLIN_PREFIX:?set GNOBLIN_PREFIX to the prefix synced into the guest}/bin/gnoblinctl"
CONFIG="$HOME/.config/gnoblin/config/99-test-blur.lua"

python3 - <<'PY'
import struct
import zlib

width, height, period = 2560, 800, 40
row = b"".join((b"\xff\xff\xff" if (x // (period // 2)) % 2 == 0 else b"\x00\x00\x00") for x in range(width))
raw = b"".join(b"\x00" + row for _ in range(height))


def chunk(kind, data):
    body = kind + data
    return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)


png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
png += chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b"")
open("/tmp/stripes.png", "wb").write(png)
PY

contrast() {
    # contrast: max minus min of the red channel along a row inside the blur-test window
    local geometry
    geometry="$("$G" window list 2>&1 | python3 -c '
import json, sys
for w in json.load(sys.stdin)["windows"]:
    if w["title"] == "blur-test":
        f = w["frame"]
        print(f["x"] + 80, f["y"] + f["height"] // 2, f["width"] - 160)')"
    grim -t ppm /tmp/blur.ppm
    python3 - $geometry <<'PY'
import sys
x, y, span = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3])
data = open("/tmp/blur.ppm", "rb").read()
parts = data.split(b"\n", 3)
width = int(parts[1].split()[0])
pixels = parts[3]
reds = [pixels[(y * width + xx) * 3] for xx in range(x, x + span)]
print(max(reds) - min(reds))
PY
}

nohup swaybg -i /tmp/stripes.png -m stretch >/dev/null 2>&1 < /dev/null &
nohup foot -T blur-test -o colors.alpha=0.5 >/dev/null 2>&1 < /dev/null &
sleep 4
before="$(contrast)"
echo "contrast without a blur rule: $before"
cp /tmp/99-test-blur.lua "$CONFIG"
"$G" config reload | tail -1
sleep 3
after="$(contrast)"
echo "contrast with blur = 40: $after"
if [ "$before" -gt 60 ] && [ "$after" -lt $((before / 2)) ]; then
    echo "PASS blur rule smooths the wallpaper behind a translucent window ($before -> $after)"
else
    echo "FAIL blur rule smooths the wallpaper behind a translucent window ($before -> $after)"
fi
pkill -x swaybg
pkill -f "foot -T blur-test"
printf -- '-- disabled\n' > "$CONFIG"
"$G" config reload | tail -1
