#!/usr/bin/env python3
"""Keep one D-Bus peer alive while exercising Mutter's remote-session path."""

from gi.repository import Gio, GLib


def call(connection, destination, object_path, interface, method, parameters=None):
    return connection.call_sync(
        destination,
        object_path,
        interface,
        method,
        parameters,
        None,
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    )


bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
remote_path = call(
    bus,
    "org.gnome.Mutter.RemoteDesktop",
    "/org/gnome/Mutter/RemoteDesktop",
    "org.gnome.Mutter.RemoteDesktop",
    "CreateSession",
).unpack()[0]

remote_id = call(
    bus,
    "org.gnome.Mutter.RemoteDesktop",
    remote_path,
    "org.freedesktop.DBus.Properties",
    "Get",
    GLib.Variant("(ss)", ("org.gnome.Mutter.RemoteDesktop.Session", "SessionId")),
).unpack()[0]

screen_path = call(
    bus,
    "org.gnome.Mutter.ScreenCast",
    "/org/gnome/Mutter/ScreenCast",
    "org.gnome.Mutter.ScreenCast",
    "CreateSession",
    GLib.Variant(
        "(a{sv})",
        ({"remote-desktop-session-id": GLib.Variant("s", remote_id)},),
    ),
).unpack()[0]

call(
    bus,
    "org.gnome.Mutter.ScreenCast",
    screen_path,
    "org.gnome.Mutter.ScreenCast.Session",
    "RecordMonitor",
    GLib.Variant("(sa{sv})", ("Meta-0", {})),
)
call(
    bus,
    "org.gnome.Mutter.RemoteDesktop",
    remote_path,
    "org.gnome.Mutter.RemoteDesktop.Session",
    "Start",
)
for pressed in (True, False):
    call(
        bus,
        "org.gnome.Mutter.RemoteDesktop",
        remote_path,
        "org.gnome.Mutter.RemoteDesktop.Session",
        "NotifyKeyboardKeycode",
        GLib.Variant("(ub)", (30, pressed)),
    )

print("PASS: raw Mutter accepted locked monitor ScreenCast and RemoteDesktop key notifications")
