"""Click one pointer button at a desktop position from inside the guest, using Mutter's RemoteDesktop interface.

Usage: python3 guest-click-button.py left|middle|right X Y

Like guest-click.py, the pointer is clamped to the top-left corner first, so the target does not depend on QEMU's
pointer mapping.
"""
import sys
import time

from gi.repository import Gio, GLib

BUTTONS = {"left": 0x110, "right": 0x111, "middle": 0x112}
button = BUTTONS[sys.argv[1]]
x, y = float(sys.argv[2]), float(sys.argv[3])

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
call(session, session_iface, "NotifyPointerButton", GLib.Variant("(ib)", (button, True)))
time.sleep(0.1)
call(session, session_iface, "NotifyPointerButton", GLib.Variant("(ib)", (button, False)))
time.sleep(0.6)
call(session, session_iface, "Stop", None)
