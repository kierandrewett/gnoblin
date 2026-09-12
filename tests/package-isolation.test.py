#!/usr/bin/env python3
"""Regression checks for installation alongside an existing GNOME session."""

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("isolation", ROOT / "scripts/check-rpm-isolation.py")
isolation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(isolation)


class IsolationTests(unittest.TestCase):
    def test_local_registration_preserves_stock_units(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            prefix = base / "runtime"
            config = base / "config"
            units = config / "systemd/user"
            units.mkdir(parents=True)
            stock = units / "org.gnome.Shell@wayland.service"
            stock.write_text("stock GNOME unit\n")
            fake_bin = base / "bin"
            fake_bin.mkdir()
            systemctl = fake_bin / "systemctl"
            systemctl.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> "$TEST_SYSTEMCTL_LOG"\n')
            systemctl.chmod(0o755)
            env = {**os.environ, "XDG_CONFIG_HOME": str(config),
                   "GNOBLIN_LIBDIR": "lib64", "PATH": f"{fake_bin}:{os.environ['PATH']}",
                   "TEST_SYSTEMCTL_LOG": str(base / "calls")}
            subprocess.run(["bash", str(ROOT / "scripts/install-session.sh"), str(prefix)],
                           env=env, check=True, capture_output=True)
            subprocess.run(["bash", str(ROOT / "scripts/register-session.sh"), str(prefix)],
                           env=env, check=True, capture_output=True)
            self.assertEqual(stock.read_text(), "stock GNOME unit\n")
            self.assertEqual((units / "gnome-session@gnoblin.target.d/gnoblin.conf").resolve(),
                             prefix / "lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf")
            calls = (base / "calls").read_text()
            self.assertNotIn("org.gnome.Shell", calls)
            self.assertIn("org.gnoblin.Shell.target", calls)

    def test_rejects_stock_files_and_capabilities(self):
        for path in ("/usr/bin/gnome-shell", "/usr/lib64/libmutter-17.so.0",
                     "/usr/lib/systemd/user/org.gnome.Shell@wayland.service",
                     "/usr/share/glib-2.0/schemas/org.gnome.shell.gschema.xml"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                isolation.validate("gnoblin-shell", path, "", "", "")
        for capability in ("gnome-shell = 49.6", "mutter = 49.5",
                           "libmutter-17.so.0()(64bit)", "pkgconfig(libmutter-17) = 49.5"):
            with self.subTest(capability=capability), self.assertRaises(ValueError):
                isolation.validate("gnoblin-shell", "", capability, "", "")
        for name, conflicts, obsoletes in (("gnome-shell", "", ""),
                                           ("gnoblin-shell", "gnome-shell < 49", ""),
                                           ("gnoblin-shell", "", "mutter")):
            with self.subTest(name=name, conflicts=conflicts, obsoletes=obsoletes), self.assertRaises(ValueError):
                isolation.validate(name, "", "", conflicts, obsoletes)

    def test_accepts_private_runtime_and_session_entries(self):
        paths = "\n".join(sorted(isolation.PUBLIC_FILES)) + "\n/usr/lib/gnoblin/bin/gnome-shell"
        isolation.validate("gnoblin-shell", paths, "gnoblin-shell = 49.6", "", "")

    def test_source_install_rejects_shared_prefixes_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            alias = Path(directory) / "system"
            alias.symlink_to("/usr")
            for prefix in ("/", "/usr", "/usr/local", "/usr/../usr", str(alias)):
                result = subprocess.run(["bash", "-c", 'source "$1"; gnoblin_env_validate_install_prefix "$2"',
                                         "test", str(ROOT / "src/tools/gnoblin-env.sh"), prefix], capture_output=True)
                self.assertNotEqual(result.returncode, 0, prefix)
            result = subprocess.run(["bash", "-c", 'source "$1"; gnoblin_env_validate_install_prefix "$2"',
                                     "test", str(ROOT / "src/tools/gnoblin-env.sh"), directory], capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_service_failure_does_not_disable_gnome_extensions(self):
        unit = (ROOT / "src/data/session/systemd-user/org.gnoblin.Shell@wayland.service.in").read_text()
        self.assertNotIn("org.gnome.Shell-disable-extensions.service", unit)

    def test_session_install_removes_legacy_extension_manager_files(self):
        with tempfile.TemporaryDirectory() as directory:
            prefix = Path(directory) / "runtime"
            legacy_paths = (
                "bin/gnome-extensions",
                "bin/gnome-extensions-app",
                "share/applications/org.gnome.Extensions.desktop",
                "share/dbus-1/services/org.gnome.Extensions.service",
                "share/glib-2.0/schemas/org.gnome.Extensions.gschema.xml",
                "share/metainfo/org.gnome.Extensions.metainfo.xml",
                "share/gnome-shell/org.gnome.Extensions",
                "share/gnome-shell/org.gnome.Extensions.data.gresource",
                "share/gnome-shell/org.gnome.Extensions.src.gresource",
                "share/gnome-shell/org.gnome.Shell.Extensions",
                "share/gnome-shell/org.gnome.Shell.Extensions.src.gresource",
                "share/bash-completion/completions/gnome-extensions",
                "share/applications/org.gnome.Shell.Extensions.desktop",
                "share/dbus-1/services/org.gnome.Shell.Extensions.service",
                "lib/systemd/user/org.gnome.Shell-disable-extensions.service",
                "share/icons/hicolor/64x64/apps/org.gnome.Extensions.png",
                "share/icons/hicolor/64x64/apps/org.gnome.Shell.Extensions.png",
            )
            for relative_path in legacy_paths:
                path = prefix / relative_path
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("legacy extension manager file\n")

            subprocess.run(["bash", str(ROOT / "scripts/install-session.sh"), str(prefix)],
                           check=True, capture_output=True)

            for relative_path in legacy_paths:
                with self.subTest(path=relative_path):
                    self.assertFalse((prefix / relative_path).exists())

    def test_rpm_build_paths_and_metadata(self):
        for project in ("mutter", "gnome-shell"):
            expanded = subprocess.check_output(["rpmspec", "-P", str(ROOT / f"packaging/rpm/{project}.spec")], text=True)
            self.assertIn("--prefix=/usr/lib/gnoblin", expanded)
            self.assertIn("--libdir=/usr/lib/gnoblin/lib64", expanded)
            self.assertNotRegex(expanded, r"(?m)^(?:Conflicts|Obsoletes):")
            self.assertNotRegex(expanded, r"(?m)^Name:\s+(?:mutter|gnome-shell)$")
            if project == "gnome-shell":
                self.assertIn("BuildRequires:  gnoblin-mutter-devel", expanded)
                self.assertIn("Exec=/usr/lib/gnoblin/bin/gnoblin-session", expanded)
                self.assertIn("-Dextensions_app=false", expanded)
                self.assertIn("-Dextensions_tool=false", expanded)
                self.assertNotIn("Requires:       gnome-control-center", expanded)
                self.assertNotRegex(expanded, r"(?m)^Requires:\s+gettext$")

    def test_build_routes_disable_extension_manager_tools(self):
        justfile = (ROOT / "Justfile").read_text()
        nix_package = (ROOT / "nix/package.nix").read_text()
        self.assertIn('if [ "{{PROJ}}" = gnome-shell ]; then options=(-Dextensions_app=false -Dextensions_tool=false)',
                      justfile)
        self.assertIn("-Dextensions_app=false", justfile)
        self.assertIn("-Dextensions_tool=false", justfile)
        self.assertIn('"-Dextensions_app=false"', nix_package)
        self.assertIn('"-Dextensions_tool=false"', nix_package)


if __name__ == "__main__":
    unittest.main()
