#!/usr/bin/env python3
"""Exercise real capture setup/cleanup against a private Mutter D-Bus stand-in.

No compositor, GUI, or host bus. Pixel delivery is checked separately.
"""

import argparse
import os
from pathlib import Path
import subprocess
import threading
import time
from gi.repository import Gio, GLib

XML = """<node>
<interface name="org.gnome.Mutter.ScreenCast">
<method name="CreateSession"><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
</interface>
<interface name="org.gnome.Mutter.ScreenCast.Session">
<method name="RecordArea"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="a{sv}" direction="in"/><arg type="o" direction="out"/></method>
<method name="Start"/><method name="Stop"/><signal name="Closed"/>
</interface>
<interface name="org.gnome.Mutter.ScreenCast.Stream">
<signal name="PipeWireStreamAdded"><arg type="u"/></signal>
</interface></node>"""
NAME = "org.gnome.Mutter.ScreenCast"
SESSION = "/org/gnome/Mutter/ScreenCast/Session/test"
STREAM = "/org/gnome/Mutter/ScreenCast/Stream/test"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("client", type=Path)
    args = parser.parse_args()
    conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    conn.call_sync(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        GLib.Variant("(su)", (NAME, 4)),
        None,
        Gio.DBusCallFlags.NONE,
        2000,
        None,
    )
    state = {}
    xml = Gio.DBusNodeInfo.new_for_xml(XML)

    def call(bus, sender, path, interface, method, params, invocation):
        state["events"].append(method)
        state["peers"].add(sender)
        if method == "RecordArea":
            x, y, width, height, props = params.unpack()
            assert (x, y, width, height) == (-32, 0, 64, 32)
            assert props.get("cursor-mode") == 0
        if state["case"] == "fail-" + method:
            invocation.return_dbus_error("org.freedesktop.DBus.Error.AccessDenied", "capture inhibited")
            return
        result = (
            GLib.Variant("(o)", (SESSION if method == "CreateSession" else STREAM,))
            if method in ("CreateSession", "RecordArea")
            else GLib.Variant("()", ())
        )
        if state["case"] == "cancel-" + method:
            GLib.timeout_add(250, lambda: invocation.return_value(result) or False)
            return
        invocation.return_value(result)
        if method == "Start" and state["case"] == "closed":
            conn.emit_signal(sender, SESSION, NAME + ".Session", "Closed", None)

    def gone(bus, sender, path, interface, signal, params, data):
        name, old, new = params.unpack()
        if name in state.get("peers", ()) and not new:
            state["gone"].add(name)

    for path, iface in (("/org/gnome/Mutter/ScreenCast", xml.interfaces[0]), (SESSION, xml.interfaces[1])):
        conn.register_object(path, iface, call, None, None)
    conn.signal_subscribe(
        "org.freedesktop.DBus",
        "org.freedesktop.DBus",
        "NameOwnerChanged",
        None,
        None,
        Gio.DBusSignalFlags.NONE,
        gone,
        None,
    )
    loop = GLib.MainLoop()
    thread = threading.Thread(target=loop.run, daemon=True)
    thread.start()
    cases = [
        "fail-CreateSession",
        "fail-RecordArea",
        "fail-Start",
        "closed",
        "timeout",
        "cancel-CreateSession",
        "cancel-RecordArea",
        "cancel-Start",
        "cancel-frame",
    ]
    for case in cases:
        state.clear()
        state.update(case=case, events=[], peers=set(), gone=set())
        before = set(Path(os.environ["TMPDIR"]).glob("gnoblin-screenshot-*"))
        started = time.monotonic()
        result = subprocess.run(
            [str(args.client)] + (["40"] if case.startswith("cancel-") else []),
            capture_output=True,
            text=True,
            timeout=16,
        )
        time.sleep(0.35)
        assert result.returncode == 1 and "ERROR " in result.stdout, (case, result)
        assert len(state["peers"]) == 1 and state["gone"] == state["peers"], (case, state)
        if case not in ("fail-CreateSession", "cancel-CreateSession"):
            assert "Stop" in state["events"], (case, state)
        assert set(Path(os.environ["TMPDIR"]).glob("gnoblin-screenshot-*")) == before
        if case != "timeout":
            assert time.monotonic() - started < 3, (case, state)
        print("PASS", case, state["events"], result.stdout.strip())
    loop.quit()
    thread.join()


if __name__ == "__main__":
    main()
