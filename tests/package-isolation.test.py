#!/usr/bin/env python3
"""Regression checks for installation alongside an existing GNOME session."""

import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("isolation", ROOT / "scripts/check-rpm-isolation.py")
isolation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(isolation)


def session_build_inputs(base):
    fixture = base / "session-build-inputs"
    fixture.mkdir()
    binary = fixture / "helper"
    binary.write_text("#!/bin/sh\nexit 0\n")
    binary.chmod(0o755)
    identity = fixture / "version.json"
    identity.write_text('{"version":"0.1.7"}\n')
    version = fixture / "version.ini"
    version.write_text("version=0.1.7\n")
    return {
        "GNOBLIN_IDLE_BINARY": str(binary),
        "GNOBLINCTL_BINARY": str(binary),
        "GNOBLIN_IDENTITY_FILE": str(identity),
        "GNOBLIN_VERSION_METADATA_FILE": str(version),
        "GNOBLIN_BINARY": str(binary),
        "GNOBLIN_VECTOR_CURSORS": "OFF",
    }


def install_monolithic_compositor(prefix, inputs):
    compositor = prefix / "bin/gnoblin"
    compositor.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(inputs["GNOBLIN_BINARY"], compositor)


def write_public_entries(prefix):
    """Write the list of public files, as the build does after it installs the session."""
    subprocess.run(
        ["cmake", f"-DGNOBLIN_PREFIX={prefix}", "-P", str(ROOT / "cmake/write-public-entries.cmake")],
        check=True,
        capture_output=True,
    )


class IsolationTests(unittest.TestCase):
    def test_source_install_removes_known_legacy_shell_files_only(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            prefix = base / "runtime"
            old_files = (
                "bin/gnome-shell",
                "libexec/gnome-shell-hotplug-sniffer",
                "share/gnome-shell/modes/gnoblin.json",
                "share/gnome-shell/gnome-shell-theme.gresource",
                "share/gnome-shell/org.gnome.Shell.Notifications.src.gresource",
            )
            for relative in old_files:
                old = prefix / relative
                old.parent.mkdir(parents=True, exist_ok=True)
                old.write_text("old Gnoblin Shell payload\n")
            marker = prefix / "share/gnome-shell/local-marker.txt"
            marker.write_text("preserve local data\n")
            inputs = session_build_inputs(base)
            install_monolithic_compositor(prefix, inputs)

            subprocess.run(
                ["cmake", f"-DGNOBLIN_PREFIX={prefix}", "-P", str(ROOT / "cmake/install-session.cmake")],
                env={**os.environ, **inputs},
                check=True,
                capture_output=True,
            )

            for relative in old_files:
                self.assertFalse((prefix / relative).exists(), relative)
            self.assertEqual(marker.read_text(), "preserve local data\n")

    def test_install_copies_a_staged_build_and_keeps_stock_units(self):
        self.check_staged_install("cached")

    def test_install_nopasswd_commands_do_not_require_sudo_validate(self):
        # Mixed PASSWD/NOPASSWD ALL rules can allow commands while verifypw=all requires a password for -v.
        self.check_staged_install("nopasswd")

    def test_install_password_required_authenticates_before_changes(self):
        self.check_staged_install("password")

    def test_install_failed_authentication_changes_nothing(self):
        self.check_staged_install("failure")

    def test_install_probe_denied_by_command_policy_uses_validate(self):
        # Permission to run install commands does not imply permission to run true.
        self.check_staged_install("scoped")

    def check_staged_install(self, sudo_policy):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            # The prefix has its own parent, as /usr/local/lib/gnoblin does. The loader check hides that parent in a
            # private mount namespace, so the stage must not be inside it.
            prefix = base / "prefixes/runtime"
            prefix.parent.mkdir()
            stage = base / "stage"
            config = base / "config"
            units = config / "systemd/user"
            units.mkdir(parents=True)
            stock = units / "org.gnome.Shell@wayland.service"
            stock.write_text("stock GNOME unit\n")
            # An old registration linked the units from the source directory. They must go, because they win over
            # the installed copy. A link that does not point into a Gnoblin prefix must stay.
            old_unit = units / "gnoblin-idle.service"
            old_unit.symlink_to("/old/checkout/install/lib/systemd/user/gnoblin-idle.service")
            foreign = units / "gnoblin-session.target"
            foreign.symlink_to("/somewhere/else/session.target")
            fake_bin = base / "bin"
            fake_bin.mkdir()
            systemctl = fake_bin / "systemctl"
            systemctl.write_text('#!/bin/sh\nprintf "systemctl %s\\n" "$*" >> "$TEST_CALLS"\n')
            systemctl.chmod(0o755)
            sudo = fake_bin / "sudo"
            sudo.write_text(
                """#!/bin/sh
printf 'sudo %s\\n' "$*" >> "$TEST_CALLS"
case "$*" in
    '-n true')
        case "$TEST_SUDO_POLICY" in
            cached|nopasswd) exit 0 ;;
            *) echo 'sudo: authentication required or command denied' >&2; exit 1 ;;
        esac ;;
    '-v')
        case "$TEST_SUDO_POLICY" in
            nopasswd|failure) echo 'sudo: a password is required' >&2; exit 1 ;;
            *) touch "$TEST_AUTHENTICATED"; exit 0 ;;
        esac ;;
esac
case "$TEST_SUDO_POLICY" in
    password|failure) [ -f "$TEST_AUTHENTICATED" ] || exit 1 ;;
esac
[ "$1" = tar ] && cat > /dev/null
exit 0
"""
            )
            sudo.chmod(0o755)
            env = {
                **os.environ,
                "HOME": str(base / "home"),
                "XDG_CONFIG_HOME": str(config),
                "GNOBLIN_LIBDIR": "lib64",
                "GNOBLIN_BIN_DIR": str(base / "usr-local-bin"),
                "GNOBLIN_SYSTEM_ROOT": str(base / "usr"),
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
                "TEST_CALLS": str(base / "calls"),
                "TEST_SUDO_POLICY": sudo_policy,
                "TEST_AUTHENTICATED": str(base / "authenticated"),
            }
            inputs = session_build_inputs(base)
            # The build puts the runtime below the stage, as DESTDIR does.
            staged = stage / prefix.relative_to("/")
            install_monolithic_compositor(staged, inputs)
            env.update(inputs)
            env["GNOBLIN_STAGE_ROOT"] = str(stage)
            subprocess.run(
                ["cmake", f"-DGNOBLIN_PREFIX={prefix}", "-P", str(ROOT / "cmake/install-session.cmake")],
                env=env,
                check=True,
                capture_output=True,
            )
            write_public_entries(staged)
            result = subprocess.run(
                ["bash", str(ROOT / "scripts/register-session.sh"), str(prefix), str(stage)],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(stock.read_text(), "stock GNOME unit\n")
            calls = (base / "calls").read_text()
            sudo_calls = [line for line in calls.splitlines() if line.startswith("sudo ")]
            if sudo_policy == "failure":
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("sudo: a password is required", result.stderr)
                self.assertEqual(sudo_calls, ["sudo -n true", "sudo -v"])
                self.assertNotIn("Installing Gnoblin", result.stdout)
                self.assertNotIn("systemctl --user daemon-reload", calls)
                self.assertTrue(old_unit.is_symlink())
                self.assertTrue(foreign.is_symlink())
                for destination in (prefix, base / "usr", base / "usr-local-bin"):
                    self.assertFalse(destination.exists(), destination)
                return
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(sudo_calls[0], "sudo -n true")
            if sudo_policy in ("password", "scoped"):
                self.assertEqual(sudo_calls[1], "sudo -v")
                self.assertTrue((base / "authenticated").exists())
            else:
                self.assertNotIn("sudo -v", sudo_calls)
            self.assertIn("Gnoblin is installed", result.stdout)
            self.assertNotIn("org.gnome.Shell", calls)
            self.assertIn(f"sudo tar -C {prefix} --no-overwrite-dir -xpf -", calls)
            self.assertIn(f"{base}/usr/lib/systemd/user/gnoblin-session.target", calls)
            self.assertIn(f"ln -sfn {prefix}/bin/gnoblinctl {base}/usr-local-bin/gnoblinctl", calls)
            self.assertNotIn("xdg-desktop-portal-gnoblin.service", calls)
            self.assertFalse(old_unit.is_symlink())
            self.assertTrue(foreign.is_symlink())
            self.assertIn("Left", result.stderr)

    def test_rejects_stock_files_and_capabilities(self):
        for path in (
            "/usr/bin/gnome-shell",
            "/usr/lib64/libmutter-51.so.0",
            "/usr/lib/systemd/user/org.gnome.Shell@wayland.service",
            "/usr/share/glib-2.0/schemas/org.gnome.shell.gschema.xml",
        ):
            with self.subTest(path=path), self.assertRaises(ValueError):
                isolation.validate("gnoblin", path, "", "", "")
        for capability in (
            "gnome-shell = 51.0",
            "mutter = 51.0",
            "libmutter-51.so.0()(64bit)",
            "pkgconfig(libmutter-51) = 51.0",
        ):
            with self.subTest(capability=capability), self.assertRaises(ValueError):
                isolation.validate("gnoblin", "", capability, "", "")
        for name, conflicts, obsoletes in (
            ("gnome-shell", "", ""),
            ("gnoblin-session", "", ""),
            ("gnoblin", "gnome-shell < 49", "gnoblin-session < 51"),
            ("gnoblin", "", "mutter"),
        ):
            with self.subTest(name=name, conflicts=conflicts, obsoletes=obsoletes), self.assertRaises(ValueError):
                isolation.validate(name, "", "", conflicts, obsoletes)

    def test_rejects_development_package_requirements_and_keeps_runtime_ones(self):
        # The private runtime ships Mutter's .pc files. A pkgconfig() requirement makes a user's install pull in a
        # -devel package, which pulled GTK 3 onto a clean Fedora system in the 0.1.10 RPM.
        for requirement in ("pkgconfig(libcanberra) >= 0.26", "pkgconfig(gtk4)"):
            with self.subTest(requirement=requirement), self.assertRaises(ValueError):
                isolation.validate("gnoblin", "", "", "", "", requirement)
        isolation.validate("gnoblin", "", "", "", "", "libgtk-4.so.1()(64bit)\nglib2 >= 2.86.0\nlua-libs >= 5.4")

    def test_gnoblin_payload_provides_and_obsoletes_the_old_session_package(self):
        self.assertNotIn("/usr/lib/systemd/user/gnoblin-recovery.service", isolation.PUBLIC_FILES)
        portal_files = {
            "/usr/share/xdg-desktop-portal/portals/gnoblin.portal",
            "/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service",
            "/usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service",
        }
        paths = (
            "\n".join(sorted(isolation.PUBLIC_FILES - portal_files)) + "\n/usr/lib/gnoblin/lib/libgnoblin-runtime.so"
        )
        isolation.validate(
            "gnoblin",
            paths,
            "gnoblin-session = 0.1.7",
            "",
            "gnoblin-session < 51\ngnoblin-shell < 51",
        )

    def test_only_the_gnoblin_package_may_ship_its_two_man_pages(self):
        for page in ("gnoblin.1", "gnoblin.1.gz", "gnoblinctl.1.zst", "gnoblinctl.1.xz"):
            with self.subTest(page=page):
                isolation.validate("gnoblin", f"/usr/share/man/man1/{page}", "", "", "")
        for name, page in (
            ("xdg-desktop-portal-gnoblin", "gnoblin.1.gz"),
            ("gnoblin", "ls.1.gz"),
            ("gnoblin", "gnoblin.2.gz"),
            ("gnoblin", "gnoblinctl-extra.1.gz"),
        ):
            with self.subTest(name=name, page=page), self.assertRaises(ValueError):
                isolation.validate(name, f"/usr/share/man/man1/{page}", "", "", "")

    def test_source_install_rejects_shared_prefixes_and_symlinks(self):
        with tempfile.TemporaryDirectory() as directory:
            alias = Path(directory) / "system"
            alias.symlink_to("/usr")
            for prefix in ("/", "/usr", "/usr/local", "/usr/../usr", str(alias)):
                result = subprocess.run(
                    [
                        "bash",
                        "-c",
                        'source "$1"; gnoblin_env_validate_install_prefix "$2"',
                        "test",
                        str(ROOT / "src/tools/gnoblin-env.sh"),
                        prefix,
                    ],
                    capture_output=True,
                )
                self.assertNotEqual(result.returncode, 0, prefix)
            result = subprocess.run(
                [
                    "bash",
                    "-c",
                    'source "$1"; gnoblin_env_validate_install_prefix "$2"',
                    "test",
                    str(ROOT / "src/tools/gnoblin-env.sh"),
                    directory,
                ],
                capture_output=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_nix_package_has_no_gnome_shell_runtime_path(self):
        package = (ROOT / "packaging/nix/package.nix").read_text()
        flake = (ROOT / "flake.nix").read_text()
        lock = (ROOT / "flake.lock").read_text()
        for obsolete in ("gnoblinShell", "gnomeShellSrc", "gnoblin-shell-service", "gjs"):
            with self.subTest(obsolete=obsolete):
                self.assertNotIn(obsolete, package)
        self.assertNotIn('"gnome-shell-src"', flake)
        self.assertNotIn('"gnome-shell-src"', lock)
        self.assertNotIn("pkgs.gnome-shell", flake)

    def test_configuration_packages_may_ship_only_their_own_files(self):
        isolation.validate("gnoblin-geoclue-integration", "/etc/geoclue/conf.d/50-gnoblin.conf", "", "", "")
        isolation.validate(
            "gnoblin-ananicy-rules", "/etc/ananicy.d/99-gnoblin.rules\n/etc/ananicy.d/99-gnoblin.types", "", "", ""
        )
        for name, path in (
            ("gnoblin-geoclue-integration", "/etc/ananicy.d/99-gnoblin.rules"),
            ("gnoblin-ananicy-rules", "/etc/geoclue/conf.d/50-gnoblin.conf"),
            ("gnoblin-ananicy-rules", "/etc/passwd"),
            ("gnoblin", "/etc/geoclue/conf.d/50-gnoblin.conf"),
        ):
            with self.subTest(name=name, path=path), self.assertRaises(ValueError):
                isolation.validate(name, path, "", "", "")

    def test_geoclue_agent_authorization_keeps_the_stock_agents(self):
        whitelist = (ROOT / "src/data/geoclue/50-gnoblin.conf").read_text()
        for agent_id in (
            "geoclue-demo-agent",
            "gnome-shell",
            "io.elementary.desktop.agent-geoclue2",
            "sm.puri.Phosh",
            "lipstick",
            "gnoblin",
        ):
            with self.subTest(agent_id=agent_id):
                self.assertIn(agent_id, whitelist)

    def test_system_schemas_are_found_through_the_installed_gir_path(self):
        component_build = (ROOT / "cmake/component-build.cmake").read_text()
        self.assertIn('component_env_prepend(GI_GIR_PATH "${installed_prefix}/share/gir-1.0")', component_build)
        runtime_env = (ROOT / "src/tools/gnoblin-env.sh").read_text()
        self.assertNotIn("GI_TYPELIB_PATH", runtime_env)


if __name__ == "__main__":
    unittest.main()
