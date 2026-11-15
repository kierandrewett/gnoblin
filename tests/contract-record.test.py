#!/usr/bin/env python3
"""Guard the contract recording made from a live legacy session (API 1.93).

scripts/contract-record.py writes tests/parity/contracts/recorded-session.json from a running session, which this test
never starts. It proves the stored recording still holds together with the inventory: the same API version, every
operation recorded once, the events a mapped and closed window must cause, and that `--check` refuses a recording
that lost an operation.
"""

import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "tests/parity/contracts"
SCRIPT = ROOT / "scripts/contract-record.py"
RECORDED = CONTRACTS / "recorded-session.json"


def check(path: Path) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(SCRIPT), "--check", "--out", str(path)],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


class ContractRecordTests(unittest.TestCase):
    def setUp(self):
        self.recorded = json.loads(RECORDED.read_text(encoding="utf-8"))

    def test_checked_in_recording_matches_the_inventory(self):
        result = check(RECORDED)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_recording_is_from_the_inventory_api_version(self):
        inventory = json.loads((CONTRACTS / "operations.json").read_text(encoding="utf-8"))["api"]
        self.assertEqual(
            (self.recorded["api"]["major"], self.recorded["api"]["minor"]),
            (inventory["major"], inventory["minor"]),
        )
        self.assertEqual((self.recorded["hello"]["api_major"], self.recorded["hello"]["api_minor"]), (1, 93))

    def test_every_operation_is_recorded_once_with_a_known_status(self):
        inventory = json.loads((CONTRACTS / "operations.json").read_text(encoding="utf-8"))["entries"]
        names = [entry["name"] for entry in self.recorded["operations"]]
        self.assertEqual(sorted(names), sorted(entry["name"] for entry in inventory))
        for entry in self.recorded["operations"]:
            self.assertIn(entry["status"], ("ok", "error", "timeout", "skipped"), entry["name"])
        # An operation that answers nothing is a recording that did not finish, not a contract.
        self.assertEqual([entry["name"] for entry in self.recorded["operations"] if entry["status"] == "timeout"], [])

    def test_a_mapped_and_closed_window_caused_its_events(self):
        seen = {entry["name"] for entry in self.recorded["events_seen"]}
        for name in ("gnoblin.window.created", "gnoblin.window.focused", "gnoblin.window.closed"):
            self.assertIn(name, seen)

    def test_check_refuses_a_recording_that_lost_an_operation(self):
        broken = dict(self.recorded)
        broken["operations"] = self.recorded["operations"][1:]
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "recorded-session.json"
            path.write_text(json.dumps(broken), encoding="utf-8")
            result = check(path)
        self.assertEqual(result.returncode, 1)
        self.assertIn("was not recorded", result.stderr)


if __name__ == "__main__":
    unittest.main()
