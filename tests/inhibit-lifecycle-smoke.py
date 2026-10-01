#!/usr/bin/env python3
"""Exercise the installed Gnoblin portal backend's end-session monitor flow.

Run twice with GNOBLIN_TEST_PREFIX pointing at a private install prefix:
  TEST_ACK=1 python3 tests/inhibit-lifecycle-smoke.py
  TEST_ACK=0 python3 tests/inhibit-lifecycle-smoke.py
Requires PyGObject and a session bus (normally provided by dbus-run-session).
"""

import os
import subprocess
import threading
import time
from gi.repository import Gio, GLib

prefix = os.environ["GNOBLIN_TEST_PREFIX"]
env = os.environ.copy()
env["XDG_DATA_DIRS"] = f"{prefix}/share:/usr/local/share:/usr/share"
env["GSETTINGS_SCHEMA_DIR"] = f"{prefix}/share/glib-2.0/schemas"
env["LD_LIBRARY_PATH"] = f"{prefix}/lib64:{prefix}/lib64/mutter-51:/usr/lib64"
env["PATH"] = f"{prefix}/bin:{env['PATH']}"
address = env["DBUS_SESSION_BUS_ADDRESS"]
flags = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION


def connection():
    return Gio.DBusConnection.new_for_address_sync(address, flags, None, None)


def request_name(conn, name):
    reply = conn.call_sync(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        GLib.Variant("(su)", (name, 4)),
        GLib.VariantType.new("(u)"),
        Gio.DBusCallFlags.NONE,
        2000,
        None,
    )
    result = reply.unpack()[0]
    if result != 1:
        raise RuntimeError(f"could not own {name}: {result}")


service = connection()
request_name(service, "org.freedesktop.ScreenSaver")
xml = "<node><interface name='org.freedesktop.ScreenSaver'><method name='GetActive'><arg type='b' direction='out'/></method></interface></node>"
node = Gio.DBusNodeInfo.new_for_xml(xml)


def service_call(conn, sender, path, iface, method, params, invocation):
    if method == "GetActive":
        invocation.return_value(GLib.Variant("(b)", (False,)))
    else:
        invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", method)


service.register_object("/org/freedesktop/ScreenSaver", node.interfaces[0], service_call, None, None)
service_loop = GLib.MainLoop()
threading.Thread(target=service_loop.run, daemon=True).start()

owner = connection()
request_name(owner, "org.gnoblin.SessionSupervisor")
attacker = connection()
observer = connection()
states = []


def state_changed(conn, sender, path, iface, signal, params, data):
    session_handle, state = params.unpack()
    value = state.get("session-state")
    states.append((session_handle, value))
    if value == 2 and os.environ.get("TEST_ACK") == "1":
        conn.call_sync(
            "org.freedesktop.impl.portal.desktop.gnoblin",
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.impl.portal.Inhibit",
            "QueryEndResponse",
            GLib.Variant("(o)", (session_handle,)),
            GLib.VariantType.new("()"),
            Gio.DBusCallFlags.NONE,
            1000,
            None,
        )


observer.signal_subscribe(
    "org.freedesktop.impl.portal.desktop.gnoblin",
    "org.freedesktop.impl.portal.Inhibit",
    "StateChanged",
    "/org/freedesktop/portal/desktop",
    None,
    Gio.DBusSignalFlags.NONE,
    state_changed,
    None,
)
observer_loop = GLib.MainLoop()
threading.Thread(target=observer_loop.run, daemon=True).start()
backend = subprocess.Popen(
    [f"{prefix}/libexec/xdg-desktop-portal-gnoblin"], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE
)
try:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        owned = owner.call_sync(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            GLib.Variant("(s)", ("org.freedesktop.impl.portal.desktop.gnoblin",)),
            GLib.VariantType.new("(b)"),
            Gio.DBusCallFlags.NONE,
            500,
            None,
        ).unpack()[0]
        if owned:
            break
        time.sleep(0.05)
    else:
        raise RuntimeError("portal backend did not acquire its D-Bus name")
    result = owner.call_sync(
        "org.freedesktop.impl.portal.desktop.gnoblin",
        "/org/freedesktop/portal/desktop",
        "org.freedesktop.impl.portal.Inhibit",
        "CreateMonitor",
        GLib.Variant(
            "(ooss)",
            (
                "/org/freedesktop/portal/desktop/request/test/monitor",
                "/org/freedesktop/portal/desktop/session/test/monitor",
                "org.test.App",
                "",
            ),
        ),
        GLib.VariantType.new("(u)"),
        Gio.DBusCallFlags.NONE,
        3000,
        None,
    ).unpack()[0]
    if result != 0:
        raise RuntimeError(f"monitor creation returned {result}")
    try:
        attacker.call_sync(
            "org.freedesktop.impl.portal.desktop.gnoblin",
            "/org/freedesktop/portal/desktop",
            "org.gnoblin.Portal.InhibitLifecycle",
            "PrepareForEnd",
            None,
            GLib.VariantType.new("()"),
            Gio.DBusCallFlags.NONE,
            3000,
            None,
        )
        raise RuntimeError("unowned sender was allowed to start end-session")
    except GLib.Error as error:
        if "AccessDenied" not in error.message and "access denied" not in error.message.lower():
            raise
    started = time.monotonic()
    owner.call_sync(
        "org.freedesktop.impl.portal.desktop.gnoblin",
        "/org/freedesktop/portal/desktop",
        "org.gnoblin.Portal.InhibitLifecycle",
        "PrepareForEnd",
        None,
        GLib.VariantType.new("()"),
        Gio.DBusCallFlags.NONE,
        3000,
        None,
    )
    elapsed = time.monotonic() - started
    deadline = time.monotonic() + 1
    while len(states) < 3 and time.monotonic() < deadline:
        time.sleep(0.01)
    observed = [state for session, state in states if session.endswith("/session/test/monitor")]
    if observed[:3] != [1, 2, 3]:
        raise RuntimeError(f"expected monitor states [1, 2, 3], got {observed}; all={states}")
    if os.environ.get("TEST_ACK") == "1" and elapsed >= 0.8:
        raise RuntimeError(f"acknowledged Query End took too long: {elapsed:.3f}s")
    if os.environ.get("TEST_ACK") != "1" and elapsed < 0.8:
        raise RuntimeError(f"unacknowledged Query End did not wait for timeout: {elapsed:.3f}s")
    print(f"PASS: monitor observed session-state {observed[:3]}; unowned request denied; wait={elapsed:.3f}s")
finally:
    backend.terminate()
    try:
        backend.wait(timeout=3)
    except subprocess.TimeoutExpired:
        backend.kill()
        backend.wait()
    service_loop.quit()
    observer_loop.quit()
    if backend.returncode not in (0, -15):
        stderr = backend.stderr.read().decode(errors="replace") if backend.stderr else ""
        print(stderr)
        raise RuntimeError(f"portal backend exited {backend.returncode}")
