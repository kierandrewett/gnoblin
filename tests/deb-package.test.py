#!/usr/bin/env python3
"""Check that Debian packages cannot export files over stock GNOME."""

import importlib.util
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("package_deb", ROOT / "scripts/package-deb.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class PackageLayoutTests(unittest.TestCase):
    def test_default_shortcut_command_dependencies_are_declared(self):
        self.assertTrue({"wireplumber", "playerctl", "brightnessctl", "libglib2.0-bin"}
                        .issubset(set(package.SERVICES)))

    def test_package_version_keeps_gnome_compatibility_and_gnoblin_semver(self):
        self.assertEqual(
            package.package_version("51.0", "0.1.0", "1", "debian", "13"),
            "51.0+gnoblin0.1.0-1~debian13",
        )

    def test_runtime_exports_only_gnoblin_names(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            prefix = root / "runtime"
            for name in (
                "bin/gnome-shell",
                "bin/gnoblin-session",
                "bin/gnoblinctl",
                "share/gnoblin/version.json",
                "share/gnoblin/init.lua.example",
                "libexec/gnoblin-seed-config",
                "share/gnome-shell/gnome-shell-theme.gresource",
                "lib/systemd/user/org.gnome.Shell@wayland.service",
                *package.PUBLIC_FILES,
            ):
                path = prefix / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            (prefix / "deps/lib64").mkdir(parents=True)
            stage = root / "package"
            private = package.stage_runtime(prefix, stage)
            package.check_layout(stage)
            self.assertTrue((private / "lib/systemd/user/org.gnome.Shell@wayland.service").exists())
            self.assertFalse((stage / "usr/lib/systemd/user/org.gnome.Shell@wayland.service").exists())
            self.assertFalse((stage / "usr/bin/gnome-shell").exists())
            self.assertEqual((stage / "usr/bin/gnoblinctl").readlink(), Path("../lib/gnoblin/bin/gnoblinctl"))
            self.assertTrue((private / "share/gnoblin/version.json").is_file())
            self.assertTrue((private / "libexec/gnoblin-seed-config").is_file())
            self.assertEqual((stage / "usr/share/gnoblin/init.lua.example").read_text(), "fixture")
            self.assertFalse((stage / "usr/share/gnoblin/version.json").exists())

    def test_shared_gnome_files_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            stage = Path(directory)
            path = stage / "usr/share/glib-2.0/schemas/org.gnome.shell.gschema.xml"
            path.parent.mkdir(parents=True)
            path.touch()
            with self.assertRaisesRegex(RuntimeError, "outside Gnoblin"):
                package.check_layout(stage)

    def test_incomplete_runtime_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "Incomplete private runtime"):
                package.stage_runtime(Path(directory), Path(directory) / "stage")


if __name__ == "__main__":
    unittest.main()
