#!/usr/bin/env python3
"""Local tests for gnoblinctl's non-D-Bus configuration-fragment workflow."""
from importlib.machinery import SourceFileLoader
import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_loader(
    "gnoblinctl", SourceFileLoader("gnoblinctl", str(ROOT / "src/tools/gnoblinctl")))
GNOBLINCTL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GNOBLINCTL)


class ConfigFragmentTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="gnoblinctl-test-")
        self.config_home = Path(self.directory.name) / "config"
        self.fragment = Path(self.directory.name) / "bingux.toml"
        self.fragment.write_text("[shell]\nosd = false\n", encoding="utf-8")
        self.environment = patch.dict(os.environ, {
            "XDG_CONFIG_HOME": str(self.config_home),
            "GNOBLIN_CONFIG": "",
        }, clear=False)
        self.environment.start()

    def tearDown(self):
        self.environment.stop()
        self.directory.cleanup()

    def test_load_is_atomic_and_idempotent(self):
        with patch.object(GNOBLINCTL, "dbus") as call:
            first = GNOBLINCTL.load_config_fragment(self.fragment)
            second = GNOBLINCTL.load_config_fragment(self.fragment)
        config = self.config_home / "gnoblin" / "gnoblin.toml"
        self.assertEqual(first["reload"], "applied")
        self.assertEqual(second["reload"], "applied")
        self.assertTrue(first["changed"])
        self.assertFalse(second["changed"])
        self.assertEqual(config.read_text(encoding="utf-8").count("include ="), 1)
        self.assertEqual(call.call_count, 2)

    def test_load_extends_an_existing_include_array(self):
        config = self.config_home / "gnoblin" / "gnoblin.toml"
        config.parent.mkdir(parents=True)
        existing = self.directory.name + "/existing.toml"
        config.write_text(f'include = ["{existing}"]\n[shell]\nosd = true\n', encoding="utf-8")
        with patch.object(GNOBLINCTL, "dbus"):
            GNOBLINCTL.load_config_fragment(self.fragment)
        contents = config.read_text(encoding="utf-8")
        self.assertIn(f'include = ["{existing}", "{self.fragment.resolve()}"]', contents)
        self.assertEqual(contents.count("include ="), 1)

    def test_validation_error_restores_previous_config(self):
        config = self.config_home / "gnoblin" / "gnoblin.toml"
        config.parent.mkdir(parents=True)
        original = "[shell]\nosd = true\n"
        config.write_text(original, encoding="utf-8")
        failure = GNOBLINCTL.CommandError("Call failed: invalid corners.smoothing")
        with patch.object(GNOBLINCTL, "dbus", side_effect=failure), self.assertRaisesRegex(
                GNOBLINCTL.CommandError, "configuration was restored: Call failed: invalid corners.smoothing"):
            GNOBLINCTL.load_config_fragment(self.fragment)
        self.assertEqual(config.read_text(encoding="utf-8"), original)

    def test_legacy_config_gets_migration_error(self):
        config = self.config_home / "gnoblin" / "gnoblin.conf"
        config.parent.mkdir(parents=True)
        config.write_text("[shell]\nosd = false\n", encoding="utf-8")
        with self.assertRaisesRegex(GNOBLINCTL.CommandError, "migrate it to gnoblin.toml"):
            GNOBLINCTL.load_config_fragment(self.fragment)

    def test_unload_is_idempotent_and_allows_missing_provider(self):
        config = self.config_home / "gnoblin" / "gnoblin.toml"
        config.parent.mkdir(parents=True)
        include = f'include = {GNOBLINCTL.json.dumps(str(self.fragment.resolve()))}\n'
        config.write_text(include + "\n[shell]\nosd = true\n", encoding="utf-8")
        with patch.object(GNOBLINCTL, "dbus") as call:
            result = GNOBLINCTL.unload_config_fragment(self.fragment)
            again = GNOBLINCTL.unload_config_fragment(self.fragment)
        self.assertTrue(result["changed"])
        self.assertFalse(again["changed"])
        self.assertNotIn("include =", config.read_text(encoding="utf-8"))
        self.assertEqual(call.call_count, 1)

        config.write_text(include, encoding="utf-8")
        self.fragment.unlink()
        with patch.object(GNOBLINCTL, "dbus", side_effect=GNOBLINCTL.CommandError("Failed to connect to bus: No medium found")):
            pending = GNOBLINCTL.unload_config_fragment(self.fragment)
        self.assertEqual(pending["reload"], "pending")
        self.assertTrue(pending["changed"])
        self.assertFalse(config.read_text(encoding="utf-8").count("include ="))


if __name__ == "__main__":
    unittest.main()
