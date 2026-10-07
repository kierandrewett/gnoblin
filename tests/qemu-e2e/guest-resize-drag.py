"""Hold Super and drag with the right button from a point inside a window, using Mutter's RemoteDesktop.

Usage: python3 guest-resize-drag.py START_X START_Y DELTA_X DELTA_Y [right|left]

The pointer is clamped to the top-left corner first, then moved to START by a relative move, so the
start position does not depend on QEMU's absolute pointer mapping. Prints the drag it performed.
"""
import sys
import time

from gi.repository import Gio, GLib

KEY_LEFTMETA = 125
BTN_RIGHT = 0x111
BTN_LEFT = 0x110
start_x, start_y, delta_x, delta_y = (float(value) for value in sys.argv[1:5])
button = BTN_LEFT if len(sys.argv) > 5 and sys.argv[5] == "left" else BTN_RIGHT

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
call(session, session_iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (start_x, start_y)))
time.sleep(0.4)
call(session, session_iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (KEY_LEFTMETA, True)))
time.sleep(0.2)
call(session, session_iface, "NotifyPointerButton", GLib.Variant("(ib)", (button, True)))
time.sleep(0.3)
steps = 12
for _ in range(steps):
    call(session, session_iface, "NotifyPointerMotionRelative",
         GLib.Variant("(dd)", (delta_x / steps, delta_y / steps)))
    time.sleep(0.05)
time.sleep(0.4)
call(session, session_iface, "NotifyPointerButton", GLib.Variant("(ib)", (button, False)))
time.sleep(0.2)
call(session, session_iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (KEY_LEFTMETA, False)))
time.sleep(0.4)
call(session, session_iface, "Stop", None)
print("dragged from", start_x, start_y, "by", delta_x, delta_y)
