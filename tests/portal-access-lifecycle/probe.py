#!/usr/bin/env python3
"""Exercise the production Access handler, request export and real GTK windows.

Failure modes defined before the fix: owner Close leaves a window; foreign
Close is accepted; caller/frontend loss leaves windows; public request removal
on frontend timeout leaves a window; response races complete twice; normal
responses leak the invisible parent; live requests expire without cancellation.
"""

import argparse
import os
from pathlib import Path
import subprocess
import threading
import time
from gi.repository import Gio, GLib

NAME = "org.freedesktop.impl.portal.desktop.gnoblin"
FRONTEND = "org.freedesktop.portal.Desktop"
PATH = "/org/freedesktop/portal/desktop"
IFACE = "org.freedesktop.impl.portal.Access"
REQUEST = "org.freedesktop.impl.portal.Request"
FLAGS = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION
PUBLIC_XML = "<node><interface name='org.freedesktop.portal.Request'><method name='Close'/></interface></node>"


def connect():
    return Gio.DBusConnection.new_for_address_sync(os.environ["DBUS_SESSION_BUS_ADDRESS"], FLAGS, None, None)


def call(bus, name, path, interface, method, params=None, timeout=2000):
    return bus.call_sync(name, path, interface, method, params, None, Gio.DBusCallFlags.NONE, timeout, None)


def name(bus, method):
    params = GLib.Variant("(su)", (FRONTEND, 4)) if method == "RequestName" else GLib.Variant("(s)", (FRONTEND,))
    call(bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus", method, params)


def stats(bus):
    return call(bus, NAME, "/org/gnoblin/AccessTest", "org.gnoblin.AccessTest", "Stats").unpack()


def wait_for(predicate, seconds=3):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if predicate():
            return True
        time.sleep(0.02)
    return False


def probe(work, case, baseline):
    frontend, caller, other = connect(), connect(), connect()
    name(frontend, "RequestName")
    handle = PATH + "/request/" + caller.get_unique_name()[1:].replace(".", "_") + "/" + case.replace("-", "_")
    info = Gio.DBusNodeInfo.new_for_xml(PUBLIC_XML)
    exported = frontend.register_object(handle, info.interfaces[0], lambda *args: None, None, None)
    checking = threading.Event()
    if case == "close-pending-check":
        introspection = Gio.DBusNodeInfo.new_for_xml(
            "<node><interface name='org.freedesktop.DBus.Introspectable'>"
            "<method name='Introspect'><arg type='s' direction='out'/></method></interface></node>"
        )

        def delayed_check(bus, sender, path, interface, method, params, invocation):
            checking.set()
            GLib.timeout_add(250, lambda: invocation.return_value(GLib.Variant("(s)", (PUBLIC_XML,))) or False)

        frontend.register_object(handle, introspection.interfaces[0], delayed_check, None, None)
    result = []
    done = threading.Event()
    errors = []

    def ready(bus, reply, data):
        try:
            result.append(bus.call_finish(reply).unpack())
        except GLib.Error as error:
            result.append(str(error))
        done.set()

    with (work / (case + ".log")).open("w+") as log:
        backend = subprocess.Popen([str(work / "backend")], stdout=log, stderr=log)
        try:
            assert wait_for(
                lambda: call(
                    other,
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "NameHasOwner",
                    GLib.Variant("(s)", (NAME,)),
                ).unpack()[0]
            ), "backend not ready"
            frontend.call(
                NAME,
                PATH,
                IFACE,
                "AccessDialog",
                GLib.Variant("(osssssa{sv})", (handle, "", "", "Access test", "Request lifecycle", "", {})),
                None,
                Gio.DBusCallFlags.NONE,
                150 if case == "timeout" else 60000,
                None,
                ready,
                None,
            )
            assert wait_for(lambda: stats(other)[0] == 1), "dialog not mapped"
            print(case, "before", stats(other), flush=True)
            if case == "foreign-close":
                try:
                    call(other, NAME, handle, REQUEST, "Close")
                    errors.append("foreign Close accepted")
                except GLib.Error as error:
                    assert "AccessDenied" in str(error), error
                if stats(other)[0] != 1:
                    errors.append("foreign Close destroyed dialog")
                try:
                    call(frontend, NAME, handle, REQUEST, "Close")
                except GLib.Error as error:
                    errors.append("owner cannot Close after foreign Close: " + str(error))
            elif case == "close-pending-check":
                assert checking.wait(3), "public request check did not start"
                call(frontend, NAME, handle, REQUEST, "Close")
            elif case == "loss-race":
                caller.close_sync(None)
                name(frontend, "ReleaseName")
            elif case == "owner-close":
                call(frontend, NAME, handle, REQUEST, "Close")
            elif case == "caller-disconnect":
                caller.close_sync(None)
            elif case == "frontend-disconnect":
                frontend.close_sync(None)
            elif case == "frontend-name-loss":
                name(frontend, "ReleaseName")
            elif case == "timeout":
                assert done.wait(2) and isinstance(result[0], str), result
                # Standard Screenshot timeout calls send_response(), which unexports
                # the public Request. It does not invoke backend Request.Close.
                frontend.unregister_object(exported)
                exported = 0
            elif case == "public-unexport":
                frontend.unregister_object(exported)
                exported = 0
            elif case == "live-request":
                time.sleep(26)
                assert stats(other)[0] == 1 and not result, (stats(other), result)
                call(
                    other,
                    NAME,
                    "/org/gnoblin/AccessTest",
                    "org.gnoblin.AccessTest",
                    "Respond",
                    GLib.Variant("(s)", ("deny",)),
                )
            else:
                response = "grant" if case == "grant" else "close" if case == "dialog-close" else "deny"
                call(
                    other,
                    NAME,
                    "/org/gnoblin/AccessTest",
                    "org.gnoblin.AccessTest",
                    "Respond",
                    GLib.Variant("(s)", (response,)),
                )
            if not wait_for(lambda: stats(other) == (0, 0), 0.3 if baseline else 3):
                errors.append("retained GTK windows " + repr(stats(other)))
            if case != "frontend-disconnect" and not done.wait(0.1 if baseline else 2):
                errors.append("Access call still pending")
            if len(result) > 1:
                errors.append("multiple completions")
            try:
                call(other, NAME, handle, REQUEST, "Close")
                errors.append("backend Request still exported")
            except GLib.Error as error:
                if not ("UnknownMethod" in str(error) or "UnknownObject" in str(error)):
                    errors.append(str(error))
            print(case, "after", stats(other), "result", result, "errors", errors, flush=True)
        finally:
            backend.terminate()
            backend.wait(timeout=3)
            for bus in (frontend, caller, other):
                if not bus.is_closed():
                    bus.close_sync(None)
            log.seek(0)
            output = log.read()
            if any(word in output for word in ("CRITICAL", "AddressSanitizer", "runtime error:")):
                errors.append(output)
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path)
    parser.add_argument("--baseline", action="store_true")
    args = parser.parse_args()
    loop = GLib.MainLoop()
    thread = threading.Thread(target=loop.run, daemon=True)
    thread.start()
    with (args.work / "broadway.log").open("w") as log:
        display = subprocess.Popen(
            ["gtk4-broadwayd", "--unixsocket=" + str(args.work / "runtime/http.sock"), ":37"], stdout=log, stderr=log
        )
        try:
            assert wait_for(lambda: (args.work / "runtime/broadway38.socket").exists()), list(
                (args.work / "runtime").iterdir()
            )
            failed = []
            for case in (
                "owner-close",
                "close-pending-check",
                "loss-race",
                "foreign-close",
                "caller-disconnect",
                "frontend-disconnect",
                "frontend-name-loss",
                "timeout",
                "public-unexport",
                "grant",
                "deny",
                "dialog-close",
                "live-request",
            ):
                if args.baseline and case in ("live-request", "close-pending-check"):
                    continue
                errors = probe(args.work, case, args.baseline)
                if errors:
                    failed.append((case, errors))
            print("FAILURES", failed, flush=True)
        finally:
            display.terminate()
            display.wait(timeout=3)
            loop.quit()
            thread.join()
    raise SystemExit(bool(failed))


if __name__ == "__main__":
    main()
