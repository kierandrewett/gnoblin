"""Hold the left button on a window titlebar and drag, using Mutter's RemoteDesktop.

Usage: python3 guest-titlebar-drag.py START_X START_Y DELTA_X DELTA_Y

The pointer is clamped to the top-left corner first, so the start position does not depend on QEMU's pointer mapping.
"""
import sys
import time

from gi.repository import Gio, GLib

start_x, start_y, delta_x, delta_y = (float(v) for v in sys.argv[1:5])
BTN_LEFT = 0x110
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (-8000.0, -8000.0)))
time.sleep(0.2)
call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (start_x, start_y)))
time.sleep(0.4)
call(session, iface, "NotifyPointerButton", GLib.Variant("(ib)", (BTN_LEFT, True)))
time.sleep(0.3)
steps = 20
for _ in range(steps):
    call(session, iface, "NotifyPointerMotionRelative", GLib.Variant("(dd)", (delta_x / steps, delta_y / steps)))
    time.sleep(0.04)
time.sleep(0.8)
call(session, iface, "NotifyPointerButton", GLib.Variant("(ib)", (BTN_LEFT, False)))
time.sleep(0.8)
call(session, iface, "Stop", None)
