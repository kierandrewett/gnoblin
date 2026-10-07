#!/usr/bin/env python3
"""Unit checks for RPM repository capability probe parsing."""

import importlib.util
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("probe_rpm_target", ROOT / "scripts/probe-rpm-target.py")
probe = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(probe)


class RpmTargetProbeTests(unittest.TestCase):
    def test_requirement_provenance_covers_runtime_contract_and_build_closure(self):
        self.assertEqual(len(probe.REQUIREMENTS), 11)
        self.assertNotIn("gjs", probe.REQUIREMENTS)
        self.assertNotIn("gcr4", probe.REQUIREMENTS)
        self.assertEqual(
            {name for name, item in probe.REQUIREMENTS.items() if item["declaredScope"] == "host-runtime-contract"},
            {"glib", "wayland", "libinput", "pipewire"},
        )
        self.assertEqual(
            {name for name, item in probe.REQUIREMENTS.items() if item["declaredScope"] == "build-closure"},
            {"gtk4", "girepository", "glycin", "libei", "libeis", "libdisplay-info"},
        )
        self.assertEqual(probe.REQUIREMENTS["wayland-protocols"]["declaredScope"], "development-package-contract")
        self.assertEqual(probe.REQUIREMENTS["wayland-protocols"]["declaredPackage"], "gnoblin")
        self.assertEqual(
            probe.REQUIREMENTS["wayland-protocols"]["floorSource"],
            "subprojects/mutter/meson.build:50,217-218",
        )
        self.assertEqual(probe.REQUIREMENTS["glib"]["floorSource"], "subprojects/mutter/meson.build:19")
        self.assertEqual(
            probe.REQUIREMENTS["girepository"]["capability"],
            "pkgconfig(gobject-introspection-1.0)",
        )
        for item in probe.REQUIREMENTS.values():
            self.assertRegex(item["floorSource"], r"^[^:]+:[0-9]+(?:-[0-9]+)?(?:,[0-9]+(?:-[0-9]+)?)?$")

    def test_rpm_specs_match_source_build_closure_floors(self):
        for component in ("girepository", "glycin", "libdisplay-info"):
            declarations = probe.REQUIREMENTS[component]["rpmSpecDeclarations"]
            self.assertEqual(
                {entry["minimum"] for entry in declarations},
                {probe.REQUIREMENTS[component]["minimum"]},
            )
            self.assertEqual({entry["note"] for entry in declarations}, {"matches-source-floor"})

    def test_treats_zypper_no_provider_status_as_a_missing_capability(self):
        original = probe.command

        def no_provider(*_):
            raise probe.subprocess.CalledProcessError(104, "zypper")

        probe.command = no_provider
        try:
            self.assertEqual(probe.zypper_candidates("pkgconfig(glycin-2)"), [])
        finally:
            probe.command = original

    def test_compares_required_upstream_floors(self):
        self.assertTrue(probe.version_at_least("2.88.3", "2.86.0"))
        self.assertTrue(probe.version_at_least("2.1.5", "2.0.beta.2"))
        self.assertTrue(probe.version_at_least("0.1.13", "0.1.13"))
        self.assertFalse(probe.version_at_least("2.80.4", "2.86.0"))
        self.assertFalse(probe.version_at_least("4.6.9", "4.14.0"))

    def test_parses_dnf_repoquery_output(self):
        original = probe.command
        probe.command = lambda *_: "glib2-devel|0|2.88.3|1.fc44\n"
        try:
            self.assertEqual(
                probe.dnf_candidates("pkgconfig(glib-2.0)"),
                [{"package": "glib2-devel", "epoch": "0", "version": "2.88.3", "release": "1.fc44"}],
            )
        finally:
            probe.command = original

    def test_parses_zypper_capability_output(self):
        original = probe.command
        probe.command = lambda *_: (
            "S  | Name       | Type    | Version    | Arch   | Repository\n"
            "---+------------+---------+------------+--------+-----------\n"
            "   | glib2-devel | package | 2.88.3-1.1 | x86_64 | Tumbleweed\n"
        )
        try:
            self.assertEqual(
                probe.zypper_candidates("pkgconfig(glib-2.0)"),
                [{"package": "glib2-devel", "epoch": "0", "version": "2.88.3-1.1", "release": ""}],
            )
        finally:
            probe.command = original


if __name__ == "__main__":
    unittest.main()
