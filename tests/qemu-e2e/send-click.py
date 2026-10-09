"""Left-click at a desktop position in the guest through QMP. Usage: send-click.py QMP_SOCKET X Y WIDTH HEIGHT
WIDTH and HEIGHT are the size of the whole desktop, so absolute pointer coordinates scale correctly."""
import json
import socket
import sys
import time

path, x, y, width, height = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4]), int(sys.argv[5])
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


stream.readline()
command("qmp_capabilities")
command("input-send-event", events=[
    {"type": "abs", "data": {"axis": "x", "value": int(x / width * 32767)}},
    {"type": "abs", "data": {"axis": "y", "value": int(y / height * 32767)}},
])
time.sleep(0.4)
command("input-send-event", events=[{"type": "btn", "data": {"down": True, "button": "left"}}])
time.sleep(0.1)
command("input-send-event", events=[{"type": "btn", "data": {"down": False, "button": "left"}}])
time.sleep(0.6)
print("clicked", x, y)
