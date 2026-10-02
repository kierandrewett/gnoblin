#!/usr/bin/env python3
"""Keep Hyprcursor gated by Gnoblin's supervised runtime state."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class HyprcursorSessionPredicateTests(unittest.TestCase):
    def test_loader_uses_the_native_supervisor_predicate(self):
        source = (ROOT / "src/cursor/gnoblin-hyprcursor.cpp").read_text()
        fixture = (ROOT / "src/cursor/test-hyprcursor.cpp").read_text()

        self.assertIn('#include "core/gnoblin-native-control.h"', source)
        self.assertIn("!gnoblin_native_control_is_session(nullptr)", source)
        self.assertNotIn("GNOME_SHELL_SESSION_MODE", source)
        self.assertIn('g_unsetenv("GNOME_SHELL_SESSION_MODE")', fixture)
        self.assertIn("supervisor_session = false", fixture)
        self.assertIn('g_setenv("GNOME_SHELL_SESSION_MODE", "gnoblin", TRUE)', fixture)


if __name__ == "__main__":
    unittest.main()
