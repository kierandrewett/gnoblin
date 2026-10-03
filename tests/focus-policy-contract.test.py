#!/usr/bin/env python3
"""Keep the public, Lua, and Mutter focus-policy defaults aligned."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class FocusPolicyContractTests(unittest.TestCase):
    def test_lua_runtime_defaults_new_window_focus_to_strict(self):
        source = (ROOT / "src/config/gnoblin-lua.c").read_text()
        self.assertIn('{"focus_new_windows", "strict", FALSE, 0, FOCUS_STRING}', source)
        self.assertIn('{"focus-new-windows", "strict"}', source)

    def test_mutter_uses_strict_for_omitted_settings_and_initial_state(self):
        preferences = (ROOT / "patches/mutter/70-config-preferences/0001-native-gnoblin-preferences.patch").read_text()
        initial = (ROOT / "patches/mutter/99-typed-window-api/0060-default-strict-new-window-focus.patch").read_text()
        self.assertIn(
            'APPLY_ENUM ("focus-new-windows", focus_new_windows, new_window_modes, '
            "G_DESKTOP_FOCUS_NEW_WINDOWS_STRICT, META_PREF_FOCUS_NEW_WINDOWS);",
            preferences,
        )
        self.assertIn(
            "+static GDesktopFocusNewWindows focus_new_windows = G_DESKTOP_FOCUS_NEW_WINDOWS_STRICT;",
            initial,
        )

    def test_user_documentation_names_strict_as_the_default(self):
        reference = (ROOT / "docs/config/configure/window_management.md").read_text()
        guide = (ROOT / "docs/focus-transfer.md").read_text()
        self.assertIn('`"strict"`          | Selects Mutter\'s new-window focus policy.', reference)
        self.assertIn('`"strict"` | Selects Mutter\'s focus policy for new windows.', guide)
        self.assertIn('"strict"` is the default', reference)


if __name__ == "__main__":
    unittest.main()
