#!/usr/bin/env python3
"""Actual 1.20.4 public Screenshot -> generic Access timeout, twice.

Only the final pixel provider is a stub. Consent is a real GTK dialog. All
processes run under the runner's private bus, display, and XDG directories.
"""

import argparse
import os
from pathlib import Path
import subprocess
import threading
import time
from gi.repository import Gio, GLib
from probe import call, connect, stats, wait_for, FRONTEND, PATH, NAME


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("work", type=Path)
    parser.add_argument("--baseline", action="store_true")
    parser.add_argument("--decisions-only", action="store_true")
    args = parser.parse_args()
    work = args.work
    portals = work / "portals"
    portals.mkdir()
    (portals / "gnoblin.portal").write_text(
        "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.gnoblin\nInterfaces=org.freedesktop.impl.portal.Screenshot;org.freedesktop.impl.portal.Access;\nUseIn=gnoblin\n"
    )
    (portals / "portals.conf").write_text(
        "[preferred]\ndefault=none\norg.freedesktop.impl.portal.Screenshot=gnoblin\norg.freedesktop.impl.portal.Access=gnoblin\n"
    )
    env = dict(os.environ, XDG_DESKTOP_PORTAL_DIR=str(portals), XDG_CURRENT_DESKTOP="gnoblin")
    bus = connect()
    loop = GLib.MainLoop()
    thread = threading.Thread(target=loop.run, daemon=True)
    thread.start()
    processes = []
    errors = []
    with (work / "processes.log").open("w+") as log:
        try:
            for command, owner in (
                (["gtk4-broadwayd", "--unixsocket=" + str(work / "runtime/http.sock"), ":37"], None),
                ([str(work / "backend")], "org.freedesktop.impl.portal.desktop.gnoblin"),
                (["/usr/libexec/xdg-permission-store"], "org.freedesktop.impl.portal.PermissionStore"),
                (["/usr/libexec/xdg-desktop-portal", "--verbose"], FRONTEND),
            ):
                processes.append(subprocess.Popen(command, stdout=log, stderr=log, env=env))
                if owner:
                    assert wait_for(
                        lambda: call(
                            bus,
                            "org.freedesktop.DBus",
                            "/org/freedesktop/DBus",
                            "org.freedesktop.DBus",
                            "NameHasOwner",
                            GLib.Variant("(s)", (owner,)),
                        ).unpack()[0],
                        5,
                    ), owner
                else:
                    assert wait_for(lambda: (work / "runtime/broadway38.socket").exists())
            for attempt in range(0 if args.decisions_only else 2):
                client = connect()
                handle = PATH + "/request/" + client.get_unique_name()[1:].replace(".", "_") + "/timeout" + str(attempt)
                replies = []
                event = threading.Event()

                def response(bus, sender, path, interface, signal, params, data):
                    replies.append(params.unpack())
                    event.set()

                subscription = client.signal_subscribe(
                    FRONTEND,
                    "org.freedesktop.portal.Request",
                    "Response",
                    handle,
                    None,
                    Gio.DBusSignalFlags.NONE,
                    response,
                    None,
                )
                started = time.monotonic()
                returned = call(
                    client,
                    FRONTEND,
                    PATH,
                    "org.freedesktop.portal.Screenshot",
                    "Screenshot",
                    GLib.Variant("(sa{sv})", ("", {"handle_token": GLib.Variant("s", "timeout" + str(attempt))})),
                ).unpack()[0]
                assert returned == handle
                assert wait_for(lambda: stats(bus)[0] == attempt + 1 if args.baseline else stats(bus)[0] == 1)
                print("public request", attempt, "mapped", stats(bus), flush=True)
                assert event.wait(30), "no public Response"
                elapsed = time.monotonic() - started
                assert replies == [(2, {})], replies
                assert 24 < elapsed < 30, elapsed
                if not wait_for(lambda: stats(bus) == (0, 0), 0.3 if args.baseline else 3):
                    errors.append((attempt, "retained GTK windows", stats(bus)))
                print(
                    "public request",
                    attempt,
                    "Response2 after",
                    round(elapsed, 2),
                    "seconds; windows",
                    stats(bus),
                    flush=True,
                )
                # Keep the original requester alive until cleanup is checked.
                client.signal_unsubscribe(subscription)
                client.close_sync(None)
            if not args.baseline or args.decisions_only:
                for action in ("caller-exit", "close", "deny"):
                    client = connect()
                    handle = PATH + "/request/" + client.get_unique_name()[1:].replace(".", "_") + "/decision"
                    replies = []
                    event = threading.Event()

                    def decision_response(bus, sender, path, interface, signal, params, data):
                        replies.append(params.unpack())
                        event.set()

                    client.signal_subscribe(
                        FRONTEND,
                        "org.freedesktop.portal.Request",
                        "Response",
                        handle,
                        None,
                        Gio.DBusSignalFlags.NONE,
                        decision_response,
                        None,
                    )
                    call(
                        client,
                        FRONTEND,
                        PATH,
                        "org.freedesktop.portal.Screenshot",
                        "Screenshot",
                        GLib.Variant("(sa{sv})", ("", {"handle_token": GLib.Variant("s", "decision")})),
                    )
                    assert wait_for(lambda: stats(bus)[0] == 1)
                    if action == "caller-exit":
                        client.close_sync(None)
                    else:
                        call(
                            bus,
                            NAME,
                            "/org/gnoblin/AccessTest",
                            "org.gnoblin.AccessTest",
                            "Respond",
                            GLib.Variant("(s)", (action,)),
                        )
                        assert event.wait(3) and replies == [(2, {})], (action, replies)
                        client.close_sync(None)
                    assert wait_for(lambda: stats(bus) == (0, 0)), stats(bus)
                    # Cancellation is not a user's decision to deny this app.
                    time.sleep(0.3)
                    try:
                        permissions, data = call(
                            bus,
                            "org.freedesktop.impl.portal.PermissionStore",
                            "/org/freedesktop/impl/portal/PermissionStore",
                            "org.freedesktop.impl.portal.PermissionStore",
                            "Lookup",
                            GLib.Variant("(ss)", ("screenshot", "screenshot")),
                        ).unpack()
                    except GLib.Error as error:
                        assert "NotFound" in str(error), error
                        permissions = {}
                    if action == "deny":
                        assert len(permissions) == 1 and list(permissions.values()) == [["no"]], permissions
                    else:
                        assert not permissions, permissions
                    print(action, "zero windows; stored decisions", list(permissions.values()), flush=True)
            print("FAILURES", errors, flush=True)
        finally:
            for process in reversed(processes):
                process.terminate()
                process.wait(timeout=3)
            loop.quit()
            thread.join()
            log.seek(0)
            output = log.read()
            if any(
                word in output for word in ("CRITICAL", "AddressSanitizer", "runtime error:", "UNEXPECTED SCREENSHOT")
            ):
                raise AssertionError(output)
    raise SystemExit(bool(errors))


if __name__ == "__main__":
    main()
