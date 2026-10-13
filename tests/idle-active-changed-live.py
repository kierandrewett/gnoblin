#!/usr/bin/env python3
"""Passively compare native lock events and ScreenSaver signals in a disposable VM.

Run in the guest's Gnoblin session. After READY, lock and authenticate normally.
This observer never requests a lock/unlock, starts a service, or changes policy.
"""

import argparse
import json
import os
from pathlib import Path
import socket
import sys
import time

from gi.repository import Gio, GLib


NAME = "org.freedesktop.ScreenSaver"
PATH = "/org/freedesktop/ScreenSaver"
EVENT = "gnoblin.session.lock-state-changed"


def observe(args, report):
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

    def call(name, path, interface, method, parameters, signature):
        return bus.call_sync(
            name,
            path,
            interface,
            method,
            parameters,
            GLib.VariantType.new(signature),
            Gio.DBusCallFlags.NONE,
            3000,
            None,
        ).unpack()

    def active():
        return call(NAME, PATH, NAME, "GetActive", None, "(b)")[0]

    def owner(name):
        return call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "GetNameOwner",
            GLib.Variant("(s)", (name,)),
            "(s)",
        )[0]

    gnome_owned = call(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        GLib.Variant("(s)", ("org.gnome.ScreenSaver",)),
        "(b)",
    )[0]
    assert not gnome_owned, "this native-session check requires GNOME ScreenSaver to be absent"
    report["owner"] = owner(NAME)
    report["socket"] = str(args.socket)
    report["signals"] = []
    report["native"] = []
    report["expected"] = []
    report["initial_get_active"] = active()
    assert report["initial_get_active"] is False, "start with the session unlocked"

    def signal(connection, sender, path, interface, name, parameters, data):
        assert parameters.get_type_string() == "(b)", parameters
        report["signals"].append(parameters.unpack()[0])

    subscription = bus.signal_subscribe(NAME, NAME, "ActiveChanged", PATH, None, Gio.DBusSignalFlags.NONE, signal, None)
    owner(NAME)  # Flush AddMatch before announcing readiness.
    try:
        with socket.socket(socket.AF_UNIX) as control:
            control.settimeout(3)
            control.connect(str(args.socket))
            for request in (
                {"op": "events", "api_version": {"major": 1, "minor": 29}, "events": [EVENT]},
                {
                    "op": "api",
                    "api_version": {"major": 1, "minor": 29},
                    "id": "initial",
                    "method": "session.status",
                    "arguments": {},
                },
            ):
                control.sendall((json.dumps(request) + "\n").encode())
            control.setblocking(False)
            pending = b""
            ready = False
            previous = False
            cycle_locked = False
            finished_at = None
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline:
                while GLib.MainContext.default().iteration(False):
                    pass
                try:
                    chunk = control.recv(4096)
                    assert chunk, "compositor disconnected during lock cycle"
                    pending += chunk
                    assert len(pending) < 65536, "oversized compositor record"
                except BlockingIOError:
                    pass
                while b"\n" in pending:
                    line, pending = pending.split(b"\n", 1)
                    record = json.loads(line)
                    event = record.get("event")
                    assert event != "error", record
                    if event == "reply" and record.get("id") == "initial":
                        report["initial_native"] = record["result"]
                        assert record["result"].get("lock_state") == "unlocked", record
                        ready = True
                        print("READY: lock and authenticate normally; observer makes no changes", flush=True)
                    elif event == EVENT:
                        assert ready, "lock changed before READY; repeat from an unlocked session"
                        report["native"].append(record)
                        state = record["state"]
                        if state == "locked":
                            cycle_locked = True
                        current = state != "unlocked"
                        if current != previous:
                            if not current:
                                assert cycle_locked, "cycle ended without a compositor-confirmed locked event"
                                cycle_locked = False
                            report["expected"].append(current)
                            previous = current
                        if len(report["expected"]) >= 2 * args.cycles and not current:
                            finished_at = time.monotonic()
                if finished_at and time.monotonic() - finished_at >= 0.5:
                    assert report["expected"] == [True, False] * args.cycles, report
                    assert report["signals"] == report["expected"], report
                    report["final_get_active"] = active()
                    assert report["final_get_active"] is False
                    assert owner(NAME) == report["owner"], "idle service restarted during the check"
                    return
                time.sleep(0.01)
            raise AssertionError(f"timed out after {args.timeout}s waiting for {args.cycles} lock/unlock cycles")
    finally:
        bus.signal_unsubscribe(subscription)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="new JSON artifact path")
    parser.add_argument(
        "--socket",
        type=Path,
        default=Path(
            os.environ.get(
                "GNOBLIN_COMPOSITOR_SOCKET",
                f"{os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')}/gnoblin/compositor-v1.sock",
            )
        ),
    )
    parser.add_argument("--cycles", type=int, default=2)
    parser.add_argument("--timeout", type=int, default=120)
    args = parser.parse_args()
    if args.cycles < 1 or args.timeout < 1:
        parser.error("cycles and timeout must be positive")
    report = {"ok": False, "cycles": args.cycles}
    with args.output.open("x") as output:
        try:
            observe(args, report)
            report["ok"] = True
        except (AssertionError, GLib.Error, OSError, ValueError) as error:
            report["error"] = str(error)
        finally:
            json.dump(report, output, indent=2)
            output.write("\n")
    print(f"{'PASS' if report['ok'] else 'FAIL'}: {args.output}")
    if not report["ok"]:
        print(report["error"], file=sys.stderr)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
