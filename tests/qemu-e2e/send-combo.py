"""Send one key combination to the guest through QMP. Usage: shortcut-combo.py QMP_SOCKET qcode..."""
import json
import socket
import sys
import time

sock = socket.socket(socket.AF_UNIX)
sock.connect(sys.argv[1])
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
keys = [{"type": "qcode", "data": code} for code in sys.argv[2:]]
print(command("send-key", keys=keys, **{"hold-time": 200}))
time.sleep(0.5)
