#!/usr/bin/env python3
"""Drive the test guest through QEMU's QMP socket: a capture of its display, typed keys, pointer motion and clicks.

    qmp.py SOCKET screendump OUT.png   the guest's display as the scan-out shows it (2D virtio-vga only: QEMU cannot
                                       read a virgl GL scan-out back)
    qmp.py SOCKET type TEXT            TEXT as key presses on the guest's virtio keyboard (a-z, 0-9, space, '-', '.')
    qmp.py SOCKET key NAME             one QEMU key code, such as ret or esc
    qmp.py SOCKET move DX DY           relative motion on the guest's virtio mouse, in steps of at most 20
    qmp.py SOCKET click [BUTTON]       press and release a mouse button (left by default)

Every input reaches the guest as a device event, so it travels libinput -> compositor -> client as a person's would.
"""

import json
import socket
import sys
import time

NAMED = {" ": "spc", "-": "minus", ".": "dot"}


class Qmp:
    def __init__(self, path: str) -> None:
        self.sock = socket.socket(socket.AF_UNIX)
        self.sock.connect(path)
        self.file = self.sock.makefile("rwb")
        self.file.readline()
        self.call("qmp_capabilities")

    def call(self, command: str, **arguments: object) -> object:
        message: dict[str, object] = {"execute": command}
        if arguments:
            message["arguments"] = arguments
        self.file.write(json.dumps(message).encode() + b"\n")
        self.file.flush()
        while True:
            reply = json.loads(self.file.readline())
            if "error" in reply:
                raise SystemExit(f"[qmp] {command}: {reply['error'].get('desc', reply['error'])}")
            if "return" in reply:
                return reply["return"]

    def events(self, *events: dict[str, object]) -> None:
        self.call("input-send-event", events=list(events))


def key_code(ch: str) -> str:
    if ch.isascii() and (ch.islower() or ch.isdigit()):
        return ch
    if ch in NAMED:
        return NAMED[ch]
    raise SystemExit(f"[qmp] no key for {ch!r}")


def main(argv: list[str]) -> int:
    if len(argv) < 3:
        print(__doc__, file=sys.stderr)
        return 2
    qmp = Qmp(argv[1])
    action, rest = argv[2], argv[3:]
    if action == "screendump":
        qmp.call("screendump", filename=rest[0], format="png")
    elif action in ("type", "key"):
        keys = [key_code(ch) for ch in rest[0]] if action == "type" else [rest[0]]
        for key in keys:
            qmp.call("send-key", keys=[{"type": "qcode", "data": key}])
            time.sleep(0.03)
    elif action == "move":
        dx, dy = int(rest[0]), int(rest[1])
        while dx or dy:
            step_x, step_y = max(-20, min(20, dx)), max(-20, min(20, dy))
            qmp.events(
                {"type": "rel", "data": {"axis": "x", "value": step_x}},
                {"type": "rel", "data": {"axis": "y", "value": step_y}},
            )
            dx, dy = dx - step_x, dy - step_y
            time.sleep(0.02)
    elif action == "click":
        button = rest[0] if rest else "left"
        for down in (True, False):
            qmp.events({"type": "btn", "data": {"down": down, "button": button}})
            time.sleep(0.1)
    else:
        print(__doc__, file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
