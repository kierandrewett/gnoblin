#!/usr/bin/env python3
"""Compile the real ScreenSaver method handler and run it with leak detection enabled."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import select
import shlex
import subprocess
import threading
import time

from gi.repository import Gio, GLib

ROOT = Path(__file__).resolve().parents[1]
NAME = "org.freedesktop.ScreenSaver"
GNOME = "org.gnome.ScreenSaver"


def call(connection, name, path, interface, method, params, signature):
    return connection.call_sync(
        name, path, interface, method, params, GLib.VariantType.new(signature), Gio.DBusCallFlags.NONE, 3000, None
    ).unpack()


def run(args):
    args.output.mkdir(mode=0o700, parents=True, exist_ok=False)
    source = args.source.resolve()
    binary = args.output.resolve() / "idle-reply-leak"
    flags = shlex.split(
        subprocess.check_output(["pkg-config", "--cflags", "--libs", "gio-unix-2.0", "json-glib-1.0"], text=True)
    )
    command = [
        args.cc,
        "-g",
        "-O1",
        "-fsanitize=address,undefined",
        "-fno-omit-frame-pointer",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-Wno-unused-parameter",
        f'-DGNOBLIN_IDLE_SOURCE="{source}"',
        str(ROOT / "tests/idle-reply-leak.c"),
        *flags,
        "-o",
        str(binary),
    ]
    subprocess.run(command, check=True)
    report = {
        "source": str(source),
        "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "cycles": args.cycles,
        "command": command,
        "ok": False,
    }
    daemon = subprocess.Popen(
        ["dbus-daemon", "--session", "--nofork", "--print-address=1"],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    connections = []
    process = None
    loop = GLib.MainLoop()
    worker = None
    try:
        assert select.select([daemon.stdout], [], [], 3)[0], "private bus did not start"
        address = daemon.stdout.readline().strip()

        def connect():
            connection = Gio.DBusConnection.new_for_address_sync(
                address,
                Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
                None,
                None,
            )
            connection.set_exit_on_close(False)
            connections.append(connection)
            return connection

        gnome = connect()
        client = connect()
        state = {"active": False, "fail": False, "calls": 0}

        def handle(connection, sender, path, interface, method, params, invocation):
            state["calls"] += 1
            if state["fail"]:
                invocation.return_dbus_error("org.gnoblin.Test.Failed", "delegated failure")
            elif method == "GetActive":
                invocation.return_value(GLib.Variant("(b)", (state["active"],)))
            else:
                state["active"] = params.unpack()[0] if method == "SetActive" else True
                invocation.return_value(GLib.Variant("()", ()))

        node = Gio.DBusNodeInfo.new_for_xml(
            f"<node><interface name='{GNOME}'>"
            "<method name='GetActive'><arg type='b' direction='out'/></method>"
            "<method name='SetActive'><arg type='b' direction='in'/></method>"
            "<method name='Lock'/></interface></node>"
        )
        registration = gnome.register_object("/org/gnome/ScreenSaver", node.interfaces[0], handle)
        assert call(
            gnome,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            GLib.Variant("(su)", (GNOME, 4)),
            "(u)",
        ) == (1,)
        worker = threading.Thread(target=loop.run, daemon=True)
        worker.start()
        # Override inherited sanitizer settings: no exclusions, no suppression files.
        env = dict(
            os.environ,
            DBUS_SESSION_BUS_ADDRESS=address,
            DBUS_SYSTEM_BUS_ADDRESS=address,
            GNOBLIN_COMPOSITOR_SOCKET=str(args.output.resolve() / "no-native-socket"),
            ASAN_OPTIONS="detect_leaks=1:halt_on_error=1",
            LSAN_OPTIONS="exitcode=23:report_objects=1",
            UBSAN_OPTIONS="halt_on_error=1",
            G_DEBUG="fatal-criticals",
        )
        process = subprocess.Popen([binary], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        deadline = time.monotonic() + 4
        while not call(
            client,
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            GLib.Variant("(s)", (NAME,)),
            "(b)",
        )[0]:
            assert process.poll() is None, "fixture exited before owning the bus name"
            assert time.monotonic() < deadline, "fixture did not own its bus name"
            time.sleep(0.01)

        def screensaver(method, params=None, signature="()"):
            return call(client, NAME, "/org/freedesktop/ScreenSaver", NAME, method, params, signature)

        for _ in range(args.cycles):
            assert screensaver("SetActive", GLib.Variant("(b)", (False,))) == ()
            assert screensaver("GetActive", signature="(b)") == (False,)
            assert screensaver("Lock") == ()
            assert screensaver("GetActive", signature="(b)") == (True,)
        state["fail"] = True
        report["errors"] = []
        for method in ("GetActive", "SetActive", "Lock"):
            try:
                screensaver(
                    method,
                    GLib.Variant("(b)", (True,)) if method == "SetActive" else None,
                    "(b)" if method == "GetActive" else "()",
                )
                raise AssertionError(f"{method} did not forward the error")
            except GLib.Error as error:
                assert "org.gnoblin.Test.Failed" in error.message and "delegated failure" in error.message, error
                report["errors"].append(
                    {"method": method, "name": Gio.DBusError.get_remote_error(error), "message": error.message}
                )
        assert state["calls"] == args.cycles * 4 + 3, state
        report["delegated_calls"] = state["calls"]
        report["functional_ok"] = True
        call(client, NAME, "/org/gnoblin/Test", "org.gnoblin.Test", "Quit", None, "()")
        stdout, stderr = process.communicate(timeout=5)
        (args.output / "stdout.log").write_text(stdout)
        (args.output / "sanitizer.log").write_text(stderr)
        report["exit_code"] = process.returncode
        report["ok"] = process.returncode == 0 and stderr == ""
        gnome.unregister_object(registration)
        print(
            f"{'PASS' if report['ok'] else 'FAIL'}: {state['calls']} delegated calls; "
            f"leak-enabled exit {process.returncode}; {args.output / 'sanitizer.log'}"
        )
        return 0 if report["ok"] else 1
    finally:
        if process and process.poll() is None:
            process.kill()
            process.communicate(timeout=3)
        loop.quit()
        if worker:
            worker.join(3)
        for connection in connections:
            connection.close_sync(None)
        daemon.terminate()
        daemon.communicate(timeout=3)
        (args.output / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, default=ROOT / "src/idle.c")
    parser.add_argument("--output", type=Path, required=True, help="new artifact directory")
    parser.add_argument("--cycles", type=int, default=100)
    parser.add_argument("--cc", default="clang")
    args = parser.parse_args()
    if args.cycles < 1:
        parser.error("cycles must be positive")
    raise SystemExit(run(args))
