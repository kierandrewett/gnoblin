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
SPEC = importlib.util.spec_from_loader("gnoblinctl", SourceFileLoader("gnoblinctl", str(ROOT / "src/tools/gnoblinctl")))
GNOBLINCTL = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(GNOBLINCTL)


class ConfigFragmentTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="gnoblinctl-test-")
        self.config_home = Path(self.directory.name) / "config"
        self.environment = patch.dict(
            os.environ,
            {
                "XDG_CONFIG_HOME": str(self.config_home),
                "GNOBLIN_CONFIG": "",
            },
            clear=False,
        )
        self.environment.start()

    def tearDown(self):
        self.environment.stop()
        self.directory.cleanup()

    def test_fresh_configuration_uses_lua_init_without_creating_it(self):
        config = self.config_home / "gnoblin" / "init.lua"
        self.assertEqual(GNOBLINCTL.user_config_path(), config)
        self.assertFalse(config.exists())

    def test_default_configuration_uses_existing_legacy_file(self):
        directory = self.config_home / "gnoblin"
        directory.mkdir(parents=True)
        (directory / "gnoblin.conf").touch()
        (directory / "gnoblin.toml").touch()
        self.assertEqual(GNOBLINCTL.user_config_path(), directory / "gnoblin.toml")

    def test_config_commands_show_the_path_and_reload(self):
        config = Path(self.directory.name) / "custom.lua"
        cli = GNOBLINCTL.parser()
        path_args = cli.parse_args(["config", "path"])
        with patch.dict(os.environ, {"GNOBLIN_CONFIG": str(config)}):
            self.assertEqual(GNOBLINCTL.dispatch(path_args, cli), str(config))
        reload_args = cli.parse_args(["config", "reload"])
        reload_args.timeout = 5
        reload_args.socket = ""
        with patch.object(GNOBLINCTL, "dbus") as call:
            result = GNOBLINCTL.dispatch(reload_args, cli)
        self.assertEqual(result, {"ok": True, "action": "config reload"})
        call.assert_called_once_with("ReloadConfig", "", [], "org.gnoblin.Shell", 5)

    def test_separate_frame_reload_is_not_a_command(self):
        cli = GNOBLINCTL.parser()
        with patch("sys.stderr"), self.assertRaises(SystemExit):
            cli.parse_args(["frame", "reload"])


if __name__ == "__main__":
    unittest.main()
