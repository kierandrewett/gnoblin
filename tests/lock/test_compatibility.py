#!/usr/bin/env python3
"""Broker compatibility-name and compositor-state tests without a desktop."""

import importlib.machinery
import pathlib
import sys
import unittest
from unittest.mock import patch

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "src" / "lock"))
lockd = importlib.machinery.SourceFileLoader(
    "gnoblin_lockd", str(ROOT / "src" / "lock" / "gnoblin-lockd.py")
).load_module()
from policy import LockPolicy, State


class FakeConnection:
    def __init__(self):
        self.registered = []
        self.signals = []

    def register_object(self, *args):
        self.registered.append(args)
        return len(self.registered)

    def emit_signal(self, *args):
        self.signals.append(args)


class CompatibilityTests(unittest.TestCase):
    def make_broker(self):
        broker = lockd.LockBroker.__new__(lockd.LockBroker)
        broker.config = type("Config", (), {"own_compatibility_names": True})()
        broker.policy = LockPolicy()
        broker.session_connection = FakeConnection()
        broker.system_connection = None
        broker.session_path = None
        broker.native_capability = 1
        broker.native_launcher_ready = True
        broker.native_state = "unlocked"
        broker.native_active = False
        broker.native_active_since = None
        broker.compatibility_name_ids = []
        broker.compatibility_owners = set()
        return broker

    def test_readiness_requires_both_names(self):
        broker = self.make_broker()
        acquired = []

        def own(_connection, name, _flags, callback, _lost):
            acquired.append((name, callback))
            return len(acquired)

        with patch.object(lockd.Gio, "bus_own_name_on_connection", own):
            broker._maybe_own_compatibility_names()
        self.assertFalse(broker.compatibility_ready)
        self.assertEqual([name for name, _ in acquired], ["org.gnome.ScreenSaver", "org.gnome.Shell.ScreenShield"])
        acquired[0][1](None, acquired[0][0])
        self.assertFalse(broker.compatibility_ready)
        acquired[1][1](None, acquired[1][0])
        self.assertTrue(broker.compatibility_ready)

    def test_compositor_state_drives_active_and_never_client_report(self):
        broker = self.make_broker()
        broker.compatibility_owners = {"org.gnome.ScreenSaver"}
        broker._apply_native_state("covering", True)
        self.assertEqual(broker.policy.state, State.COMPOSITOR_COVERING)
        self.assertTrue(broker.native_active)
        self.assertEqual(broker.session_connection.signals[-1][2:4], ("org.gnome.ScreenSaver", "ActiveChanged"))
        broker._apply_native_state("locked", True)
        self.assertEqual(broker.policy.state, State.COMPOSITOR_LOCKED)
        broker._apply_native_state("unlocked", False)
        self.assertEqual(broker.policy.state, State.UNLOCKED)

    def test_native_locked_state_is_authoritative_without_broker_request(self):
        broker = self.make_broker()
        broker._apply_native_state("locked", True)
        self.assertEqual(broker.policy.state, State.COMPOSITOR_LOCKED)
        self.assertFalse(broker.policy.idle_lock_allowed)


if __name__ == "__main__":
    unittest.main()
