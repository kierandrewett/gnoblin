#!/usr/bin/env python3
"""Keep Lua titlebar layout wired to Mutter's preference handler."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class WindowButtonLayoutContractTests(unittest.TestCase):
    def test_mutter_applies_lua_layout_and_owns_the_gsettings_key(self):
        patch = (ROOT / "patches/mutter/99-typed-window-api/0090-configure-titlebar-button-layout.patch").read_text()
        self.assertIn('g_variant_lookup_value (preferences, "button-layout", G_VARIANT_TYPE_VARDICT)', patch)
        self.assertIn("button_layout_handler (layout_value, &ignored, NULL);", patch)
        self.assertIn('g_strdup ("minimize,maximize,close")', patch)
        self.assertIn('"resize-with-right-button", "disable-workarounds", "button-layout", NULL', patch)

    def test_shared_config_validation_and_runtime_case_cover_the_shape(self):
        validator = (ROOT / "src/config/gnoblin-config.c").read_text()
        runtime_test = (ROOT / "tests/lua-config-test.c").read_text()
        self.assertIn('g_str_equal(name, "button-layout")', validator)
        self.assertIn('g_variant_is_of_type(side, G_VARIANT_TYPE("av"))', validator)
        self.assertIn("button_layout = {", runtime_test)
        self.assertIn("left = {'menu'}", runtime_test)
        self.assertIn("button_layout = {right = {'unknown'}}", runtime_test)


if __name__ == "__main__":
    unittest.main()
