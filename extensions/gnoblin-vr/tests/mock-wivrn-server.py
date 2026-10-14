#!/usr/bin/env python3
"""Stands in for wivrn-server's D-Bus interface so the headset-detection path can be exercised
without a headset. Usage: mock-wivrn-server.py [system-name]. SIGUSR1 disconnects the headset."""

import signal
import sys

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

NAME = sys.argv[1] if len(sys.argv) > 1 else "Quest 3"
XML = """<node><interface name="io.github.wivrn.Server">
<property name="HeadsetConnected" type="b" access="read"/>
<property name="SystemName" type="s" access="read"/>
<property name="SessionRunning" type="b" access="read"/>
</interface></node>"""

state = {"HeadsetConnected": True, "SystemName": NAME, "SessionRunning": False}
loop = GLib.MainLoop()
connection = None


def get_property(conn, sender, path, interface, name):
    return GLib.Variant("b" if name != "SystemName" else "s", state[name])


def on_bus(conn, name):
    global connection
    connection = conn
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    conn.register_object("/io/github/wivrn/Server", node.interfaces[0], None, get_property, None)


def disconnect():
    state["HeadsetConnected"] = False
    state["SystemName"] = ""
    connection.emit_signal(
        None,
        "/io/github/wivrn/Server",
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        GLib.Variant("(sa{sv}as)", ("io.github.wivrn.Server", {"HeadsetConnected": GLib.Variant("b", False)}, [])),
    )
    return False


Gio.bus_own_name(
    Gio.BusType.SESSION, "io.github.wivrn.Server", Gio.BusNameOwnerFlags.NONE, on_bus, None, lambda *a: loop.quit()
)
GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGUSR1, disconnect)
GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGTERM, loop.quit)
loop.run()
