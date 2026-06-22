#!/usr/bin/env python3
"""Tiny document-portal stand-in for the isolated devkit bus.

The Mutter Devkit viewer needs org.freedesktop.portal.Desktop for ScreenCast,
but xdg-desktop-portal also probes org.freedesktop.portal.Documents at startup.
Running the real xdg-document-portal on the devkit's private bus tries to mount
FUSE at the user's real /run/user/.../doc and prints noisy permission warnings.

This stub implements only the GetMountPoint method that xdg-desktop-portal reads
during startup. File-chooser/document export workflows are out of scope for the
devkit smoke path; they should use a real session, not this isolated bus.
"""

import signal
import sys
import warnings

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

warnings.filterwarnings("ignore", category=DeprecationWarning)

MOUNT = sys.argv[1].encode() + b"\0"
XML = """
<node>
  <interface name="org.freedesktop.portal.Documents">
    <method name="GetMountPoint">
      <arg type="ay" name="path" direction="out"/>
    </method>
    <property name="version" type="u" access="read"/>
  </interface>
</node>
"""


def main():
    node = Gio.DBusNodeInfo.new_for_xml(XML)
    loop = GLib.MainLoop()

    def method_call(_conn, _sender, _path, _iface, method_name, _params, invocation):
        if method_name == "GetMountPoint":
            invocation.return_value(GLib.Variant("(ay)", (MOUNT,)))
            return
        invocation.return_error_literal(
            Gio.dbus_error_quark(),
            Gio.DBusError.UNKNOWN_METHOD,
            method_name,
        )

    def get_property(_conn, _sender, _path, _iface, prop_name):
        if prop_name == "version":
            return GLib.Variant("u", 4)
        return None

    conn = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    conn.register_object(
        "/org/freedesktop/portal/documents",
        node.interfaces[0],
        method_call,
        get_property,
        None,
    )
    owner = Gio.bus_own_name_on_connection(
        conn,
        "org.freedesktop.portal.Documents",
        Gio.BusNameOwnerFlags.NONE,
        None,
        None,
    )
    signal.signal(signal.SIGTERM, lambda *_: loop.quit())
    try:
        loop.run()
    finally:
        Gio.bus_unown_name(owner)


if __name__ == "__main__":
    main()
