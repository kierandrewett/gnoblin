#!/usr/bin/env python3
"""Run the real idle service and CLI on a private D-Bus and compositor socket.

Usage: python3 tests/idle-active-changed.py --idle BUILD/gnoblin-idle --ctl BUILD/gnoblinctl
No host service or compositor is started or modified. Requires PyGObject.
"""

import argparse
import json
import os
from pathlib import Path
import select
import shutil
import socket
import subprocess
import tempfile
import threading
import time

from gi.repository import Gio, GLib


NAME = "org.freedesktop.ScreenSaver"
PATH = "/org/freedesktop/ScreenSaver"
LOCK_EVENT = "gnoblin.session.lock-state-changed"
GNOME = "org.gnome.ScreenSaver"


def wait_for(predicate, message, timeout=4):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        while GLib.MainContext.default().iteration(False):
            pass
        if predicate():
            return
        time.sleep(0.01)
    raise AssertionError(message)


def settle(seconds=0.15):
    deadline = time.monotonic() + seconds
    wait_for(lambda: time.monotonic() >= deadline, "main loop did not settle")


class Compositor:
    """Only the existing session.status, session.lock and event wire contracts."""

    def __init__(self, path):
        self.path = path
        self.state = "unlocked"
        self.revision = 0
        self.connections = {}
        self.snapshots = 0
        self.requests = 0
        self.mutex = threading.RLock()
        self.errors = []
        self.listener = socket.socket(socket.AF_UNIX)
        self.listener.bind(str(path))
        self.listener.listen()
        self.stopping = threading.Event()
        self.threads = []
        self.thread = threading.Thread(target=self.accept, daemon=True)
        self.thread.start()

    @staticmethod
    def send(conn, value):
        conn.sendall((json.dumps(value) + "\n").encode())

    def accept(self):
        while not self.stopping.is_set():
            if not select.select([self.listener], [], [], 0.05)[0]:
                continue
            conn, _ = self.listener.accept()
            with self.mutex:
                self.connections[conn] = []
            thread = threading.Thread(target=self.serve, args=(conn,), daemon=True)
            self.threads.append(thread)
            thread.start()

    def serve(self, conn):
        try:
            with conn, conn.makefile("rb") as stream:
                # client_connected() sends this before reading any requests. Include the
                # greeting on every connection, including GetActive and reconnects.
                self.send(
                    conn,
                    {
                        "event": "hello",
                        "version": 1,
                        "api_major": 1,
                        "api_minor": 81,
                        "state_revision": 0,
                        "revision_scope": "window-workspace-monitor-layer-input-device-source-capability-state",
                        "revision_semantics": "monotonic-per-session",
                        "methods": ["session.status", "session.lock"],
                        "events": [LOCK_EVENT],
                        "capabilities": [],
                        "features": [],
                    },
                )
                for line in stream:
                    request = json.loads(line)
                    with self.mutex:
                        if request["op"] == "events":
                            self.connections[conn] = request["events"]
                            self.send(
                                conn,
                                {
                                    "event": "subscribed",
                                    "events": request["events"],
                                    "api_version": request["api_version"],
                                },
                            )
                            continue
                        assert request["op"] == "api", request
                        method = request["method"]
                        if method == "session.status":
                            result = {
                                "state": "running",
                                "lock_available": True,
                                "lock_state": self.state,
                                "revision": self.revision,
                            }
                            if LOCK_EVENT in self.connections[conn]:
                                self.snapshots += 1
                        elif method == "session.lock":
                            self.requests += 1
                            result = {"dispatched": True, "subscribers": 1}
                        else:
                            raise AssertionError(request)
                        self.send(conn, {"event": "reply", "id": request["id"], "result": result})
        except (BrokenPipeError, ConnectionResetError):
            pass
        except BaseException as error:
            self.errors.append(error)
        finally:
            with self.mutex:
                self.connections.pop(conn, None)

    def transition(self, state, send=True):
        with self.mutex:
            self.state = state
            self.revision += 1
            if send:
                for conn, events in list(self.connections.items()):
                    if LOCK_EVENT in events:
                        self.send(
                            conn,
                            {
                                "event": LOCK_EVENT,
                                "state": state,
                                "revision": self.revision,
                                "sequence": self.revision,
                                "time": time.monotonic_ns() // 1000,
                            },
                        )

    def disconnect(self):
        with self.mutex:
            for conn in list(self.connections):
                conn.shutdown(socket.SHUT_RDWR)

    def inject(self, value):
        with self.mutex:
            for conn, events in list(self.connections.items()):
                if LOCK_EVENT in events:
                    conn.sendall(value)

    def close(self):
        if self.stopping.is_set():
            return
        self.stopping.set()
        self.thread.join(2)
        self.disconnect()
        for thread in self.threads:
            thread.join(2)
        self.listener.close()
        self.path.unlink()
        assert not self.errors, self.errors


class Gnome:
    """Serve GNOME on a separate context so synchronous delegation can complete."""

    def __init__(self, bus):
        self.bus = bus
        self.active = True
        self.ready = threading.Event()
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()
        assert self.ready.wait(3), "GNOME fixture did not start"

    def serve(self):
        context = GLib.MainContext.new()
        context.push_thread_default()
        try:
            self.connection = self.bus.connect()
            node = Gio.DBusNodeInfo.new_for_xml(
                f"<node><interface name='{GNOME}'>"
                "<method name='GetActive'><arg type='b' direction='out'/></method>"
                "<method name='SetActive'><arg type='b' direction='in'/></method>"
                "<method name='Lock'/><signal name='ActiveChanged'><arg type='b'/></signal>"
                "</interface></node>"
            )
            self.connection.register_object("/org/gnome/ScreenSaver", node.interfaces[0], self.call)
            assert self.bus.dbus("RequestName", GLib.Variant("(su)", (GNOME, 4)), "(u)", self.connection) == (1,)
            self.loop = GLib.MainLoop.new(context, False)
            self.ready.set()
            self.loop.run()
        finally:
            context.pop_thread_default()

    def call(self, connection, sender, path, interface, method, parameters, invocation):
        if method == "GetActive":
            invocation.return_value(GLib.Variant("(b)", (self.active,)))
        else:
            self.active = parameters.unpack()[0] if method == "SetActive" else True
            self.connection.emit_signal(
                None, "/org/gnome/ScreenSaver", GNOME, "ActiveChanged", GLib.Variant("(b)", (self.active,))
            )
            invocation.return_value(None)

    def close(self):
        self.bus.dbus("ReleaseName", GLib.Variant("(s)", (GNOME,)), "(u)", self.connection)
        self.loop.quit()
        self.thread.join(3)
        assert not self.thread.is_alive()


class Bus:
    def __init__(self):
        self.process = subprocess.Popen(
            ["dbus-daemon", "--session", "--nofork", "--print-address=1"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        assert select.select([self.process.stdout], [], [], 5)[0], "private bus did not start"
        self.address = self.process.stdout.readline().strip()
        self.connections = []
        self.observer = self.connect()
        self.signals = []
        self.observer.signal_subscribe(
            NAME, NAME, "ActiveChanged", PATH, None, Gio.DBusSignalFlags.NONE, self.signal, None
        )
        self.dbus("ListNames", None, "(as)")  # Flush the observer's AddMatch before starting idle.

    def connect(self):
        connection = Gio.DBusConnection.new_for_address_sync(
            self.address,
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None,
            None,
        )
        connection.set_exit_on_close(False)
        self.connections.append(connection)
        return connection

    def signal(self, connection, sender, path, interface, signal, parameters, data):
        assert parameters.get_type_string() == "(b)", parameters
        self.signals.append(parameters.unpack()[0])

    def dbus(self, method, params, result, connection=None):
        return (
            (connection or self.observer)
            .call_sync(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                method,
                params,
                GLib.VariantType.new(result),
                Gio.DBusCallFlags.NONE,
                2000,
                None,
            )
            .unpack()
        )

    def owned(self):
        return self.dbus("NameHasOwner", GLib.Variant("(s)", (NAME,)), "(b)")[0]

    def call(self, method, params=None, result="()"):
        return self.observer.call_sync(
            NAME, PATH, NAME, method, params, GLib.VariantType.new(result), Gio.DBusCallFlags.NONE, 3000, None
        ).unpack()

    def expect(self, expected, label):
        wait_for(lambda: self.signals == expected, f"{label}: expected ActiveChanged {expected}, got {self.signals}")
        settle()
        assert self.signals == expected, (label, self.signals)
        print(f"PASS {label}: {self.signals}", flush=True)

    def close(self):
        for connection in self.connections:
            connection.close_sync(None)
        self.process.terminate()
        self.process.communicate(timeout=3)


def run(args):
    with tempfile.TemporaryDirectory(prefix="idle-active-", dir="/var/tmp") as directory:
        root = Path(directory)
        (root / "bin").mkdir()
        (root / "libexec").mkdir()
        shutil.copy2(args.idle, root / "libexec/gnoblin-idle")
        (root / "bin/gnoblinctl").symlink_to(args.ctl.resolve())
        compositor = Compositor(root / "control.sock")
        bus = Bus()
        env = dict(
            os.environ,
            DBUS_SESSION_BUS_ADDRESS=bus.address,
            DBUS_SYSTEM_BUS_ADDRESS=bus.address,
            GNOBLIN_COMPOSITOR_SOCKET=str(compositor.path),
            XDG_RUNTIME_DIR=str(root),
        )
        log = open(root / "idle.log", "w+")
        process = None
        gnome = None
        try:
            process = subprocess.Popen([root / "libexec/gnoblin-idle"], env=env, stdout=log, stderr=log)
            wait_for(bus.owned, "idle did not own its private D-Bus name")
            assert bus.call("GetActive", result="(b)") == (False,)
            settle()
            assert bus.signals == [], "greeting/subscription/snapshot invented a transition"
            bus.signals.clear()
            compositor.transition("covering")
            assert bus.call("GetActive", result="(b)") == (True,)
            bus.expect([True], "lock transition (GetActive already reports true)")
            for state in ("covering", "locked", "locked", "failsafe", "covering", "locked"):
                compositor.transition(state)
            settle()
            bus.expect([True], "duplicate and active-to-active transitions")
            snapshots = compositor.snapshots
            settle(1.2)
            assert compositor.snapshots == snapshots, "healthy subscription was unnecessarily reconnected"
            print("PASS hello/subscribed/reply handshake retains a stable event connection")
            compositor.transition("unlocked")
            bus.expect([True, False], "unlock")
            assert bus.call("GetActive", result="(b)") == (False,)
            bus.call("Lock")
            bus.call("SetActive", GLib.Variant("(b)", (True,)))
            assert compositor.requests == 2
            bus.expect([True, False], "requests alone do not activate")
            try:
                bus.call("SetActive", GLib.Variant("(b)", (False,)))
                raise AssertionError("SetActive(false) unexpectedly succeeded")
            except GLib.Error as error:
                assert "NotSupported" in str(error), error

            compositor.transition("locked")
            bus.expect([True, False, True], "lock before reconnect")
            snapshots = compositor.snapshots
            compositor.disconnect()
            wait_for(lambda: compositor.snapshots > snapshots, "did not resubscribe and snapshot")
            bus.expect([True, False, True], "reconnect with unchanged active state")
            compositor.close()
            settle()
            bus.expect([True, False, True], "disconnect does not invent unlock")
            compositor = Compositor(root / "control.sock")
            wait_for(lambda: compositor.snapshots > 0, "did not reconnect after compositor restart")
            bus.expect([True, False, True, False], "reconnect reconciles changed snapshot")
            compositor.transition("locked")
            bus.expect([True, False, True, False, True], "events continue after reconnect")

            process.terminate()
            process.wait(timeout=3)
            assert process.returncode == 0, process.returncode
            wait_for(lambda: not bus.owned(), "service kept its name after termination")
            bus.expect([True, False, True, False, True], "shutdown does not invent unlock")
            (root / "gnoblin").mkdir()
            (root / "gnoblin/compositor-v1.sock").symlink_to(compositor.path)
            del env["GNOBLIN_COMPOSITOR_SOCKET"]
            process = subprocess.Popen([root / "libexec/gnoblin-idle"], env=env, stdout=log, stderr=log)
            wait_for(bus.owned, "idle did not restart")
            assert bus.call("GetActive", result="(b)") == (True,)
            settle()
            bus.signals.clear()
            compositor.transition("unlocked")
            bus.expect([False], "restart while locked on the default socket seeds the next unlock")

            bus.signals.clear()
            gnome = Gnome(bus)
            bus.expect([True], "GNOME ownership reconciles its active state")
            assert bus.call("GetActive", result="(b)") == (True,)
            compositor.transition("locked")
            compositor.transition("unlocked")
            bus.expect([True], "native events suppressed while GNOME owns the source")
            bus.call("SetActive", GLib.Variant("(b)", (False,)))
            bus.expect([True, False], "GNOME SetActive and signal delegation")
            bus.call("SetActive", GLib.Variant("(b)", (False,)))
            bus.expect([True, False], "GNOME duplicate signal")
            bus.call("Lock")
            bus.expect([True, False, True], "GNOME Lock delegation")
            gnome.close()
            gnome = None
            bus.expect([True, False, True, False], "GNOME disappearance returns to native state")

            compositor.transition("locked")
            bus.expect([True, False, True, False, True], "native source resumes")
            for invalid in (
                b'{"event":null}\n',
                b"not-json\n",
                b"x" * (64 * 1024 + 1),
                b'{"event":"error","id":"lock-state","message":"compositor is stopping"}\n',
            ):
                snapshots = compositor.snapshots
                compositor.inject(invalid)
                wait_for(lambda: compositor.snapshots > snapshots, "invalid stream did not reconnect")
                bus.expect([True, False, True, False, True], "invalid input does not invent unlock")

            # An event split across socket reads must be emitted once, only after its newline.
            compositor.transition("unlocked", send=False)
            compositor.inject(b'{"event":"gnoblin.session.lock-state-changed",')
            bus.expect([True, False, True, False, True], "partial record is buffered")
            compositor.inject(b'"state":"unlocked"}\n')
            bus.expect([True, False, True, False, True, False], "fragmented unlock record")

            artifact = root / "observer.json"
            observer = subprocess.Popen(
                [
                    "python3",
                    str(Path(__file__).with_name("idle-active-changed-live.py")),
                    "--output",
                    str(artifact),
                    "--timeout",
                    "8",
                ],
                env=env,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            try:
                assert select.select([observer.stdout], [], [], 4)[0], "observer did not start"
                assert observer.stdout.readline().startswith("READY:"), "observer was not ready"
                for state in ("covering", "locked", "locked", "unlocked") * 2:
                    compositor.transition(state)
                    settle(0.03)
                stdout, stderr = observer.communicate(timeout=4)
                assert observer.returncode == 0, (stdout, stderr)
                report = json.loads(artifact.read_text())
                assert report["ok"] and report["signals"] == [True, False, True, False], report
                print("PASS passive VM observer against private fixtures (not a Wayland lock)")
            finally:
                if observer.poll() is None:
                    observer.terminate()
                    observer.communicate(timeout=3)

            compositor.close()
            settle()
            process.terminate()
            process.wait(timeout=3)
            assert process.returncode == 0, process.returncode
            wait_for(lambda: not bus.owned(), "retrying service kept its bus name after shutdown")
            bus.expect([True, False] * 5, "shutdown with reconnect pending")
        finally:
            if gnome:
                gnome.close()
            if process and process.poll() is None:
                process.terminate()
                process.wait(timeout=3)
            log.seek(0)
            output = log.read()
            log.close()
            if output:
                print(output, end="")
            compositor.close()
            bus.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--idle", required=True, type=Path)
    parser.add_argument("--ctl", required=True, type=Path)
    run(parser.parse_args())
