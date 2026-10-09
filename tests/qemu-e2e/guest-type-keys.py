"""Type keys through Mutter RemoteDesktop. Usage: python3 guest-type-keys.py KEY... (p r i v e t f o enter)"""
import sys
import time

from gi.repository import Gio, GLib

CODES = {"p": 25, "r": 19, "i": 23, "v": 47, "e": 18, "t": 20, "f": 33, "o": 24, "enter": 28}
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
iface = "org.gnome.Mutter.RemoteDesktop.Session"


def call(path, interface, method, args):
    return bus.call_sync("org.gnome.Mutter.RemoteDesktop", path, interface, method, args, None,
                         Gio.DBusCallFlags.NONE, 10000, None)


session = call("/org/gnome/Mutter/RemoteDesktop", "org.gnome.Mutter.RemoteDesktop", "CreateSession", None).unpack()[0]
call(session, iface, "Start", None)
time.sleep(0.5)
for name in sys.argv[1:]:
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (CODES[name], True)))
    time.sleep(0.06)
    call(session, iface, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (CODES[name], False)))
    time.sleep(0.12)
time.sleep(0.5)
call(session, iface, "Stop", None)
