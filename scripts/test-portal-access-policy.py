#!/usr/bin/env python3
"""Prove GNOME Shell Access requests require an explicit dialog response."""

import concurrent.futures
import sys
import time

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

ALLOWED_NAME = "org.gnome.RemoteDesktop.Handover"
SHELL_NAME = "org.gnome.Shell"
ACCESS_PATH = "/org/freedesktop/portal/desktop"
REQUEST_PATH = "/org/freedesktop/portal/desktop/request/gnoblin_policy"


def call_access(connection: Gio.DBusConnection) -> GLib.Variant:
    parameters = GLib.Variant(
        "(osssssa{sv})",
        (REQUEST_PATH, "", "", "Policy probe", "", "Policy probe", {}),
    )
    return connection.call_sync(
        SHELL_NAME,
        ACCESS_PATH,
        "org.freedesktop.impl.portal.Access",
        "AccessDialog",
        parameters,
        GLib.VariantType.new("(ua{sv})"),
        Gio.DBusCallFlags.NONE,
        10_000,
        None,
    )


def close_request(connection: Gio.DBusConnection) -> tuple[bool, str | None]:
    try:
        connection.call_sync(
            SHELL_NAME,
            REQUEST_PATH,
            "org.freedesktop.impl.portal.Request",
            "Close",
            None,
            None,
            Gio.DBusCallFlags.NONE,
            250,
            None,
        )
        return True, None
    except GLib.Error as error:
        return False, error.message


def main() -> int:

    connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    request_name = connection.call_sync(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        GLib.Variant("(su)", (ALLOWED_NAME, 0)),
        GLib.VariantType.new("(u)"),
        Gio.DBusCallFlags.NONE,
        5_000,
        None,
    )
    if request_name.unpack()[0] != 1:
        print(f"FAIL: could not own allowed portal sender name {ALLOWED_NAME}", file=sys.stderr)
        return 1

    with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
        response_future = executor.submit(call_access, connection)
        # An automatic policy response must not win this observation window.
        time.sleep(0.25)

        deadline = time.monotonic() + 5
        closed = False
        last_close_error = "request object was not available"
        while time.monotonic() < deadline and not response_future.done():
            closed, last_close_error = close_request(connection)
            if closed:
                break
            time.sleep(0.05)

        try:
            response = response_future.result(timeout=12).unpack()[0]
        except (concurrent.futures.TimeoutError, GLib.Error) as error:
            detail = str(error) or last_close_error
            print(f"FAIL: Access dialog did not complete cleanly: {detail}", file=sys.stderr)
            return 1

    if response == 0:
        print("FAIL: Access request was approved without an explicit response", file=sys.stderr)
        return 1
    if not closed or response != 2:
        print(f"FAIL: expected a caller-closed response (2), got {response}", file=sys.stderr)
        return 1

    print("PASS: portal Access request required an explicit response")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
