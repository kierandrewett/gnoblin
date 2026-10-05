#!/usr/bin/env python3
"""Keep compositor cursor preferences wired to Gnoblin Lua configuration."""

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CONTROL = (ROOT / "src/native-control/gnoblin-native-control.c").read_text()
MUTTER_PATCH = (
    ROOT / "patches/mutter/70-config-preferences/0009-configure-cursor-theme-and-size-from-lua.patch"
).read_text()


class CursorConfigContractTest(unittest.TestCase):
    def test_reload_applies_lua_cursor_settings_and_documented_defaults(self):
        start = CONTROL.index("static void native_settings_changed(")
        end = CONTROL.index("static gboolean native_touchpad_action_supported(", start)
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
