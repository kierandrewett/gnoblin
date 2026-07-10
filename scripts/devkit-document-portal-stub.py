#!/usr/bin/env python3
"""gnoblin devkit -- minimal org.freedesktop.portal.Documents stub.

devkit_dbus.py isolates the devkit's D-Bus session from the host: it copies in
only the portal services a nested gnoblin session needs, plus this stub for
org.freedesktop.portal.Documents. The real xdg-document-portal FUSE-mounts a
per-app document store under $XDG_RUNTIME_DIR/doc; that's the wrong thing to
do from an isolated devkit bus (wrong runtime dir, and it doesn't clean up
when the devkit session ends). This stub owns the same bus name and object
path and answers the read-only calls a portal client checks first
(``version``, ``GetMountPoint``) so activation succeeds instead of failing --
it has no document store, so every method that would create, grant, list, or
look up a document returns org.freedesktop.DBus.Error.NotSupported. Devkit
sessions don't exercise sandboxed document access, so that is enough for a
normal dev/test run; a client that actually needs the document portal should
run outside the devkit.

Usage: devkit-document-portal-stub.py <mount-dir>
"""

from __future__ import annotations

import sys

import gi

gi.require_version("GLib", "2.0")
gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib  # noqa: E402

BUS_NAME = "org.freedesktop.portal.Documents"
# NB: object path uses lowercase "documents" even though the bus name and
# interface are capitalised -- that's the real portal's own layout
# (/usr/share/dbus-1/interfaces/org.freedesktop.portal.Documents.xml).
OBJECT_PATH = "/org/freedesktop/portal/documents"
INTERFACE_NAME = "org.freedesktop.portal.Documents"

# Every method the real portal exposes (arg signatures copied from
# /usr/share/dbus-1/interfaces/org.freedesktop.portal.Documents.xml), so a
# real call is routed to our handler instead of being rejected by GDBus's own
# signature check before we get a chance to return NotSupported.
UNSUPPORTED_METHODS = {
    "Add": '<arg type="h" name="o_path_fd" direction="in"/>'
           '<arg type="b" name="reuse_existing" direction="in"/>'
           '<arg type="b" name="persistent" direction="in"/>'
           '<arg type="s" name="doc_id" direction="out"/>',
    "AddNamed": '<arg type="h" name="o_path_parent_fd" direction="in"/>'
                '<arg type="ay" name="filename" direction="in"/>'
                '<arg type="b" name="reuse_existing" direction="in"/>'
                '<arg type="b" name="persistent" direction="in"/>'
                '<arg type="s" name="doc_id" direction="out"/>',
    "AddFull": '<arg type="ah" name="o_path_fds" direction="in"/>'
               '<arg type="u" name="flags" direction="in"/>'
               '<arg type="s" name="app_id" direction="in"/>'
               '<arg type="as" name="permissions" direction="in"/>'
               '<arg type="as" name="doc_ids" direction="out"/>'
               '<arg type="a{sv}" name="extra_out" direction="out"/>',
    "AddNamedFull": '<arg type="h" name="o_path_fd" direction="in"/>'
                    '<arg type="ay" name="filename" direction="in"/>'
                    '<arg type="u" name="flags" direction="in"/>'
                    '<arg type="s" name="app_id" direction="in"/>'
                    '<arg type="as" name="permissions" direction="in"/>'
                    '<arg type="s" name="doc_id" direction="out"/>'
                    '<arg type="a{sv}" name="extra_out" direction="out"/>',
    "GrantPermissions": '<arg type="s" name="doc_id" direction="in"/>'
                        '<arg type="s" name="app_id" direction="in"/>'
                        '<arg type="as" name="permissions" direction="in"/>',
    "RevokePermissions": '<arg type="s" name="doc_id" direction="in"/>'
                         '<arg type="s" name="app_id" direction="in"/>'
                         '<arg type="as" name="permissions" direction="in"/>',
    "Delete": '<arg type="s" name="doc_id" direction="in"/>',
    "Lookup": '<arg type="ay" name="filename" direction="in"/>'
              '<arg type="s" name="doc_id" direction="out"/>',
    "Info": '<arg type="s" name="doc_id" direction="in"/>'
            '<arg type="ay" name="path" direction="out"/>'
            '<arg type="a{sas}" name="apps" direction="out"/>',
    "List": '<arg type="s" name="app_id" direction="in"/>'
            '<arg type="a{say}" name="docs" direction="out"/>',
    "GetHostPaths": '<arg type="as" name="doc_ids" direction="in"/>'
                    '<arg type="a{say}" name="paths" direction="out"/>',
}

INTERFACE_XML = f"""
<node>
  <interface name="{INTERFACE_NAME}">
    <property name="version" type="u" access="read"/>
    <method name="GetMountPoint">
      <arg type="ay" name="path" direction="out"/>
    </method>
    {"".join(f'<method name="{name}">{args}</method>' for name, args in UNSUPPORTED_METHODS.items())}
  </interface>
</node>
"""


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: devkit-document-portal-stub.py <mount-dir>", file=sys.stderr)
        return 1
    mount_point = sys.argv[1].encode() + b"\0"

    node_info = Gio.DBusNodeInfo.new_for_xml(INTERFACE_XML)
    iface_info = node_info.lookup_interface(INTERFACE_NAME)
    loop = GLib.MainLoop()

    def handle_method_call(connection, sender, object_path, interface_name,
                            method_name, parameters, invocation):
        del connection, sender, object_path, interface_name, parameters
        if method_name == "GetMountPoint":
            invocation.return_value(GLib.Variant("(ay)", (mount_point,)))
            return
        invocation.return_dbus_error(
            "org.freedesktop.DBus.Error.NotSupported",
            f"gnoblin devkit stub: {method_name} needs the real xdg-document-portal",
        )

    def handle_get_property(connection, sender, object_path, interface_name,
                             property_name):
        del connection, sender, object_path, interface_name
        if property_name == "version":
            return GLib.Variant("u", 5)
        return None

    def on_bus_acquired(connection, name):
        del name
        connection.register_object(
            OBJECT_PATH, iface_info,
            handle_method_call, handle_get_property, None)

    def on_name_lost(connection, name):
        # The real portal already owns the name, or D-Bus activation raced --
        # either way there's nothing left for the stub to do.
        del connection, name
        loop.quit()

    Gio.bus_own_name(
        Gio.BusType.SESSION, BUS_NAME, Gio.BusNameOwnerFlags.NONE,
        on_bus_acquired, None, on_name_lost)

    loop.run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
