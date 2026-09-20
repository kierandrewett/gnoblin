#!/usr/bin/env python3
"""Exercise distro selection and command planning without changing host packages."""

import importlib.util
from pathlib import Path
import shlex
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent
spec = importlib.util.spec_from_file_location("check_build_deps", ROOT / "scripts/check-build-deps.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)


def shell(code, *args):
    return subprocess.run(
        ["bash", "-eu", "-c", "source scripts/build-deps.sh; " + code, "test", *args],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )


class BuildDependencies(unittest.TestCase):
    def test_release_and_derivative_detection(self):
        for distro, like, expected in (
            ("fedora", "", "fedora"),
            ("arch", "", "arch"),
            ("cachyos", "arch", "arch"),
            ("ubuntu", "debian", "debian"),
            ("linuxmint", "ubuntu debian", "debian"),
            ("debian", "", "debian"),
            ("opensuse-tumbleweed", "opensuse suse", "opensuse"),
            ("opensuse-leap", "suse", "opensuse"),
        ):
            with self.subTest(distro=distro):
                result = shell('build_distro_family "$1" "$2"', distro, like)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), expected)
        self.assertNotEqual(shell('build_distro_family alpine ""').returncode, 0)

    def test_dry_run_commands(self):
        for family, manager in (
            ("fedora", "dnf"),
            ("arch", "pacman"),
            ("debian", "apt-get"),
            ("opensuse", "zypper"),
        ):
            with self.subTest(family=family):
                result = shell('install_build_dependencies "$1" true true', family)
                self.assertEqual(result.returncode, 0, result.stderr)
                commands = [shlex.split(line) for line in result.stdout.splitlines()]
                self.assertTrue(all(manager in command for command in commands))
                if family == "debian":
                    self.assertIn("update", commands[0])
                    self.assertIn("DEBIAN_FRONTEND=noninteractive", commands[1])
                    self.assertIn("liblua5.4-dev", commands[1])
                    self.assertIn("libsysprof-capture-4-dev", commands[1])
                if family == "opensuse":
                    self.assertIn("refresh", commands[0])
                    self.assertIn("remove", commands[1])
                    self.assertIn("busybox-gawk", commands[1])
                    self.assertIn("--non-interactive", commands[2])
                    self.assertIn("gawk", commands[2])
                    self.assertIn("pkgconfig(wayland-server)", commands[2])
                if family == "arch":
                    self.assertIn("-S", commands[0])
                    self.assertIn("glycin", commands[0])
                    self.assertNotIn("libglycin", commands[0])
                    self.assertNotIn("-Syu", commands[0])
                    self.assertNotIn("-Sy", commands[0])
                if family == "fedora":
                    self.assertIn("expat-devel", commands[0])
                    self.assertNotIn("copr", result.stdout)
                    self.assertNotIn("builddep", result.stdout)

    def test_interactive_setup_does_not_accept_prompts(self):
        for family in ("fedora", "arch", "debian", "opensuse"):
            result = shell('install_build_dependencies "$1" false true', family)
            self.assertEqual(result.returncode, 0, result.stderr)
            for flag in ("--non-interactive", "--noconfirm", "DEBIAN_FRONTEND", " -y "):
                self.assertNotIn(flag, result.stdout)

    def test_wrapper_options(self):
        result = subprocess.run(
            [str(ROOT / "build.sh"), "--dry-run", "--no-deps", "--yes"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("check-build-deps.py", result.stdout)
        self.assertIn("just build-local", result.stdout)
        for args in (("--no-deps", "--deps-only"), ("--unknown",)):
            result = subprocess.run([str(ROOT / "build.sh"), *args], capture_output=True)
            self.assertEqual(result.returncode, 2)

    def test_default_build_never_invokes_a_package_manager(self):
        result = subprocess.run(
            [str(ROOT / "build.sh"), "--dry-run", "--yes"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        for command in ("sudo", "dnf", "pacman", "apt-get", "zypper"):
            self.assertNotIn(command, result.stdout)
        self.assertIn("just build-local", result.stdout)

    def test_constraints_come_from_meson_not_a_second_version_list(self):
        source = """
glib_req = '>= 2.86.0'
wayland_req = '>= 1.26'
schemas_req = '>= 51.rc'
glib_dep = dependency('glib-2.0', version: glib_req)
wayland_dep = dependency('wayland-server',
    version: wayland_req)
schemas_dep = dependency('gsettings-desktop-schemas', version: schemas_req)
private_dep = dependency(private_name, version: glib_req)
"""
        self.assertEqual(
            list(checker.requirements(source)),
            [("glib-2.0", ">= 2.86.0"), ("wayland-server", ">= 1.26")],
        )


if __name__ == "__main__":
    unittest.main()
