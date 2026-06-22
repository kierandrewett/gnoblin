#!/usr/bin/env python3
# Regression: the topbar global-menu widget must consume a DBusMenu export,
# render/open a submenu through the normal worker path, and activate a leaf row
# by sending com.canonical.dbusmenu.Event(..., "clicked", ...).
import importlib.util
import pathlib
import subprocess
import sys
import time
import warnings

import gi

gi.require_version("Gio", "2.0")
from gi.repository import Gio, GLib

warnings.filterwarnings("ignore", category=DeprecationWarning)

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

BUS = "org.gnoblin.TestDbusMenu"
PATH = "/org/gnoblin/TestDbusMenu"

XML = """
<node>
  <interface name="com.canonical.dbusmenu">
    <method name="GetLayout">
      <arg type="i" name="parentId" direction="in"/>
      <arg type="i" name="recursionDepth" direction="in"/>
      <arg type="as" name="propertyNames" direction="in"/>
      <arg type="u" name="revision" direction="out"/>
      <arg type="(ia{sv}av)" name="layout" direction="out"/>
    </method>
    <method name="AboutToShow">
      <arg type="i" name="id" direction="in"/>
      <arg type="b" name="needUpdate" direction="out"/>
    </method>
    <method name="Event">
      <arg type="i" name="id" direction="in"/>
      <arg type="s" name="eventId" direction="in"/>
      <arg type="v" name="data" direction="in"/>
      <arg type="u" name="timestamp" direction="in"/>
    </method>
  </interface>
</node>
"""


def dbusmenu_node(node_id, props, children=None):
    return GLib.Variant(
        "(ia{sv}av)",
        (
            node_id,
            {k: GLib.Variant(t, v) for k, (t, v) in props.items()},
            children or [],
        ),
    )


class FakeDbusMenu:
    def __init__(self, conn):
        self.conn = conn
        self.calls = []
        self.events = []
        self.about_to_show = []
        node = Gio.DBusNodeInfo.new_for_xml(XML)
        self.registration = conn.register_object(
            PATH,
            node.interfaces[0],
            self._method_call,
            None,
            None,
        )
        reply = conn.call_sync(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            GLib.Variant("(su)", (BUS, 0)),
            GLib.VariantType.new("(u)"),
            Gio.DBusCallFlags.NONE,
            1000,
            None,
        ).unpack()[0]
        if reply not in (1, 4):  # primary owner / already owner
            raise RuntimeError(f"could not own {BUS}: RequestName reply {reply}")

    def close(self):
        try:
            self.conn.call_sync(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "ReleaseName",
                GLib.Variant("(s)", (BUS,)),
                GLib.VariantType.new("(u)"),
                Gio.DBusCallFlags.NONE,
                1000,
                None,
            )
        except Exception:
            pass
        try:
            self.conn.unregister_object(self.registration)
        except Exception:
            pass

    def _layout(self, parent_id):
        quit_item = dbusmenu_node(
            42,
            {
                "label": ("s", "_Quit"),
                "visible": ("b", True),
                "enabled": ("b", True),
            },
        )
        file_menu = dbusmenu_node(
            10,
            {
                "label": ("s", "_File"),
                "visible": ("b", True),
                "enabled": ("b", True),
                "children-display": ("s", "submenu"),
            },
            [quit_item],
        )
        if parent_id == 0:
            return (0, {}, [file_menu])
        if parent_id == 10:
            return (10, {}, [quit_item])
        return (parent_id, {}, [])

    def _method_call(self, _conn, sender, _path, _iface, method_name, params, invocation):
        unpacked = params.unpack()
        self.calls.append((method_name, unpacked))
        if method_name == "GetLayout":
            parent_id = unpacked[0]
            invocation.return_value(GLib.Variant("(u(ia{sv}av))", (1, self._layout(parent_id))))
            return
        if method_name == "AboutToShow":
            self.about_to_show.append(unpacked[0])
            invocation.return_value(GLib.Variant("(b)", (True,)))
            return
        if method_name == "Event":
            self.events.append(unpacked)
            invocation.return_value(GLib.Variant("()", ()))
            return
        invocation.return_error_literal(
            Gio.dbus_error_quark(),
            Gio.DBusError.UNKNOWN_METHOD,
            f"{sender} called unknown {method_name}",
        )


def pump_context():
    ctx = GLib.MainContext.default()
    while ctx.pending():
        ctx.iteration(False)


def wait_for(proc, dk, predicate, timeout=8):
    deadline = time.time() + timeout
    while time.time() < deadline:
        pump_context()
        if predicate():
            return True
        if proc.poll() is not None:
            return False
        if dk.crashed():
            return False
        time.sleep(0.05)
    pump_context()
    return predicate()


def wait_for_process(dk, needle, proc, timeout=5):
    return wait_for(proc, dk, lambda: bool(dk.processes(needle)), timeout)


def fail_with_logs(message, dk, log_path, service):
    print(f"FAIL: {message}")
    print(f"  calls={service.calls!r}")
    print(f"  about_to_show={service.about_to_show!r}")
    print(f"  events={service.events!r}")
    if dk.crashed():
        print(f"  compositor: {dk.crashed()}")
        print(dk._tail())
    if log_path.exists():
        tail = "\n".join(log_path.read_text(errors="replace").splitlines()[-40:])
        if tail:
            print(f"  topbar log tail:\n{tail}")
    return 1


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    service = None
    try:
        # Deliberately use the real default topbar layout here. The user-facing
        # regression is losing the appmenu from the default left zone, not just
        # breaking an isolated `left = appmenu` test layout.
        dk.boot(with_monitor=True)
        service = FakeDbusMenu(dk.conn)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_APPMENU_TEST"] = f"dbusmenu;{BUS};{PATH}"
        env["GNOBLIN_APPMENU_AUTOCLICK"] = "0"
        env["GNOBLIN_APPMENU_AUTOACTIVATE"] = "0"
        log_path = dk.tmp / "topbar-dbusmenu.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-topbar")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        if not wait_for_process(dk, "gnoblin-topbar", proc):
            return fail_with_logs("gnoblin-topbar did not stay running", dk, log_path, service)

        if not wait_for(proc, dk, lambda: any(c[0] == "GetLayout" and c[1][0] == 0 for c in service.calls)):
            return fail_with_logs("topbar never fetched the DBusMenu bar", dk, log_path, service)

        if not wait_for(proc, dk, lambda: 10 in service.about_to_show):
            return fail_with_logs("topbar never opened the File submenu", dk, log_path, service)

        if not wait_for(proc, dk, lambda: any(e[0] == 42 and e[1] == "clicked" for e in service.events)):
            return fail_with_logs("topbar never activated the DBusMenu leaf item", dk, log_path, service)

        if dk.crashed():
            return fail_with_logs("compositor crashed", dk, log_path, service)
        if proc.poll() is not None:
            return fail_with_logs(f"gnoblin-topbar exited rc={proc.returncode}", dk, log_path, service)

        print("PASS: topbar renders/opens/activates a DBusMenu global menu")
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        if logf:
            logf.close()
        if service:
            service.close()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
