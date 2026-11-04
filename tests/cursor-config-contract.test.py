#!/usr/bin/env python3
"""Keep compositor cursor preferences wired to Gnoblin Lua configuration."""

import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

from _sources import control_source  # noqa: E402
import patch_series  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]
CONTROL = control_source()
MUTTER_PATCH = patch_series.find("mutter", "configure-cursor-theme-and-size-from-lua").read_text()


class CursorConfigContractTest(unittest.TestCase):
    def test_reload_applies_lua_cursor_settings_and_documented_defaults(self):
        start = CONTROL.index("void native_apply_runtime_preferences(")
        end = CONTROL.index("void native_settings_changed(", start)
        implementation = CONTROL[start:end]

        self.assertIn('g_variant_lookup_value(config, "cursor", G_VARIANT_TYPE_VARDICT)', implementation)
        self.assertIn('const char* cursor_theme = "default";', implementation)
        self.assertIn("gint64 cursor_size = 24;", implementation)
        self.assertIn('g_variant_lookup(cursor_preferences, "theme", "&s", &cursor_theme)', implementation)
        self.assertIn('g_variant_lookup(cursor_preferences, "size", "x", &cursor_size)', implementation)
        self.assertIn("meta_prefs_set_gnoblin_cursor_config(cursor_theme, (int)cursor_size)", implementation)

    def test_cursor_gsettings_changes_do_not_override_lua_in_gnoblin(self):
        self.assertIn("if (is_gnoblin_session () && settings == SETTINGS (SCHEMA_INTERFACE)", MUTTER_PATCH)
        self.assertIn("KEY_GNOME_CURSOR_THEME", MUTTER_PATCH)
        self.assertIn("KEY_GNOME_CURSOR_SIZE", MUTTER_PATCH)


if __name__ == "__main__":
    unittest.main()
