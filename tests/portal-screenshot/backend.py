#!/usr/bin/env python3
"""Test production backend request lifecycle on a private bus with fake pixels."""

import os
from pathlib import Path
import subprocess
import sys
import threading
import time
import urllib.parse
from gi.repository import Gio, GLib

NAME = "org.freedesktop.impl.portal.desktop.gnoblin"
PATH = "/org/freedesktop/portal/desktop"
IFACE = "org.freedesktop.impl.portal.Screenshot"
FLAGS = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION


def main():
    conn = Gio.DBusConnection.new_for_address_sync(os.environ["DBUS_SESSION_BUS_ADDRESS"], FLAGS, None, None)
    other = Gio.DBusConnection.new_for_address_sync(os.environ["DBUS_SESSION_BUS_ADDRESS"], FLAGS, None, None)
    conn.call_sync(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        GLib.Variant("(su)", ("org.freedesktop.portal.Desktop", 4)),
        None,
        Gio.DBusCallFlags.NONE,
        2000,
        None,
    )
    with subprocess.Popen([sys.argv[1]], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) as backend:
        assert backend.stdout.readline().strip() == "READY"
        logs = []
        log_thread = threading.Thread(target=lambda: logs.extend(backend.stdout), daemon=True)
        log_thread.start()
        loop = GLib.MainLoop()
        thread = threading.Thread(target=loop.run, daemon=True)
        thread.start()
        try:
            for i, case in enumerate(("success", "close", "wrong-close", "disconnect", "pick-color")):
                handle = PATH + "/request/" + conn.get_unique_name()[1:].replace(".", "_") + "/test" + str(i)
                result = []
                event = threading.Event()
                log_start = len(logs)

                def done(bus, reply, data):
                    try:
                        result.append(bus.call_finish(reply).unpack())
                    except GLib.Error as error:
                        result.append(error)
                    event.set()

                params = GLib.Variant("(ossa{sv})", (handle, "", "", {}))
                if case == "disconnect":
                    # Losing the frontend name invalidates all of its pending requests.
                    conn.call(
                        NAME, PATH, IFACE, "Screenshot", params, None, Gio.DBusCallFlags.NONE, 3000, None, done, None
                    )
                    time.sleep(0.05)
                    conn.call_sync(
                        "org.freedesktop.DBus",
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "ReleaseName",
                        GLib.Variant("(s)", ("org.freedesktop.portal.Desktop",)),
                        None,
                        Gio.DBusCallFlags.NONE,
                        1000,
                        None,
                    )
                else:
                    conn.call(
                        NAME,
                        PATH,
                        IFACE,
                        "PickColor" if case == "pick-color" else "Screenshot",
                        params,
                        None,
                        Gio.DBusCallFlags.NONE,
                        3000,
                        None,
                        done,
                        None,
                    )
                if case in ("close", "wrong-close"):
                    time.sleep(0.05)
                    if case == "wrong-close":
                        try:
                            other.call_sync(
                                NAME,
                                handle,
                                "org.freedesktop.impl.portal.Request",
                                "Close",
                                None,
                                None,
                                Gio.DBusCallFlags.NONE,
                                1000,
                                None,
                            )
                            raise AssertionError("foreign Close accepted")
                        except GLib.Error as error:
                            assert "AccessDenied" in str(error)
                    else:
                        conn.call_sync(
                            NAME,
                            handle,
                            "org.freedesktop.impl.portal.Request",
                            "Close",
                            None,
                            None,
                            Gio.DBusCallFlags.NONE,
                            1000,
                            None,
                        )
                assert event.wait(4), case
                assert len(result) == 1 and not isinstance(result[0], Exception), (case, result)
                code, values = result[0]
                if case in ("success", "wrong-close"):
                    assert code == 0 and values.get("uri", "").startswith("file://"), (case, result)
                    file = Path(urllib.parse.unquote(urllib.parse.urlparse(values["uri"]).path))
                    assert file.read_bytes()[:8] == b"\x89PNG\r\n\x1a\n"
                    file.unlink()
                else:
                    assert code != 0 and "uri" not in values, (case, result)
                time.sleep(0.3)
                if case == "pick-color":
                    assert not any(line.strip() == "CAPTURE" for line in logs[log_start:]), (case, logs)
                assert not list(Path(os.environ["TMPDIR"]).glob("gnoblin-screenshot-*")), case
                # A completed request must no longer accept Close.
                try:
                    conn.call_sync(
                        NAME,
                        handle,
                        "org.freedesktop.impl.portal.Request",
                        "Close",
                        None,
                        None,
                        Gio.DBusCallFlags.NONE,
                        1000,
                        None,
                    )
                    raise AssertionError("request leaked")
                except GLib.Error as error:
                    assert "UnknownMethod" in str(error) or "UnknownObject" in str(error), error
                print("PASS", case, code)
                if case == "disconnect":
                    conn.call_sync(
                        "org.freedesktop.DBus",
                        "/org/freedesktop/DBus",
                        "org.freedesktop.DBus",
                        "RequestName",
                        GLib.Variant("(su)", ("org.freedesktop.portal.Desktop", 4)),
                        None,
                        Gio.DBusCallFlags.NONE,
                        1000,
                        None,
                    )
        finally:
            backend.terminate()
            backend.wait(timeout=3)
            loop.quit()
            thread.join()
            log_thread.join()
            errors = backend.stderr.read()
            assert "CRITICAL" not in errors and "AddressSanitizer" not in errors, errors


if __name__ == "__main__":
    main()
