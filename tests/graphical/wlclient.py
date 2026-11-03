#!/usr/bin/env python3
"""The Wayland client of the graphical VM test. It needs the Python standard library only.

usage: wlclient.py --globals FILE --events FILE [--width W] [--height H] [--seconds N]

It runs in the guest, in the Gnoblin session. It speaks the Wayland wire protocol over the compositor socket, so the
guest needs no toolkit, no compiler and no client package: Python 3 is there on every system that Gnoblin installs on.

  1. It writes every global that the registry announces to --globals as JSON.
  2. It maps one xdg_toplevel (app id org.gnoblin.GraphicalTest) of W x H pixels, filled with the first colour
     (magenta). A test outside the guest finds the window in a screenshot by this colour.
  3. It writes each pointer and keyboard event of its surface to --events, one JSON object per line.
  4. When the key G (evdev code 34) is pressed, it fills the window with the second colour (green). A screenshot
     shows that a key from outside the guest reached the client and the client drew again.

It stops on xdg_toplevel.close, on SIGTERM or after --seconds. A protocol error ends it with status 2.
"""

import argparse
import array
import json
import mmap
import os
import select
import socket
import struct
import sys
import time

FIRST_COLOUR = (255, 0, 255)
SECOND_COLOUR = (0, 200, 0)
SWITCH_KEY = 34  # KEY_G
APP_ID = "org.gnoblin.GraphicalTest"
TITLE = "Gnoblin graphical test"
WL_SHM_FORMAT_XRGB8888 = 1
SEAT_POINTER, SEAT_KEYBOARD = 1, 2


def u32(value):
    return struct.pack("<I", value & 0xFFFFFFFF)


def i32(value):
    return struct.pack("<i", value)


def string(text):
    data = text.encode() + b"\0"
    return u32(len(data)) + data + b"\0" * (-len(data) % 4)


class Reader:
    """Reads the arguments of one event, in order."""

    def __init__(self, payload):
        self.payload, self.offset = payload, 0

    def u32(self):
        value = struct.unpack_from("<I", self.payload, self.offset)[0]
        self.offset += 4
        return value

    def i32(self):
        value = struct.unpack_from("<i", self.payload, self.offset)[0]
        self.offset += 4
        return value

    def fixed(self):
        return self.i32() / 256.0

    def string(self):
        length = self.u32()
        data = self.payload[self.offset : self.offset + length]
        self.offset += length + (-length % 4)
        return data.rstrip(b"\0").decode(errors="replace")


class Connection:
    def __init__(self, path):
        self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.socket.connect(path)
        self.next_id = 2
        self.interfaces = {1: "wl_display"}
        self.buffer = b""

    def new(self, interface):
        object_id = self.next_id
        self.next_id += 1
        self.interfaces[object_id] = interface
        return object_id

    def send(self, object_id, opcode, payload=b"", fd=None):
        message = u32(object_id) + u32(((8 + len(payload)) << 16) | opcode) + payload
        ancillary = []
        if fd is not None:
            ancillary = [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", [fd]).tobytes())]
        self.socket.sendmsg([message], ancillary)

    def events(self, timeout):
        """The complete events that arrive within timeout seconds, as (object id, opcode, payload)."""
        ready, _, _ = select.select([self.socket], [], [], timeout)
        if ready:
            data, ancillary, _, _ = self.socket.recvmsg(65536, socket.CMSG_SPACE(16 * 4))
            if not data:
                raise ConnectionError("the compositor closed the connection")
            for level, kind, payload in ancillary:
                if level == socket.SOL_SOCKET and kind == socket.SCM_RIGHTS:
                    # The only event with a file descriptor that this client gets is wl_keyboard.keymap. It does not
                    # read the keymap: it logs evdev key codes.
                    for fd in array.array("i", payload[: len(payload) - len(payload) % 4]):
                        os.close(fd)
            self.buffer += data
        result = []
        while len(self.buffer) >= 8:
            object_id, word = struct.unpack_from("<II", self.buffer)
            size = word >> 16
            if size < 8 or len(self.buffer) < size:
                break
            result.append((object_id, word & 0xFFFF, self.buffer[8:size]))
            self.buffer = self.buffer[size:]
        return result


def shared_memory(size):
    """A file descriptor of size bytes that the compositor can map."""
    if hasattr(os, "memfd_create"):
        fd = os.memfd_create("gnoblin-graphical-test")
    else:  # Python 3.7 and older
        path = os.path.join(os.environ["XDG_RUNTIME_DIR"], f"gnoblin-graphical-test-{os.getpid()}")
        fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_EXCL, 0o600)
        os.unlink(path)
    os.ftruncate(fd, size)
    return fd


def fill(memory, offset, width, height, colour):
    red, green, blue = colour
    row = bytes([blue, green, red, 255]) * width
    for line in range(height):
        start = offset + line * width * 4
        memory[start : start + width * 4] = row


class Client:
    def __init__(self, arguments):
        self.arguments = arguments
        runtime = os.environ["XDG_RUNTIME_DIR"]
        display = os.environ.get("WAYLAND_DISPLAY", "wayland-0")
        self.wire = Connection(display if display.startswith("/") else os.path.join(runtime, display))
        self.events_file = open(arguments.events, "a", buffering=1)
        self.globals = []
        self.done_callbacks = set()
        self.configured = False
        self.mapped = False
        self.closed = False
        self.colour = None
        self.pointer = self.keyboard = None
        self.seat = None

    def log(self, event, **fields):
        fields.update(event=event, time=round(time.monotonic(), 3))
        self.events_file.write(json.dumps(fields) + "\n")

    def roundtrip(self):
        callback = self.wire.new("wl_callback")
        self.wire.send(1, 0, u32(callback))
        while callback not in self.done_callbacks:
            self.dispatch(1.0)

    def bind(self, interface, highest):
        match = [item for item in self.globals if item["interface"] == interface]
        if not match:
            raise RuntimeError(f"the compositor has no {interface}")
        version = min(match[0]["version"], highest)
        object_id = self.wire.new(interface)
        self.wire.send(self.registry, 0, u32(match[0]["name"]) + string(interface) + u32(version) + u32(object_id))
        return object_id

    def dispatch(self, timeout):
        for object_id, opcode, payload in self.wire.events(timeout):
            self.handle(self.wire.interfaces.get(object_id), object_id, opcode, Reader(payload))

    def handle(self, interface, object_id, opcode, event):
        if interface == "wl_display" and opcode == 0:
            failed, code, message = event.u32(), event.u32(), event.string()
            self.log("protocol-error", object=failed, code=code, message=message)
            print(f"wlclient: protocol error on object {failed}: {code} {message}", file=sys.stderr)
            raise SystemExit(2)
        if interface == "wl_registry" and opcode == 0:
            name, name_interface, version = event.u32(), event.string(), event.u32()
            self.globals.append({"name": name, "interface": name_interface, "version": version})
        elif interface == "wl_callback" and opcode == 0:
            self.done_callbacks.add(object_id)
            if object_id == getattr(self, "frame_callback", None) and not self.mapped:
                self.mapped = True
                self.log("mapped", width=self.arguments.width, height=self.arguments.height)
        elif interface == "xdg_wm_base" and opcode == 0:
            self.wire.send(object_id, 3, u32(event.u32()))
        elif interface == "xdg_surface" and opcode == 0:
            serial = event.u32()
            self.wire.send(self.xdg_surface, 4, u32(serial))
            if not self.configured:
                self.configured = True
                self.log("configured", serial=serial)
                self.draw(0, FIRST_COLOUR)
        elif interface == "xdg_toplevel" and opcode == 1:
            self.log("close")
            self.closed = True
        elif interface == "wl_seat" and opcode == 0:
            capabilities = event.u32()
            if capabilities & SEAT_POINTER and self.pointer is None:
                self.pointer = self.wire.new("wl_pointer")
                self.wire.send(self.seat, 0, u32(self.pointer))
            if capabilities & SEAT_KEYBOARD and self.keyboard is None:
                self.keyboard = self.wire.new("wl_keyboard")
                self.wire.send(self.seat, 1, u32(self.keyboard))
            self.log("seat", capabilities=capabilities)
        elif interface == "wl_pointer":
            self.pointer_event(opcode, event)
        elif interface == "wl_keyboard":
            self.keyboard_event(opcode, event)

    def pointer_event(self, opcode, event):
        if opcode == 0:
            event.u32()
            surface = event.u32()
            self.log("pointer-enter", surface=surface == self.surface, x=event.fixed(), y=event.fixed())
        elif opcode == 1:
            self.log("pointer-leave")
        elif opcode == 2:
            event.u32()
            self.log("pointer-motion", x=event.fixed(), y=event.fixed())
        elif opcode == 3:
            event.u32()
            event.u32()
            self.log("pointer-button", button=event.u32(), pressed=event.u32() == 1)

    def keyboard_event(self, opcode, event):
        if opcode == 1:
            event.u32()
            self.log("keyboard-enter", surface=event.u32() == self.surface)
        elif opcode == 2:
            self.log("keyboard-leave")
        elif opcode == 3:
            event.u32()
            event.u32()
            key, pressed = event.u32(), event.u32() == 1
            self.log("key", key=key, pressed=pressed)
            if key == SWITCH_KEY and pressed and self.colour != SECOND_COLOUR:
                self.draw(1, SECOND_COLOUR)
                self.log("repainted", colour=list(SECOND_COLOUR))

    def draw(self, index, colour):
        width, height = self.arguments.width, self.arguments.height
        fill(self.memory, index * width * height * 4, width, height, colour)
        self.frame_callback = self.wire.new("wl_callback")
        self.wire.send(self.surface, 1, u32(self.buffers[index]) + i32(0) + i32(0))
        self.wire.send(self.surface, 2, i32(0) + i32(0) + i32(width) + i32(height))
        self.wire.send(self.surface, 3, u32(self.frame_callback))
        self.wire.send(self.surface, 6)
        self.colour = colour

    def run(self):
        arguments = self.arguments
        self.registry = self.wire.new("wl_registry")
        self.wire.send(1, 1, u32(self.registry))
        self.roundtrip()
        with open(arguments.globals, "w") as handle:
            json.dump(sorted(self.globals, key=lambda item: item["interface"]), handle, indent=1)
        compositor = self.bind("wl_compositor", 4)
        shm = self.bind("wl_shm", 1)
        wm_base = self.bind("xdg_wm_base", 1)
        # Without a seat the window still maps. The input checks then fail on the missing events.
        if any(item["interface"] == "wl_seat" for item in self.globals):
            self.seat = self.bind("wl_seat", 5)

        width, height = arguments.width, arguments.height
        size = 2 * width * height * 4
        fd = shared_memory(size)
        self.memory = mmap.mmap(fd, size)
        pool = self.wire.new("wl_shm_pool")
        self.wire.send(shm, 0, u32(pool) + i32(size), fd=fd)
        self.buffers = []
        for index in (0, 1):
            buffer = self.wire.new("wl_buffer")
            self.wire.send(
                pool, 0, u32(buffer) + i32(index * width * height * 4) + i32(width) + i32(height)
                + i32(width * 4) + u32(WL_SHM_FORMAT_XRGB8888)
            )  # fmt: skip
            self.buffers.append(buffer)
        os.close(fd)

        self.surface = self.wire.new("wl_surface")
        self.wire.send(compositor, 0, u32(self.surface))
        self.xdg_surface = self.wire.new("xdg_surface")
        self.wire.send(wm_base, 2, u32(self.xdg_surface) + u32(self.surface))
        toplevel = self.wire.new("xdg_toplevel")
        self.wire.send(self.xdg_surface, 1, u32(toplevel))
        self.wire.send(toplevel, 2, string(TITLE))
        self.wire.send(toplevel, 3, string(APP_ID))
        self.wire.send(self.surface, 6)
        self.log("started", pid=os.getpid())

        deadline = time.monotonic() + arguments.seconds
        while not self.closed and time.monotonic() < deadline:
            self.dispatch(0.5)
        self.log("exit", closed=self.closed)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--globals", required=True, help="write the registry globals here, as JSON")
    parser.add_argument("--events", required=True, help="append the input events here, one JSON object per line")
    parser.add_argument("--width", type=int, default=480)
    parser.add_argument("--height", type=int, default=360)
    parser.add_argument("--seconds", type=float, default=900)
    arguments = parser.parse_args()
    try:
        Client(arguments).run()
    except (OSError, RuntimeError, KeyError) as exception:
        print(f"wlclient: {exception}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
