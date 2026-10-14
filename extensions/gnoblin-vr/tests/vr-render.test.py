#!/usr/bin/env python3
"""gnoblin-vr --render-eyes on a published scene: do window frames land where the scene says?

Uses the Vulkan renderer offscreen (lavapipe is enough), so it needs no headset or runtime."""

import os
import struct
import subprocess
import sys
import tempfile

binary = sys.argv[1]
SIZE = 960
BACKGROUND = (8, 9, 13)


def frame(path, colour, alpha=255, width=64, height=64, serial=1):
    r, g, b = colour
    a = alpha
    pixel = bytes([b * a // 255, g * a // 255, r * a // 255, a])
    with open(path, "wb") as handle:
        handle.write(struct.pack("<IIIIQQ", 0x31465647, width, height, width * 4, serial, 0))
        handle.write(pixel * (width * height))


def surface(index, name, space, anchor, local, width, height):
    return (
        f"\n[surface.{index}]\nid=window:{name}\nframe=window_{name}\nspace={space}\ndocked=0\n"
        f"anchor_pos={anchor[0]};{anchor[1]};{anchor[2]}\nanchor_rot=0;0;0;1\n"
        f"local_pos={local[0]};{local[1]};{local[2]}\nlocal_rot=0;0;0;1\nwidth={width}\nheight={height}\n"
    )


def read_ppm(path):
    with open(path, "rb") as handle:
        data = handle.read()
    fields = []
    index = 0
    while len(fields) < 4:
        end = data.index(b"\n", index)
        fields.extend(data[index:end].split())
        index = end + 1
    width, height = int(fields[1]), int(fields[2])
    return width, height, data[index:]


def pixel(image, x, y):
    width, _, data = image
    offset = (y * width + x) * 3
    return tuple(data[offset : offset + 3])


def near(actual, expected, tolerance=3):
    return all(abs(a - b) <= tolerance for a, b in zip(actual, expected))


runtime = tempfile.mkdtemp(prefix="gnoblin-vr-render-")
directory = os.path.join(runtime, "gnoblin-vr")
os.makedirs(os.path.join(directory, "frames"))
frame(os.path.join(directory, "frames", "window_back.rgba"), (255, 0, 0))
frame(os.path.join(directory, "frames", "window_front.rgba"), (0, 0, 255))
frame(os.path.join(directory, "frames", "window_glass.rgba"), (0, 255, 0), alpha=128)
frame(os.path.join(directory, "frames", "window_hud.rgba"), (255, 255, 0))
scene = "[scene]\nrevision=1\ncount=4\n"
# Lowest rank first. back: a large red window two metres ahead of the head; front: a blue one
# overlapping its lower right; glass: half transparent green over the upper left; hud: head-locked.
scene += surface(0, "back", "world", (0, 0, 0), (0, 1.6, -2), 1.6, 1.6)
scene += surface(1, "front", "world", (0, 0, 0), (0.4, 1.3, -1.9), 0.6, 0.6)
scene += surface(2, "glass", "world", (0, 0, 0), (-0.4, 1.9, -1.8), 0.6, 0.6)
scene += surface(3, "hud", "view", (0, 0, 0), (0.45, -0.35, -1.0), 0.2, 0.2)
with open(os.path.join(directory, "scene.ini"), "w") as handle:
    handle.write(scene)

output = os.path.join(runtime, "eyes")
environment = dict(os.environ, XDG_RUNTIME_DIR=runtime, GNOBLIN_VR_HEAD="0,1.6,0")
result = subprocess.run([binary, "--render-eyes", output], env=environment, capture_output=True, text=True, timeout=60)
assert result.returncode == 0, result.stdout + result.stderr
assert "rendered 4 surface(s)" in result.stdout, result.stdout

left = read_ppm(os.path.join(output, "left.ppm"))
right = read_ppm(os.path.join(output, "right.ppm"))
assert left[0] == SIZE and left[1] == SIZE

# Screen position of a point at depth d, lateral x, height y relative to the head: 480 + 480 * (x/d) / tan(50deg).
TAN = 1.19175


def at(x, y, d):
    return int(480 + 480 * (x / d) / TAN), int(480 - 480 * (y / d) / TAN)


assert near(pixel(left, 10, 10), BACKGROUND), pixel(left, 10, 10)
assert near(pixel(left, *at(0.0, 0.0, 2.0)), (255, 0, 0)), "red window centre"
assert near(pixel(left, *at(0.4, -0.3, 1.9)), (0, 0, 255)), "blue window over red"
glass = pixel(left, *at(-0.4, 0.3, 1.8))
# Half transparent green over red: red keeps about half, green gains about half.
assert 110 <= glass[0] <= 146 and 110 <= glass[1] <= 146 and glass[2] <= 4, glass
assert near(pixel(left, *at(0.45, -0.35, 1.0)), (255, 255, 0)), "head-locked window"


# The eyes are 64 mm apart, so the same window edge sits at different columns.
def edge_column(image, row, colour):
    for x in range(SIZE):
        if near(pixel(image, x, row), colour):
            return x
    return None


row = at(0, 0.5, 2.0)[1]
assert edge_column(left, row, (255, 0, 0)) != edge_column(right, row, (255, 0, 0)), "stereo disparity"

# A new frame replaces the old pixels without touching the scene file.
frame(os.path.join(directory, "frames", "window_back.rgba"), (255, 128, 0), serial=2)
result = subprocess.run([binary, "--render-eyes", output], env=environment, capture_output=True, text=True, timeout=60)
assert result.returncode == 0, result.stderr
left = read_ppm(os.path.join(output, "left.ppm"))
assert near(pixel(left, *at(0.0, 0.0, 2.0)), (255, 128, 0)), "updated frame"
if os.environ.get("GNOBLIN_VR_TEST_KEEP"):
    print(output)
print("ok")
