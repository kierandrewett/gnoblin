#!/usr/bin/env python3
"""Focused, headless tests for lock-policy transitions only."""

import pathlib
import sys
import unittest

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "src" / "lock"))
from policy import LockPolicy, State


class LockPolicyTests(unittest.TestCase):
    def test_presentation_requires_the_active_capability(self):
        policy = LockPolicy()
        request = policy.request_lock("manual")
        self.assertIsNotNone(request)
        self.assertFalse(policy.report_client_presented("wrong"))
        self.assertEqual(policy.state, State.REQUESTED)
        self.assertTrue(policy.report_client_presented(request.token))
        self.assertEqual(policy.state, State.REQUESTED)
        self.assertTrue(policy.compositor_locked(now=12))
        self.assertEqual(policy.state, State.COMPOSITOR_LOCKED)
        self.assertEqual(policy.active_since, 12)

    def test_client_report_is_diagnostic_only(self):
        policy = LockPolicy()
        request = policy.request_lock("idle")
        self.assertTrue(policy.report_client_presented(request.token))
        self.assertEqual(policy.state, State.REQUESTED)
        self.assertEqual(policy.active_reason, "idle")

    def test_second_request_cannot_replace_current_locker(self):
        policy = LockPolicy()
        first = policy.request_lock("manual")
        self.assertIsNone(policy.request_lock("sleep"))
        self.assertTrue(policy.compositor_locked())
        self.assertIsNone(policy.request_lock("idle"))

    def test_client_death_after_presentation_fails_closed(self):
        policy = LockPolicy()
        request = policy.request_lock("sleep")
        policy.compositor_locked()
        policy.locker_disconnected()
        self.assertEqual(policy.state, State.COMPOSITOR_LOCKED)
        self.assertIsNotNone(policy.active_token)

    def test_client_death_before_confirmation_is_failed_not_unlocked(self):
        policy = LockPolicy()
        policy.request_lock("sleep")
        policy.locker_disconnected()
        self.assertEqual(policy.state, State.FAILED)

    def test_locker_failure_can_be_retried_but_is_not_an_unlock(self):
        policy = LockPolicy()
        request = policy.request_lock("sleep")
        self.assertTrue(policy.report_failed(request.token))
        self.assertEqual(policy.state, State.FAILED)
        self.assertIsNotNone(policy.request_lock("retry"))

    def test_only_compositor_unlock_completion_restores_state(self):
        policy = LockPolicy()
        request = policy.request_lock("manual")
        policy.compositor_locked()
        policy.compositor_unlocked()
        self.assertEqual(policy.state, State.UNLOCKED)
        self.assertIsNone(policy.active_token)

    def test_idle_inhibitors_are_cookie_scoped(self):
        policy = LockPolicy()
        first = policy.inhibit("player", "video")
        second = policy.inhibit("call", "meeting")
        self.assertFalse(policy.idle_lock_allowed)
        self.assertTrue(policy.uninhibit(first))
        self.assertFalse(policy.idle_lock_allowed)
        self.assertTrue(policy.uninhibit(second))
        self.assertTrue(policy.idle_lock_allowed)


if __name__ == "__main__":
    unittest.main()
