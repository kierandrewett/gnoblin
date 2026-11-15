#!/usr/bin/env python3
"""Guard the adapter support-matrix page against drift from its two inputs.

design/adapter-support-matrices.md is generated from the recorded reference session (tests/parity/reference-session/)
and from every adapter's support-matrix.json. This test regenerates nothing: it asks the script whether the stored page
is current, and spot-checks that the recording still holds the names the page's claims rest on.
"""

from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts/reference-session-matrix.py"
RECORD = ROOT / "tests/parity/reference-session"


class ReferenceSessionMatrixTests(unittest.TestCase):
    def test_stored_page_is_current(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "--check"], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_recording_names_what_mutter_owns(self):
        names = (RECORD / "dbus-names.txt").read_text(encoding="utf-8")
        for name in (
            "org.gnome.Mutter.DisplayConfig",
            "org.gnome.Mutter.ScreenCast",
            "org.gnome.Mutter.RemoteDesktop",
            "org.gnome.Mutter.InputCapture",
            "org.freedesktop.a11y.Manager",
        ):
            self.assertIn(name, names)

    def test_recording_says_which_mutter_it_ran(self):
        run = (RECORD / "run.txt").read_text(encoding="utf-8")
        self.assertIn("mutter 51.0", run)

    def test_the_session_objects_were_introspected_while_alive(self):
        sessions = (RECORD / "dbus/sessions.xml").read_text(encoding="utf-8")
        for interface in (
            "org.gnome.Mutter.ScreenCast.Session",
            "org.gnome.Mutter.RemoteDesktop.Session",
            "org.gnome.Mutter.InputCapture.Session",
        ):
            self.assertIn(interface, sessions)


if __name__ == "__main__":
    unittest.main()
