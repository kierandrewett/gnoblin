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
import sys
import time

from gi.repository import Gio, GLib

from policy import LockPolicy, State

BUS_NAME = "org.gnoblin.Lock"
OBJECT_PATH = "/org/gnoblin/Lock"
INTERFACE = "org.gnoblin.Lock"
LOGIN1 = "org.freedesktop.login1"
NATIVE_NAME = "org.gnoblin.SessionLock"
NATIVE_PATH = "/org/gnoblin/SessionLock"
NATIVE_INTERFACE = "org.gnoblin.SessionLock"
SCREENSAVER_PATH = "/org/gnome/ScreenSaver"

INTROSPECTION = Gio.DBusNodeInfo.new_for_xml("""
<node>
  <interface name='org.gnoblin.Lock'>
    <method name='Lock'><arg name='reason' type='s' direction='in'/></method>
    <method name='GetState'><arg name='state' type='s' direction='out'/></method>
    <method name='GetLastReason'><arg name='reason' type='s' direction='out'/></method>
    <method name='ReportPresented'><arg name='token' type='s' direction='in'/></method>
    <method name='ReportFailed'><arg name='token' type='s' direction='in'/><arg name='reason' type='s' direction='in'/></method>
    <method name='Inhibit'><arg name='application' type='s' direction='in'/><arg name='reason' type='s' direction='in'/><arg name='cookie' type='u' direction='out'/></method>
    <method name='UnInhibit'><arg name='cookie' type='u' direction='in'/></method>
    <property name='Active' type='b' access='read'/>
    <property name='CompatibilityReady' type='b' access='read'/>
  </interface>
</node>
""")

SCREENSAVER_INTROSPECTION = Gio.DBusNodeInfo.new_for_xml("""
<node>
  <interface name='org.gnome.ScreenSaver'>
    <method name='Lock'/>
    <method name='GetActive'><arg name='active' type='b' direction='out'/></method>
    <method name='SetActive'><arg name='active' type='b' direction='in'/></method>
    <method name='GetActiveTime'><arg name='seconds' type='u' direction='out'/></method>
    <signal name='ActiveChanged'><arg name='active' type='b'/></signal>
    <signal name='WakeUpScreen'/>
  </interface>
</node>
""")


class Config:
    def __init__(self, path: Path):
        parser = configparser.ConfigParser(interpolation=None)
        parser.read(path)
        section = parser["Lock"] if parser.has_section("Lock") else {}
        self.enabled = str(section.get("Enabled", "false")).lower() == "true"
        self.idle_timeout_seconds = max(0, int(section.get("IdleTimeoutSeconds", "0")))
        self.own_compatibility_names = str(section.get("OwnCompatibilityNames", "false")).lower() == "true"


class LockBroker:
    def __init__(self, config: Config):
        self.config = config
        self.policy = LockPolicy()
        self.system_connection: Gio.DBusConnection | None = None
        self.session_connection: Gio.DBusConnection | None = None
        self.sleep_inhibitor_fd: int | None = None
        self.session_path: str | None = None
        self.presentation_timeout: int | None = None
        self.sleep_pending = False
        self.inhibitor_senders: dict[int, str] = {}
        self.idle_watch_id: int | None = None
        self.native_state = "unavailable"
        self.native_active = False
        self.native_capability = 0
        self.native_launcher_ready = False
        self.native_active_since: float | None = None
        self.compatibility_name_ids: list[int] = []
        self.compatibility_owners: set[str] = set()

    def start(self) -> None:
        if not self.config.enabled:
            raise RuntimeError("[Lock] Enabled=true is required")
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
            if self.policy.state is State.COMPOSITOR_LOCKED:
                self._drop_sleep_inhibitor()
            else:
                self.request_lock("sleep")
        else:
            # Re-establish the continuously held inhibitor after logind has
            # released its sleep transaction; the session must still be locked.
            self._take_sleep_inhibitor()
            self.sleep_pending = False

    def request_lock(self, reason: str) -> bool:
        request = self.policy.request_lock(reason)
        if request is None:
            return False
        try:
            accepted = self.session_connection.call_sync(
                NATIVE_NAME, NATIVE_PATH, NATIVE_INTERFACE, "RequestLock",
                GLib.Variant("(s)", (reason,)), GLib.VariantType("(b)"),
                Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
        except GLib.Error as error:
            self.policy.report_failed(request.token)
            print(f"gnoblin-lockd: native lock request failed: {error}", file=sys.stderr)
            return False
        if not accepted:
            self.policy.report_failed(request.token)
            return False
        self.presentation_timeout = GLib.timeout_add_seconds(5, self._presentation_timed_out)
        return True

    def _presentation_timed_out(self) -> bool:
        self.presentation_timeout = None
        # Do not drop the delay FD and never mark active.  logind's configured
        # delay maximum eventually proceeds; only compositor black fallback can
        # make that safe, hence this service is not production-enabled.
        return GLib.SOURCE_REMOVE

    def report_presented(self, token: str) -> bool:
        return self.policy.report_client_presented(token)

    def compositor_lock_confirmed(self) -> bool:
        """Reserved for a compositor-authoritative callback, never D-Bus."""
        if not self.policy.compositor_locked():
            return False
        if self.presentation_timeout is not None:
            GLib.source_remove(self.presentation_timeout)
            self.presentation_timeout = None
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
        elif method == "GetLastReason":
            invocation.return_value(GLib.Variant("(s)", (self.policy.active_reason or "",)))
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
            return GLib.Variant("b", self.native_active)
        if name == "CompatibilityReady":
            return GLib.Variant("b", self.compatibility_ready)
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
        self._start_idle_monitor()
        self._watch_native_lock()

    @property
    def compatibility_ready(self) -> bool:
        return (
            self.config.own_compatibility_names
            and self.native_capability >= 1
            and self.native_launcher_ready
            and {"org.gnome.ScreenSaver", "org.gnome.Shell.ScreenShield"} <= self.compatibility_owners
        )

    def _watch_native_lock(self) -> None:
        self.session_connection.signal_subscribe(
            NATIVE_NAME, NATIVE_INTERFACE, "StateChanged", NATIVE_PATH, None,
            Gio.DBusSignalFlags.NONE, self._native_state_changed)
        self.session_connection.signal_subscribe(
            NATIVE_NAME, "org.freedesktop.DBus.Properties", "PropertiesChanged", NATIVE_PATH, None,
            Gio.DBusSignalFlags.NONE, self._native_properties_changed)
        try:
            properties = self.session_connection.call_sync(
                NATIVE_NAME, NATIVE_PATH, "org.freedesktop.DBus.Properties", "GetAll",
                GLib.Variant("(s)", (NATIVE_INTERFACE,)), GLib.VariantType("(a{sv})"),
                Gio.DBusCallFlags.NONE, 1000, None).unpack()[0]
            self._apply_native_properties(properties)
        except GLib.Error:
            # The native server is optional until protocol/capture validation.
            return

    def _native_properties_changed(self, _connection, _sender, _path, _iface, _signal, parameters) -> None:
        interface, changed, _invalidated = parameters.unpack()
        if interface == NATIVE_INTERFACE:
            self._apply_native_properties(changed)

    def _apply_native_properties(self, properties) -> None:
        values = {key: value.unpack() for key, value in properties.items()}
        state = values.get("State", self.native_state)
        active = values.get("Active", self.native_active)
        self.native_capability = values.get("Capability", self.native_capability)
        self.native_launcher_ready = values.get("LauncherReady", self.native_launcher_ready)
        self._apply_native_state(state, active)
        self._maybe_own_compatibility_names()

    def _native_state_changed(self, _connection, _sender, _path, _iface, _signal, parameters) -> None:
        state, active, _presentation_confirmed = parameters.unpack()
        self._apply_native_state(state, active)

    def _apply_native_state(self, state: str, active: bool) -> None:
        was_active = self.native_active
        self.native_state = state
        self.native_active = active
        if active and not was_active:
            self.native_active_since = time.monotonic()
        elif not active:
            self.native_active_since = None
        if state == "unlocked":
            self.policy.compositor_unlocked()
        elif state == "covering":
            self.policy.compositor_covering()
        elif state in ("locked", "failsafe"):
            self.policy.compositor_locked()
        self._set_locked_hint(active)
        if was_active != active and self.compatibility_owners:
            self.session_connection.emit_signal(
                None, SCREENSAVER_PATH, "org.gnome.ScreenSaver", "ActiveChanged",
                GLib.Variant("(b)", (active,)))

    def _set_locked_hint(self, active: bool) -> None:
        if self.system_connection is None or self.session_path is None:
            return
        self.system_connection.call(
            LOGIN1, self.session_path, LOGIN1 + ".Session", "SetLockedHint",
            GLib.Variant("(b)", (active,)), None, Gio.DBusCallFlags.NONE, 5000, None, None, None)

    def _maybe_own_compatibility_names(self) -> None:
        if (not self.config.own_compatibility_names or self.native_capability < 1 or
                not self.native_launcher_ready or self.compatibility_name_ids):
            return
        self.session_connection.register_object(
            SCREENSAVER_PATH, SCREENSAVER_INTROSPECTION.interfaces[0],
            self._screensaver_method_call, self._screensaver_get_property, None)
        for name in ("org.gnome.ScreenSaver", "org.gnome.Shell.ScreenShield"):
            self.compatibility_name_ids.append(Gio.bus_own_name_on_connection(
                self.session_connection, name, Gio.BusNameOwnerFlags.DO_NOT_QUEUE,
                lambda _connection, acquired, name=name: self._compatibility_name_acquired(name),
                lambda _connection, _name: None))

    def _compatibility_name_acquired(self, name: str) -> None:
        self.compatibility_owners.add(name)

    def _screensaver_method_call(self, _connection, _sender, _path, _interface, method, parameters, invocation):
        if method == "Lock":
            self.request_lock("compatibility")
            invocation.return_value(None)
        elif method == "SetActive":
            if parameters.unpack()[0]:
                self.request_lock("compatibility")
            invocation.return_value(None)
        elif method == "GetActive":
            invocation.return_value(GLib.Variant("(b)", (self.native_active,)))
        elif method == "GetActiveTime":
            seconds = 0 if self.native_active_since is None else int(time.monotonic() - self.native_active_since)
            invocation.return_value(GLib.Variant("(u)", (seconds,)))

    def _screensaver_get_property(self, *_args):
        return None

    def _idle_call(self, method: str, parameters: GLib.Variant, reply_type: str):
        return self.session_connection.call_sync(
            "org.gnome.Mutter.IdleMonitor", "/org/gnome/Mutter/IdleMonitor/Core",
            "org.gnome.Mutter.IdleMonitor", method, parameters,
            GLib.VariantType(reply_type), Gio.DBusCallFlags.NONE, 5000, None).unpack()

    def _start_idle_monitor(self) -> None:
        """Use compositor input time, never a broker wall-clock timer."""
        timeout = self.config.idle_timeout_seconds
        if timeout == 0:
            return
        threshold_ms = timeout * 1000
        try:
            self.session_connection.signal_subscribe(
                "org.gnome.Mutter.IdleMonitor", "org.gnome.Mutter.IdleMonitor", "WatchFired",
                "/org/gnome/Mutter/IdleMonitor/Core", None, Gio.DBusSignalFlags.NONE,
                self._idle_watch_fired)
            self.idle_watch_id = self._idle_call(
                "AddIdleWatch", GLib.Variant("(t)", (threshold_ms,)), "(u)")[0]
            idle_ms = self._idle_call("GetIdletime", GLib.Variant("()", ()), "(t)")[0]
            if idle_ms >= threshold_ms:
                self._request_idle_lock()
        except GLib.Error as error:
            # Failing closed here means no idle-triggered lock. Do not replace
            # compositor time with a less reliable wall-clock approximation.
            print(f"gnoblin-lockd: idle monitor unavailable: {error}", file=sys.stderr)

    def _idle_watch_fired(self, _connection, _sender, _path, _iface, _signal, parameters) -> None:
        if parameters.unpack()[0] == self.idle_watch_id:
            self._request_idle_lock()

    def _request_idle_lock(self) -> None:
        if self.policy.idle_lock_allowed:
            self.request_lock("idle")

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
