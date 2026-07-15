#!/usr/bin/env python3
"""Prove GNOME Shell scopes privileged APIs and fork policy by session mode."""

import concurrent.futures
import os
import sys
import time
from typing import cast

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

ALLOWED_NAME = "org.gnome.RemoteDesktop.Handover"
SHELL_NAME = "org.gnome.Shell"
SHELL_PATH = "/org/gnome/Shell"
EXTENSIONS_INTERFACE = "org.gnome.Shell.Extensions"
COMPATIBLE_EXTENSION = "scope-compatible@gnoblin"
OUTDATED_EXTENSION = "scope-outdated@gnoblin"
NOTIFICATIONS_SERVICE = "org.gnome.Shell.Notifications"
NOTIFICATIONS_NAME = "org.freedesktop.Notifications"
ACCESS_PATH = "/org/freedesktop/portal/desktop"
REQUEST_PATH = "/org/freedesktop/portal/desktop/request/gnoblin_policy"


def eval_policy_matches(connection: Gio.DBusConnection) -> bool:
    expectation = os.environ.get("GNOBLIN_EXPECT_UNSAFE_MODE", "0")
    if expectation not in {"0", "1"}:
        print(f"FAIL: invalid GNOBLIN_EXPECT_UNSAFE_MODE value: {expectation}", file=sys.stderr)
        return False

    reply = connection.call_sync(
        SHELL_NAME,
        SHELL_PATH,
        "org.gnome.Shell",
        "Eval",
        GLib.Variant("(s)", ("40 + 2",)),
        GLib.VariantType.new("(bs)"),
        Gio.DBusCallFlags.NONE,
        5_000,
        None,
    )
    success, _result = reply.unpack()
    if expectation == "1" and success:
        print("PASS: org.gnome.Shell.Eval was explicitly enabled")
        return True
    if expectation == "0" and not success:
        print("PASS: org.gnome.Shell.Eval remained restricted")
        return True

    state = "executed" if success else "was denied"
    print(f"FAIL: org.gnome.Shell.Eval {state} against the explicit policy", file=sys.stderr)
    return False


def call_extension(
    connection: Gio.DBusConnection,
    method: str,
    uuid: str,
    result_signature: str,
) -> object:
    reply = connection.call_sync(
        SHELL_NAME,
        SHELL_PATH,
        EXTENSIONS_INTERFACE,
        method,
        GLib.Variant("(s)", (uuid,)),
        GLib.VariantType.new(result_signature),
        Gio.DBusCallFlags.NONE,
        5_000,
        None,
    )
    return reply.unpack()[0]


def extension_info(connection: Gio.DBusConnection, uuid: str) -> dict[str, object]:
    return cast(
        dict[str, object],
        call_extension(connection, "GetExtensionInfo", uuid, "(a{sv})"),
    )


def enable_extension(connection: Gio.DBusConnection, uuid: str) -> bool:
    return cast(bool, call_extension(connection, "EnableExtension", uuid, "(b)"))


def wait_extension_state(
    connection: Gio.DBusConnection,
    uuid: str,
    expected_state: int,
) -> dict[str, object]:
    deadline = time.monotonic() + 5
    info: dict[str, object] = {}
    while time.monotonic() < deadline:
        info = extension_info(connection, uuid)
        if int(info.get("state", 0)) == expected_state:
            return info
        time.sleep(0.05)
    return info


def extension_scope_matches(connection: Gio.DBusConnection) -> bool:
    mode = os.environ.get("GNOBLIN_ACTIVE_MODE")
    if mode not in {"gnoblin", "user"}:
        print(f"FAIL: invalid GNOBLIN_ACTIVE_MODE value: {mode}", file=sys.stderr)
        return False

    compatible_enabled = enable_extension(connection, COMPATIBLE_EXTENSION)
    compatible = wait_extension_state(connection, COMPATIBLE_EXTENSION, 1)
    if not compatible_enabled or int(compatible.get("state", 0)) != 1:
        print(
            f"FAIL: panel scope probe failed in {mode} mode: {compatible}",
            file=sys.stderr,
        )
        return False

    outdated_enabled = enable_extension(connection, OUTDATED_EXTENSION)
    expected_state = 1 if mode == "gnoblin" else 4
    outdated = wait_extension_state(connection, OUTDATED_EXTENSION, expected_state)
    outdated_state = int(outdated.get("state", 0))
    if mode == "gnoblin" and outdated_enabled and outdated_state == 1:
        print("PASS: Gnoblin mode accepted an extension with incompatible metadata")
        return True
    if mode == "user" and outdated_state == 4:
        print("PASS: stock mode retained extension version validation")
        return True

    print(
        f"FAIL: extension validation leaked across {mode} mode: {outdated}",
        file=sys.stderr,
    )
    return False


def notification_owner_matches(connection: Gio.DBusConnection) -> bool:
    connection.call_sync(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "StartServiceByName",
        GLib.Variant("(su)", (NOTIFICATIONS_SERVICE, 0)),
        GLib.VariantType.new("(u)"),
        Gio.DBusCallFlags.NONE,
        5_000,
        None,
    )

    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        reply = connection.call_sync(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            GLib.Variant("(s)", (NOTIFICATIONS_NAME,)),
            GLib.VariantType.new("(b)"),
            Gio.DBusCallFlags.NONE,
            5_000,
            None,
        )
        if reply.unpack()[0]:
            print("PASS: stock mode kept org.freedesktop.Notifications owned")
            return True
        time.sleep(0.05)

    print(
        "FAIL: stock mode released org.freedesktop.Notifications for a Gnoblin-only setting",
        file=sys.stderr,
    )
    return False


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
    if not eval_policy_matches(connection):
        return 1
    if os.environ.get("GNOBLIN_EXPECT_EXTENSION_SCOPE") == "1":
        if not extension_scope_matches(connection):
            return 1
    if os.environ.get("GNOBLIN_EXPECT_NOTIFICATION_OWNER") == "1":
        if not notification_owner_matches(connection):
            return 1

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
