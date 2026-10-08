#!/usr/bin/env python3
"""Keep the public, Lua, and Mutter focus-policy defaults aligned."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class FocusPolicyContractTests(unittest.TestCase):
    def test_lua_runtime_defaults_new_window_focus_to_strict(self):
        source = (ROOT / "src/config/gnoblin-lua.c").read_text()
        self.assertIn('{"focus_new_windows", "prevent", FALSE, 0, FOCUS_STRING}', source)
        self.assertIn('{"focus-new-windows", "prevent"}', source)

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

    def test_user_documentation_names_prevent_as_the_default(self):
        reference = (ROOT / "docs/config/configure/window_management.md").read_text()
        guide = (ROOT / "docs/focus-transfer.md").read_text()
        focus_row = next(line for line in reference.splitlines() if line.startswith("| `focus_new_windows`"))
        focus_cells = [cell.strip() for cell in focus_row.strip("|").split("|")]
        self.assertEqual(focus_cells[2], '`"prevent"`')
        self.assertEqual(focus_cells[3], "Selects Mutter's new-window focus policy.")
        guide_row = next(line for line in guide.splitlines() if line.startswith("| `focus_new_windows`"))
        guide_cells = [cell.strip() for cell in guide_row.strip("|").split("|")]
        self.assertEqual(guide_cells[1], '`"allow"`, `"prevent"`')
        self.assertEqual(guide_cells[2], '`"allow"`')
        self.assertEqual(guide_cells[3], "Selects Mutter's focus policy for new windows.")
        self.assertIn("It is the compositor's value when the", reference)


if __name__ == "__main__":
    unittest.main()
