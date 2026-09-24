#!/usr/bin/env python3
"""Opt-in session-lock policy broker.

The broker is deliberately not installed/enabled by the Gnoblin session yet.
It needs a compositor implementation of ext-session-lock-v1 before it can make
a security claim.  See README.md for the installation and verification gate.
"""

from __future__ import annotations

import configparser
import os
from pathlib import Path
import shlex
import subprocess
import sys

from gi.repository import Gio, GLib

from policy import LockPolicy, State

BUS_NAME = "org.gnoblin.Lock"
OBJECT_PATH = "/org/gnoblin/Lock"
INTERFACE = "org.gnoblin.Lock"
LOGIN1 = "org.freedesktop.login1"

INTROSPECTION = Gio.DBusNodeInfo.new_for_xml("""
<node>
  <interface name='org.gnoblin.Lock'>
    <method name='Lock'><arg name='reason' type='s' direction='in'/></method>
    <method name='GetState'><arg name='state' type='s' direction='out'/></method>
    <method name='ReportPresented'><arg name='token' type='s' direction='in'/></method>
    <method name='ReportFailed'><arg name='token' type='s' direction='in'/><arg name='reason' type='s' direction='in'/></method>
    <method name='Inhibit'><arg name='application' type='s' direction='in'/><arg name='reason' type='s' direction='in'/><arg name='cookie' type='u' direction='out'/></method>
    <method name='UnInhibit'><arg name='cookie' type='u' direction='in'/></method>
    <property name='Active' type='b' access='read'/>
  </interface>
</node>
""")


class Config:
    def __init__(self, path: Path):
        parser = configparser.ConfigParser(interpolation=None)
        parser.read(path)
        section = parser["Lock"] if parser.has_section("Lock") else {}
        self.enabled = str(section.get("Enabled", "false")).lower() == "true"
        self.command = section.get("Command", "").strip()
        self.idle_timeout_seconds = max(0, int(section.get("IdleTimeoutSeconds", "0")))
        # Compatibility names are unsafe while GNOME ScreenShield is active.
        self.own_screensaver_names = str(section.get("OwnScreenSaverNames", "false")).lower() == "true"


class LockBroker:
    def __init__(self, config: Config):
        self.config = config
        self.policy = LockPolicy()
        self.system_connection: Gio.DBusConnection | None = None
        self.session_connection: Gio.DBusConnection | None = None
        self.sleep_inhibitor_fd: int | None = None
        self.session_path: str | None = None
        self.locker: subprocess.Popen[str] | None = None
        self.presentation_timeout: int | None = None
        self.sleep_pending = False
        self.inhibitor_senders: dict[int, str] = {}

    def start(self) -> None:
        if not self.config.enabled:
            raise RuntimeError("[Lock] Enabled=true is required")
        if not self.config.command:
            raise RuntimeError("[Lock] Command is required")
        self.system_connection = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
        self.session_path = self._session_path()
        self._take_sleep_inhibitor()
        self._watch_login1()

    def _session_path(self) -> str:
        result = self.system_connection.call_sync(
            LOGIN1, "/org/freedesktop/login1", LOGIN1 + ".Manager", "GetSessionByPID",
            GLib.Variant("(u)", (os.getpid(),)), GLib.VariantType("(o)"),
            Gio.DBusCallFlags.NONE, -1, None)
        return result.unpack()[0]

    def _take_sleep_inhibitor(self) -> None:
        """Hold a *continuous* delay inhibitor before sleep can be announced."""
        if self.sleep_inhibitor_fd is not None:
            return
        result, fd_list = self.system_connection.call_with_unix_fd_list_sync(
            LOGIN1, "/org/freedesktop/login1", LOGIN1 + ".Manager", "Inhibit",
            GLib.Variant("(ssss)", ("sleep", "gnoblin-lockd", "wait for lock presentation", "delay")),
            GLib.VariantType("(h)"), Gio.DBusCallFlags.NONE, -1, None, None)
        self.sleep_inhibitor_fd = fd_list.get(result.unpack()[0])

    def _drop_sleep_inhibitor(self) -> None:
        if self.sleep_inhibitor_fd is not None:
            os.close(self.sleep_inhibitor_fd)
            self.sleep_inhibitor_fd = None

    def _watch_login1(self) -> None:
        self.system_connection.signal_subscribe(LOGIN1, LOGIN1 + ".Session", "Lock", self.session_path,
                                         None, Gio.DBusSignalFlags.NONE,
                                         lambda *_: self.request_lock("login1"))
        self.system_connection.signal_subscribe(LOGIN1, LOGIN1 + ".Manager", "PrepareForSleep",
                                         "/org/freedesktop/login1", None, Gio.DBusSignalFlags.NONE,
                                         self._prepare_for_sleep)

    def _prepare_for_sleep(self, _connection, _sender, _path, _iface, _signal, params) -> None:
        sleeping = params.unpack()[0]
        if sleeping:
            self.sleep_pending = True
            self.request_lock("sleep")
            if self.policy.state is State.PRESENTED:
                self._drop_sleep_inhibitor()
        else:
            # Re-establish the continuously held inhibitor after logind has
            # released its sleep transaction; the session must still be locked.
            self._take_sleep_inhibitor()
            self.sleep_pending = False

    def request_lock(self, reason: str) -> bool:
        request = self.policy.request_lock(reason)
        if request is None:
            return False
        env = os.environ.copy()
        env["GNOBLIN_LOCK_TOKEN"] = request.token
        env["GNOBLIN_LOCK_REASON"] = reason
        try:
            self.locker = subprocess.Popen(shlex.split(self.config.command), env=env, text=True)
        except (OSError, ValueError) as error:
            self.policy.report_failed(request.token)
            print(f"gnoblin-lockd: locker launch failed: {error}", file=sys.stderr)
            return False
        GLib.child_watch_add(GLib.PRIORITY_DEFAULT, self.locker.pid, self._locker_exited)
        self.presentation_timeout = GLib.timeout_add_seconds(5, self._presentation_timed_out)
        return True

    def _locker_exited(self, _pid: int, _status: int) -> None:
        if self.policy.state is State.REQUESTED and self.policy.active_token:
            self.policy.report_failed(self.policy.active_token)
        else:
            self.policy.locker_disconnected()

    def _presentation_timed_out(self) -> bool:
        self.presentation_timeout = None
        # Do not drop the delay FD and never mark active.  logind's configured
        # delay maximum eventually proceeds; only compositor black fallback can
        # make that safe, hence this service is not production-enabled.
        return GLib.SOURCE_REMOVE

    def report_presented(self, token: str) -> bool:
        if not self.policy.report_presented(token):
            return False
        if self.presentation_timeout is not None:
            GLib.source_remove(self.presentation_timeout)
            self.presentation_timeout = None
        # Prototype signal only.  A real compositor implementation must make
        # `locked` attest full-output presentation before a pending sleep may
        # proceed.  Keep the continuous inhibitor for ordinary/manual locks.
        if self.sleep_pending:
            self._drop_sleep_inhibitor()
        return True

    def report_failed(self, token: str, reason: str) -> bool:
        accepted = self.policy.report_failed(token)
        if accepted:
            print(f"gnoblin-lockd: locker failed: {reason}", file=sys.stderr)
        return accepted

    def method_call(self, _connection, sender, _path, _interface, method, parameters, invocation):
        if method == "Lock":
            self.request_lock(parameters.unpack()[0])
            invocation.return_value(None)
        elif method == "GetState":
            invocation.return_value(GLib.Variant("(s)", (self.policy.state.value,)))
        elif method == "ReportPresented":
            if not self.report_presented(parameters.unpack()[0]):
                invocation.return_dbus_error("org.gnoblin.Lock.Error.InvalidPresentation", "unknown lock token")
            else:
                invocation.return_value(None)
        elif method == "ReportFailed":
            token, reason = parameters.unpack()
            if not self.report_failed(token, reason):
                invocation.return_dbus_error("org.gnoblin.Lock.Error.InvalidPresentation", "unknown lock token")
            else:
                invocation.return_value(None)
        elif method == "Inhibit":
            app, reason = parameters.unpack()
            cookie = self.policy.inhibit(app, reason)
            self.inhibitor_senders[cookie] = sender
            invocation.return_value(GLib.Variant("(u)", (cookie,)))
        elif method == "UnInhibit":
            cookie = parameters.unpack()[0]
            if self.inhibitor_senders.get(cookie) == sender:
                self.inhibitor_senders.pop(cookie, None)
                self.policy.uninhibit(cookie)
            invocation.return_value(None)

    def get_property(self, _connection, _sender, _path, _interface, name):
        if name == "Active":
            return GLib.Variant("b", self.policy.state is State.PRESENTED)
        return None

    def bus_acquired(self, connection, _name) -> None:
        self.session_connection = connection
        # PyGObject maps these direct callbacks to GClosures.  Constructing a
        # Gio.DBusInterfaceVTable from Python does not support callback kwargs.
        connection.register_object(OBJECT_PATH, INTROSPECTION.interfaces[0],
                                   self.method_call, self.get_property, None)
        connection.signal_subscribe("org.freedesktop.DBus", "org.freedesktop.DBus",
                                    "NameOwnerChanged", "/org/freedesktop/DBus", None,
                                    Gio.DBusSignalFlags.NONE, self._name_owner_changed)

    def _name_owner_changed(self, _connection, _sender, _path, _iface, _signal, parameters) -> None:
        name, _old_owner, new_owner = parameters.unpack()
        if new_owner:
            return
        for cookie, sender in tuple(self.inhibitor_senders.items()):
            if sender == name:
                self.inhibitor_senders.pop(cookie, None)
                self.policy.uninhibit(cookie)


def main() -> int:
    config_path = Path(os.environ.get("GNOBLIN_LOCK_CONFIG", Path.home() / ".config/gnoblin/lock.conf"))
    broker = LockBroker(Config(config_path))
    broker.start()
    Gio.bus_own_name(Gio.BusType.SESSION, BUS_NAME, Gio.BusNameOwnerFlags.DO_NOT_QUEUE,
                     broker.bus_acquired, None, None)
    GLib.MainLoop().run()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
