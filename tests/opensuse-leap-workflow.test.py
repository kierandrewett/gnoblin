#!/usr/bin/env python3
"""Keep the Leap release gate distinct from the rolling Tumbleweed gate."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]


class OpenSUSELeapWorkflowTests(unittest.TestCase):
    def test_each_supported_leap_base_builds_the_private_rpm_chain(self):
        workflow = (ROOT / ".github/workflows/opensuse-leap.yml").read_text()

        self.assertIn("workflow_call:", workflow)
        self.assertIn("registry.opensuse.org/opensuse/leap:${{ matrix.version }}", workflow)
        self.assertIn('version: "15.6"', workflow)
        self.assertIn('version: "16.0"', workflow)
        self.assertIn("bash packaging/opensuse/build-chain.sh", workflow)
        self.assertIn("scripts/check-rpm-isolation.py", workflow)
        self.assertIn("opensuse-leap-15.6-rpms", workflow)
        self.assertIn("opensuse-leap-16.0-rpms", workflow)

    def test_each_leap_build_is_installed_beside_and_removed_from_stock_gnome(self):
        workflow = (ROOT / ".github/workflows/opensuse-leap.yml").read_text()
        coinstall = workflow.split("\n  coinstall:\n", 1)[1]

        self.assertIn("needs: build", coinstall)
        self.assertIn("gdm gnome-session gnome-shell mutter xdg-desktop-portal-gnome", coinstall)
        self.assertIn("rpm -V gnome-session gnome-shell mutter", coinstall)
        self.assertGreaterEqual(coinstall.count("rpm -V gnome-session gnome-shell mutter"), 2)
        self.assertIn("zypper --non-interactive remove", coinstall)
        self.assertIn("! test -e /usr/share/wayland-sessions/gnoblin.desktop", coinstall)
        self.assertIn("! test -e /usr/lib/systemd/user/org.gnoblin.Shell.target", coinstall)


if __name__ == "__main__":
    unittest.main()
