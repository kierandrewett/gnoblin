#!/usr/bin/env python3
"""Static boundaries for the openSUSE Tumbleweed package adapter."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "packaging" / "opensuse"


class OpenSUSEPackagingTests(unittest.TestCase):
    def test_private_runtime_and_tumbleweed_capabilities(self):
        for name in ("mutter.spec", "gnome-shell.spec"):
            content = (SPECS / name).read_text()
            self.assertIn("%global _prefix /usr/lib/gnoblin", content)
            self.assertIn("%global __provides_exclude_from ^%{_prefix}/.*$", content)
            self.assertIn("pkgconfig(", content)
            self.assertNotIn("mesa-libEGL-devel", content)

    def test_session_uses_host_discovery_paths_and_gnoblin_names(self):
        shell = (SPECS / "gnome-shell.spec").read_text()
        self.assertIn("/usr/share/wayland-sessions/gnoblin.desktop", shell)
        self.assertIn("/usr/lib/systemd/user/org.gnoblin.Shell.target", shell)
        self.assertIn("/usr/lib/systemd/user/gnome-session@gnoblin.target.d/", shell)
        self.assertNotIn("org.gnome.Shell@wayland.service", shell)

    def test_private_stack_is_not_required_for_repository_probe(self):
        for name in ("mutter.spec", "gnome-shell.spec"):
            content = (SPECS / name).read_text()
            self.assertIn("%bcond_with gnoblin_stack", content)
            self.assertIn("%if %{with gnoblin_stack}", content)

    def test_meta_uses_tumbleweed_runtime_library_names(self):
        meta = (SPECS / "gnoblin.spec").read_text()
        self.assertIn("Requires:       libinput10 >= 1.30", meta)
        self.assertIn("Requires:       libwayland-client0 >= 1.26", meta)

    def test_check_script_keeps_the_probe_non_installing(self):
        check = (SPECS / "check-buildrequires.sh").read_text()
        self.assertIn("--without gnoblin_stack", check)
        self.assertIn("install --dry-run --no-recommends", check)


if __name__ == "__main__":
    unittest.main()
