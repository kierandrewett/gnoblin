#!/usr/bin/env python3
"""Check pinned Meson requirement extraction and the source-build CLI."""

import importlib.util
import json
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("check_build_deps", ROOT / "scripts/check-build-deps.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


class BuildRequirements(unittest.TestCase):
    def test_dependency_check_applies_pipewire_compatibility_floor(self):
        source = """
gsettings_desktop_schemas_req = '>= 51.rc'
wayland_server_req = '>= 1.26'
wayland_server_dep = dependency('wayland-server', version: wayland_server_req)
libinput_req = '>= 1.31.0'
libinput_dep = dependency('libinput', version: libinput_req)
libpipewire_req = '>= 1.6.0'
libpipewire_dep = dependency('libpipewire-0.3', version: libpipewire_req)
"""

        patched = checker.apply_dependency_version_patches(source, "mutter", "meson.build")

        self.assertEqual(
            list(checker.requirements(patched)),
            [
                ("wayland-server", ">= 1.25"),
                ("libinput", ">= 1.30.0"),
                ("libpipewire-0.3", ">= 1.4.11"),
            ],
        )

    def test_requirement_extraction_includes_required_unversioned_dependencies(self):
        source = """
glib_req = '>= 2.86.0'
wayland_req = '>= 1.25'
schemas_req = '>= 49.1'
glib_dep = dependency('glib-2.0', version: glib_req)
wayland_dep = dependency('wayland-server',
    version: wayland_req)
schemas_dep = dependency('gsettings-desktop-schemas', version: schemas_req)
private_dep = dependency(private_name, version: glib_req)
unversioned_dep = dependency('gnome-desktop-4')
optional_dep = dependency('optional', required: false)
option_dep = dependency('option-dependent', required: get_option('feature'))
subproject('bundled-lib')
bundled_dep = dependency('bundled-lib')
"""
        self.assertEqual(
            list(checker.requirements(source)),
            [
                ("glib-2.0", ">= 2.86.0"),
                ("wayland-server", ">= 1.25"),
                ("gnome-desktop-4", None),
                ("option-dependent", None),
            ],
        )

    def test_requirement_extraction_skips_bundled_subprojects_from_root_file(self):
        self.assertEqual(
            list(checker.requirements("dependency('bundled-lib')", bundled={"bundled-lib"})),
            [],
        )

    def test_pinned_meson_sources_produce_versioned_requirements(self):
        versions = json.loads((ROOT / "gnome-versions.json").read_text())
        projects = {
            "mutter": "meson.build",
            "xdg-desktop-portal-gnome": "src/meson.build",
        }
        for project, build_file in projects.items():
            with self.subTest(project=project):
                commit = versions["components"][project]["commit"]
                result = subprocess.run(
                    ["git", "-C", str(ROOT / "subprojects" / project), "show", f"{commit}:{build_file}"],
                    capture_output=True,
                    text=True,
                )
                if result.returncode:
                    self.skipTest(f"pinned {project} source is not present in this checkout")
                requirements = list(checker.requirements(result.stdout))
                self.assertTrue(requirements)
                self.assertTrue(all(module for module, _version in requirements))

    def test_pinned_portal_sources_include_required_gnome_desktop_modules(self):
        versions = json.loads((ROOT / "gnome-versions.json").read_text())
        revision = versions["components"]["xdg-desktop-portal-gnome"]["commit"]
        result = subprocess.run(
            [
                "git",
                "-C",
                str(ROOT / "subprojects" / "xdg-desktop-portal-gnome"),
                "show",
                f"{revision}:src/meson.build",
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode:
            self.skipTest("pinned portal source is not present in this checkout")
        modules = {module for module, _version in checker.requirements(result.stdout, include_schemas=True)}
        self.assertIn("gnome-desktop-4", modules)
        self.assertIn("gnome-bg-4", modules)

    def test_pinned_portal_wrap_includes_gtk4_minimum(self):
        self.assertEqual(
            checker.WRAPPED_PROJECT_REQUIREMENTS["xdg-desktop-portal-gnome"],
            (("gtk4", ">= 4.20.0"),),
        )

    def test_current_build_dry_run_reports_options_without_provisioning(self):
        with tempfile.TemporaryDirectory() as directory:
            output_dir = Path(directory) / "prefix"
            result = subprocess.run(
                [
                    str(ROOT / "build.sh"),
                    "--dry-run",
                    "--prefix",
                    str(output_dir),
                    "--jobs",
                    "3",
                    "--without-xwayland",
                    "--target",
                    "gnoblin",
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertRegex(result.stdout, r"Output prefix:\s+" + re.escape(str(output_dir)))
            self.assertRegex(result.stdout, r"Ninja target:\s+gnoblin\n")
            self.assertRegex(result.stdout, r"XWayland:\s+false")
            # A chosen target has its own steps, so the plan is not printed for it.
            self.assertIn("The steps depend on the target gnoblin", result.stdout)
            self.assertNotIn("Steps, in order", result.stdout)
            self.assertFalse(output_dir.exists())
            for obsolete in ("sudo", "dnf", "pacman", "apt-get", "zypper", "--yes", "--no-deps"):
                self.assertNotIn(obsolete, result.stdout)
            core_result = subprocess.run(
                [str(ROOT / "build.sh"), "--dry-run", "--without-portal"],
                capture_output=True,
                text=True,
            )
            self.assertEqual(core_result.returncode, 0, core_result.stderr)
            self.assertRegex(core_result.stdout, r"Ninja target:\s+gnoblin-public-entries")
            self.assertRegex(core_result.stdout, r"Portal backend:\s+omitted")
            self.assertNotIn("Build the portal backend", core_result.stdout)
            self.assertNotIn("portal backend (reset", core_result.stdout)
            self.assertIn("subprojects/mutter (reset", core_result.stdout)
            portal_result = subprocess.run(
                [str(ROOT / "build.sh"), "--dry-run"],
                capture_output=True,
                text=True,
            )
            self.assertEqual(portal_result.returncode, 0, portal_result.stderr)
            self.assertRegex(portal_result.stdout, r"Ninja target:\s+standalone-session")
            # The default build prints the plan: its steps, what it writes and what it leaves alone.
            for heading in ("Steps, in order", "Writes", "Does not touch"):
                self.assertIn(heading, portal_result.stdout)
            self.assertRegex(portal_result.stdout, r"\d+ patches in patches/mutter")
            self.assertRegex(portal_result.stdout, r"Portal backend:\s+included")
            self.assertIn("Build the portal backend", portal_result.stdout)
            system_result = subprocess.run(
                [str(ROOT / "build.sh"), "--dry-run", "--layout", "system", "--destdir", "/tmp/stage-for-the-plan"],
                capture_output=True,
                text=True,
            )
            self.assertEqual(system_result.returncode, 0, system_result.stderr)
            self.assertIn("Publish the public entries under /usr", system_result.stdout)
            self.assertIn("/tmp/stage-for-the-plan", system_result.stdout)
            self.assertNotIn("/usr and other system paths", system_result.stdout)

    def test_build_cli_rejects_invalid_or_misplaced_options(self):
        for args in (
            ("--unknown",),
            ("--jobs", "0"),
            ("--terminal", "foot"),
            ("--preview", "--dry-run"),
            ("--with-portal", "--preview"),
            ("--with-portal", "--target", "gnoblin"),
            ("--register-session", "--dry-run"),
        ):
            with self.subTest(args=args):
                result = subprocess.run([str(ROOT / "build.sh"), *args], capture_output=True, text=True)
                self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main()
