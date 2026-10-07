import sys, time
from gi.repository import Gio, GLib
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
def call(path, iface, method, args, dest="org.freedesktop.GeoClue2"):
    return bus.call_sync(dest, path, iface, method, args, None, Gio.DBusCallFlags.NONE, 40000, None)
client = call("/org/freedesktop/GeoClue2/Manager", "org.freedesktop.GeoClue2.Manager", "GetClient", None).unpack()[0]
print("client", client)
P = "org.freedesktop.DBus.Properties"
C = "org.freedesktop.GeoClue2.Client"
call(client, P, "Set", GLib.Variant("(ssv)", (C, "DesktopId", GLib.Variant("s", "foot"))))
call(client, P, "Set", GLib.Variant("(ssv)", (C, "RequestedAccuracyLevel", GLib.Variant("u", 8))))
try:
    call(client, C, "Start", None)
    print("started")
except Exception as error:
    print("start failed:", error)
