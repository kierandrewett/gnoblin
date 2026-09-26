#!/usr/bin/env python3
"""Check that Debian packages cannot export files over stock GNOME."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("package_deb", ROOT / "scripts/package-deb.py")
package = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package)


class PackageLayoutTests(unittest.TestCase):
    def test_modern_deb_provisioning_keeps_host_gtk4_development_files(self):
        provision = (ROOT / "scripts/provision-deb-container.sh").read_text()
        dependencies = (ROOT / "scripts/build-deps.sh").read_text()
        self.assertIn("legacy_private_gtk=false", provision)
        self.assertIn("private_deb_addons=true", provision)
        self.assertIn(
            'install_build_dependencies debian true false "$legacy_private_gtk" "$private_deb_addons"', provision
        )
        filtered = dependencies.split('if "$legacy_private_gtk"; then', 1)[1].split("fi", 1)[0]
        self.assertIn("libgtk-4-dev", filtered)

    def test_private_deb_addons_do_not_require_missing_host_glycin_or_hyprcursor(self):
        dependencies = (ROOT / "scripts/build-deps.sh").read_text()
        filtered = dependencies.split('if "$private_deb_addons"; then', 1)[1].split("fi", 1)[0]
        self.assertIn("libglycin-2-dev", filtered)
        self.assertIn("libhyprcursor-dev", filtered)
        self.assertNotIn("libgtk-4-dev", filtered)

    def test_legacy_compatibility_runtime_pins_private_pango_for_gtk4(self):
        recipes = json.loads((ROOT / "packaging/deb/compat-bootstrap.json").read_text())
        by_name = {recipe["name"]: recipe for recipe in recipes}
        pango = by_name["pango"]
        gtk4 = by_name["gtk4"]
        self.assertEqual(pango["version"], "1.50.14")
        self.assertEqual(pango["sha256"], "1d67f205bfc318c27a29cfdfb6828568df566795df0cb51d2189cde7f2d581e8")
        self.assertIn("pango.pc", pango["private_pkgconfig"])
        self.assertIn("pangocairo.pc", pango["private_pkgconfig"])
        self.assertIn("Pango-1.0.typelib", pango["private_typelibs"])
        self.assertIn("pango", gtk4["requires"])

    def test_legacy_compatibility_runtime_uses_modern_cbindgen(self):
        provision = (ROOT / "scripts/provision-deb-compat-container.sh").read_text()
        self.assertIn("cargo install cbindgen", provision)
        self.assertIn("--version 0.28.0", provision)

    def test_distributed_copyright_keeps_company_and_upstream_attribution(self):
        text = package.debian_copyright_text()
        self.assertIn("Copyright © 2026 Working Directory Ltd.", text)
        self.assertIn("GNOME-derived portions are Copyright ©", text)
        self.assertIn("Copyright: GNOME Project", text)

    def test_default_shortcut_command_dependencies_are_declared(self):
        self.assertTrue({"wireplumber", "playerctl", "brightnessctl", "libglib2.0-bin"}.issubset(set(package.SERVICES)))
        self.assertNotIn("gir1.2-gtk4layershell-1.0", package.SERVICES)

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
                "share/icons/Adwaita-Hyprcursor/hyprcursors/default.hlc",
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
            self.assertEqual(
                (private / "share/icons/Adwaita-Hyprcursor/hyprcursors/default.hlc").read_text(), "fixture"
            )
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
