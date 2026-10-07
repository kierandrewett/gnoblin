"""Left-click at a desktop position from inside the guest, using Mutter's RemoteDesktop interface.

Usage: python3 - X Y < guest-click.py

QEMU's absolute pointer does not map reliably onto a two-GPU desktop, so this clamps the pointer into
the top-left corner with a large relative move and then moves by the target offset.
"""
import sys
import time

from gi.repository import Gio, GLib

BTN_LEFT = 0x110
x, y = float(sys.argv[1]), float(sys.argv[2])

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
session_iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, iface, method, args):
    return bus.call_sync(
        "org.gnome.Mutter.RemoteDesktop", path, iface, method, args, None,
        Gio.DBusCallFlags.NONE, 10000, None,
    )


session = call(
    "/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None
).unpack()[0]
call(session, session_iface, "Start", None)
time.sleep(0.5)
call(session, session_iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (-8000.0, -8000.0)))
time.sleep(0.3)
call(session, session_iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (x, y)))
time.sleep(0.4)
call(session, session_iface, "NotifyPointerButton", GLib.Variant("(ib)", (BTN_LEFT, True)))
time.sleep(0.1)
call(session, session_iface, "NotifyPointerButton", GLib.Variant("(ib)", (BTN_LEFT, False)))
time.sleep(0.6)
call(session, session_iface, "Stop", None)
print("clicked", x, y)
