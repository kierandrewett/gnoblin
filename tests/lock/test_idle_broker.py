#!/usr/bin/env python3
"""Unit checks for compositor-driven idle dispatch, without a real session."""

import importlib.machinery
import pathlib
import sys
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "src" / "lock"))
lockd = importlib.machinery.SourceFileLoader(
    "gnoblin_lockd", str(ROOT / "src" / "lock" / "gnoblin-lockd.py")
).load_module()
from policy import LockPolicy


class IdleDispatchTests(unittest.TestCase):
    def make_broker(self):
        broker = lockd.LockBroker.__new__(lockd.LockBroker)
        broker.policy = LockPolicy()
        broker.idle_watch_id = 42
        broker.reasons = []
        broker.request_lock = broker.reasons.append
        return broker

    def test_idle_watch_dispatches_idle_reason(self):
        broker = self.make_broker()
        lockd.LockBroker._idle_watch_fired(
            broker, None, None, None, None, None, type("P", (), {"unpack": lambda _: (42,)})()
        )
        self.assertEqual(broker.reasons, ["idle"])

    def test_inhibitor_suppresses_idle_dispatch(self):
        broker = self.make_broker()
        broker.policy.inhibit("player", "video")
        lockd.LockBroker._request_idle_lock(broker)
        self.assertEqual(broker.reasons, [])

    def test_other_watch_is_ignored(self):
        broker = self.make_broker()
        lockd.LockBroker._idle_watch_fired(
            broker, None, None, None, None, None, type("P", (), {"unpack": lambda _: (41,)})()
        )
        self.assertEqual(broker.reasons, [])

    def test_disconnected_dbus_owner_loses_its_inhibitors(self):
        broker = self.make_broker()
        broker.inhibitor_senders = {7: ":1.7", 8: ":1.8"}
        broker.policy.inhibitors = {7: ("video", "playing"), 8: ("call", "active")}
        lockd.LockBroker._name_owner_changed(
            broker, None, None, None, None, None,
            type("P", (), {"unpack": lambda _: (":1.7", ":1.7", "")})(),
        )
        self.assertNotIn(7, broker.policy.inhibitors)
        self.assertIn(8, broker.policy.inhibitors)


if __name__ == "__main__":
    unittest.main()
