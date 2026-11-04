#!/usr/bin/env python3
"""Guard the parity contract inventory against staleness and drift.

The inventory in tests/parity/contracts is what the parity harness compares the candidate with, so
it is only useful while it matches the C sources. This test regenerates nothing: it asks the script
whether the checked-in files are current, then spot-checks the names that the shell and the harness
rely on and proves that every entry still points at a line that holds it.
"""

import json
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
CONTRACTS = ROOT / "tests/parity/contracts"
SCRIPT = ROOT / "scripts/contract-inventory.py"


def load(name: str) -> dict:
    return json.loads((CONTRACTS / name).read_text(encoding="utf-8"))


def entries(name: str) -> list[dict]:
    return load(name)["entries"]


class ContractInventoryTests(unittest.TestCase):
    def test_checked_in_inventory_is_current(self):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--check"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("stale:", result.stderr)

    def test_every_document_declares_its_kind_and_count(self):
        expected = {
            "operations.json": "operations",
            "events.json": "events",
            "cli.json": "cli",
            "config-keys.json": "config-keys",
        }
        for name, kind in expected.items():
            with self.subTest(document=name):
                document = load(name)
                self.assertEqual(document["format"], 1)
                self.assertEqual(document["kind"], kind)
                self.assertEqual(document["generated_by"], "scripts/contract-inventory.py")
                self.assertEqual(document["count"], len(document["entries"]))
                self.assertGreater(document["count"], 0)

    def test_operations_are_unique_sorted_and_traceable(self):
        operations = entries("operations.json")
        names = [operation["name"] for operation in operations]
        self.assertEqual(names, sorted(names))
        self.assertEqual(len(names), len(set(names)))
        for operation in operations:
            with self.subTest(operation=operation["name"]):
                line = (ROOT / operation["source"]).read_text(encoding="utf-8").splitlines()[operation["line"] - 1]
                self.assertIn(f'"{operation["name"]}"', line)
                self.assertIn(f"X({operation['id']},", line)

    def test_events_are_unique_and_traceable(self):
        events = entries("events.json")
        names = [event["name"] for event in events]
        self.assertEqual(names, sorted(names))
        self.assertEqual(len(names), len(set(names)))
        for event in events:
            with self.subTest(event=event["name"]):
                self.assertTrue(event["channels"])
                self.assertLessEqual(set(event["channels"]), {"socket", "runtime"})
                line = (ROOT / event["source"]).read_text(encoding="utf-8").splitlines()[event["line"] - 1]
                self.assertIn(f'"{event["name"]}"', line)

    def test_cli_commands_and_actions_are_traceable(self):
        commands = {entry["command"]: entry for entry in entries("cli.json")}
        self.assertEqual(list(commands), sorted(commands))
        for command, entry in commands.items():
            with self.subTest(command=command):
                line = (ROOT / entry["source"]).read_text(encoding="utf-8").splitlines()[entry["line"] - 1]
                self.assertIn(f'"{command}"', line)
                for action in entry["actions"]:
                    self.assertIsNotNone(action["summary"], action["action"])
                    action_line = (ROOT / action["source"]).read_text(encoding="utf-8").splitlines()[action["line"] - 1]
                    self.assertIn(f'"{action["action"]}"', action_line)

    def test_config_keys_are_traceable(self):
        for entry in entries("config-keys.json"):
            with self.subTest(key=entry["key"]):
                line = (ROOT / entry["source"]).read_text(encoding="utf-8").splitlines()[entry["line"] - 1]
                self.assertIn(f'"{entry["key"]}"', line)

    def test_known_operations_are_inventoried(self):
        operations = {entry["name"]: entry for entry in entries("operations.json")}
        for name in (
            "window.focus",
            "window.close",
            "window.thumbnail",
            "workspaces.list",
            "workspace.rename",
            "layers.list",
            "monitors.list",
            "session.lock",
            "config.path",
            "version",
        ):
            self.assertIn(name, operations)
        focus = operations["window.focus"]
        self.assertEqual(focus["gate_minor"], 10)
        self.assertEqual(focus["dispatch"], "GN_DISPATCH_GRANT")
        self.assertIn("GRANT", focus["flags"])
        self.assertEqual(focus["effective_cli_minor"], 64)
        self.assertEqual([call["path"] for call in focus["lua"]], ["Window:focus"])

    def test_known_events_are_inventoried(self):
        events = {entry["name"]: entry for entry in entries("events.json")}
        for name in (
            "gnoblin.session.lock-requested",
            "gnoblin.osd.requested",
            "gnoblin.window.created",
            "gnoblin.window.menu-requested",
            "gnoblin.process.exited",
        ):
            self.assertIn(name, events)
        self.assertTrue(events["gnoblin.session.lock-requested"]["subscription_table"])
        self.assertEqual(events["gnoblin.process.exited"]["channels"], ["runtime"])

    def test_bingux_cli_commands_are_inventoried(self):
        commands = {entry["command"]: entry for entry in entries("cli.json")}
        window = {action["action"] for action in commands["window"]["actions"]}
        self.assertLessEqual({"list", "focus", "workspace", "thumbnail"}, window)
        self.assertEqual([action["action"] for action in commands["layer"]["actions"]], ["list"])

    def test_bingux_config_keys_are_inventoried(self):
        keys = {entry["key"]: entry for entry in entries("config-keys.json")}
        for key in ("window-management", "window-rules", "layer-shell", "keybindings", "workspaces"):
            self.assertIn(key, keys)
        self.assertEqual(keys["cursor"]["runtime_override_keys"], ["theme", "size"])
        self.assertFalse(keys["window-rules"]["runtime_overridable"])


if __name__ == "__main__":
    unittest.main()
