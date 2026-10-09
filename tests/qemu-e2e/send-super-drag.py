"""Hold Super and drag with a mouse button through QMP relative events, so the guest sees a normal pointer.

Usage: send-super-drag.py QMP_SOCKET START_X START_Y DELTA_X DELTA_Y [right|left]

The pointer is clamped to the top-left corner with a large relative move, then moved to START. This does
not depend on QEMU's absolute pointer mapping, which is unreliable on a two-GPU desktop.
"""
import json
import socket
import sys
import time

path = sys.argv[1]
start_x, start_y, delta_x, delta_y = (int(value) for value in sys.argv[2:6])
button = sys.argv[6] if len(sys.argv) > 6 else "right"

sock = socket.socket(socket.AF_UNIX)
sock.connect(path)
stream = sock.makefile("rw")


def command(name, **arguments):
    stream.write(json.dumps({"execute": name, "arguments": arguments}) + "\n")
    stream.flush()
    while True:
        reply = json.loads(stream.readline())
        if "return" in reply or "error" in reply:
            return reply


def move(dx, dy):
    command("input-send-event", events=[
        {"type": "rel", "data": {"axis": "x", "value": dx}},
        {"type": "rel", "data": {"axis": "y", "value": dy}},
    ])


stream.readline()
command("qmp_capabilities")
for _ in range(6):
    move(-2000, -2000)
    time.sleep(0.05)
time.sleep(0.3)
# Relative events may be accelerated by the guest, so move in small steps and let the caller verify.
remaining_x, remaining_y = start_x, start_y
while remaining_x > 0 or remaining_y > 0:
    step_x, step_y = min(remaining_x, 40), min(remaining_y, 40)
    move(step_x, step_y)
    remaining_x -= step_x
    remaining_y -= step_y
    time.sleep(0.02)
time.sleep(0.4)
command("input-send-event", events=[{"type": "key", "data": {"down": True, "key": {"type": "qcode", "data": "meta_l"}}}])
time.sleep(0.2)
command("input-send-event", events=[{"type": "btn", "data": {"down": True, "button": button}}])
time.sleep(0.3)
steps = 12
for _ in range(steps):
    move(delta_x // steps, delta_y // steps)
    time.sleep(0.05)
time.sleep(0.4)
command("input-send-event", events=[{"type": "btn", "data": {"down": False, "button": button}}])
time.sleep(0.2)
command("input-send-event", events=[{"type": "key", "data": {"down": False, "key": {"type": "qcode", "data": "meta_l"}}}])
print("dragged", button, "from", start_x, start_y, "by", delta_x, delta_y)
