#!/usr/bin/env python3
"""Regression checks for installation alongside an existing GNOME session."""

import importlib.util
import json
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
                '#!/bin/sh\nprintf "sudo %s\\n" "$*" >> "$TEST_CALLS"\n[ "$1" = tar ] && cat > /dev/null\nexit 0\n'
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
                check=True,
                capture_output=True,
                text=True,
            )
            self.assertEqual(stock.read_text(), "stock GNOME unit\n")
            calls = (base / "calls").read_text()
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

    def test_manifest_has_one_gnoblin_runtime_package(self):
        manifest = json.loads((ROOT / "packaging/native-packages.json").read_text())
        packages = manifest["packages"]
        self.assertNotIn("gnoblin-session", packages)
        self.assertIn("lua", packages["gnoblin"]["requires"])
        self.assertEqual(manifest["requirements"]["lua"]["minVersion"], "5.4")
        self.assertIn("gtk4", packages["xdg-desktop-portal-gnoblin"]["requires"])
        self.assertEqual(manifest["requirements"]["gtk4"]["minVersion"], "4.20.0")
        self.assertNotIn("xdg-desktop-portal-gnoblin", packages["gnoblin"].get("requiresSameMajor", []))
        self.assertEqual(manifest["requirements"]["gsettings-desktop-schemas"]["minVersion"], "49.1")
        arch = (ROOT / "packaging/arch/PKGBUILD").read_text()
        makedepends = next(line for line in arch.splitlines() if line.startswith("makedepends="))
        depends = next(line for line in arch.splitlines() if line.startswith("depends="))
        self.assertIn("'lua>=5.4'", arch)
        for build_dependency in (
            "'libxkbcommon'",
            "'wayland-protocols>=1.48'",
        ):
            self.assertIn(build_dependency, makedepends)
        for runtime_dependency in ("'cairo'", "'pango'", "'wayland'", "'libxkbcommon'"):
            self.assertNotIn(runtime_dependency, depends)
        self.assertIn("'gsettings-desktop-schemas>=49.1'", arch)
        self.assertNotIn("'gtk4>=4.20.0'", arch)
        self.assertNotIn("'xdg-desktop-portal>=1.20.0'", arch)
        portal_arch = (ROOT / "packaging/arch/portal/PKGBUILD").read_text()
        self.assertIn("'gtk4>=4.20.0'", portal_arch)
        self.assertIn("'xdg-desktop-portal>=1.20.0'", portal_arch)
        self.assertIn("systemd-libs", packages["gnoblin"]["requires"])
        self.assertNotIn("systemd", packages["gnoblin"]["requires"])

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

    def test_rpm_build_paths_and_metadata(self):
        for project in ("xdg-desktop-portal-gnoblin", "gnoblin"):
            expanded = subprocess.check_output(
                ["rpmspec", "-P", str(ROOT / f"packaging/rpm/{project}.spec")], text=True
            )
            if project == "xdg-desktop-portal-gnoblin":
                self.assertIn("--prefix=/usr/lib/gnoblin", expanded)
                self.assertIn("--libdir=/usr/lib/gnoblin/lib64", expanded)
            elif project == "gnoblin":
                self.assertIn("./build.sh --layout system", expanded)
                self.assertIn("--prefix /usr/lib/gnoblin", expanded)
                self.assertIn("GNOBLIN_LIBDIR=lib64", expanded)
            if project != "gnoblin":
                self.assertNotRegex(expanded, r"(?m)^(?:Conflicts|Obsoletes):")
            self.assertNotRegex(expanded, r"(?m)^Name:\s+(?:mutter|gnome-shell)$")
            self.assertNotRegex(expanded, r"(?m)^Provides:\s+lib(?:mutter|shell-|st-)")
            self.assertNotIn("-Degl_device", expanded)
            if project == "gnoblin":
                version = subprocess.check_output(
                    [str(ROOT / "scripts/gnoblin-version.py"), "get", "version"], text=True
                ).strip()
                gnome_major = json.loads((ROOT / "packaging/native-packages.json").read_text())["release"]["gnomeMajor"]
                self.assertIn(f"Provides:       gnoblin-session = {version}", expanded)
                self.assertIn(f"Obsoletes:      gnoblin-session < {gnome_major}", expanded)
                self.assertIn(f"Obsoletes:      gnoblin-shell < {gnome_major}", expanded)
                self.assertNotIn("BuildRequires:  gnoblin-mutter-devel", expanded)
                # The old separate Mutter package may exist on upgraded hosts, so the
                # spec obsoletes it, but nothing may require it or build against it.
                self.assertIn(f"Obsoletes:      gnoblin-mutter < {gnome_major + 1}", expanded)
                self.assertNotRegex(expanded, r"(?m)^(Build)?Requires:\s+gnoblin-mutter")
                self.assertNotIn("Requires:       gnoblin-session", expanded)
                self.assertNotIn("Requires:       gnoblin-shell", expanded)
                self.assertNotIn("Requires:       gjs", expanded)
                self.assertNotIn("GNOBLIN_INSTALL_GNOME_COMPAT", expanded)
                for build_requirement in ("BuildRequires:  pkgconfig(xkbcommon)",):
                    self.assertIn(build_requirement, expanded)
                self.assertNotIn("gnoblin-recovery", expanded)
                # The session file's Exec line comes from the build's system layout step, which the layout test checks.
                arch = (ROOT / "packaging/arch/PKGBUILD").read_text()
                self.assertNotIn("gnoblin-recovery", arch)

    def test_portal_rpm_requires_gtk_422_for_build_and_runtime(self):
        expanded = subprocess.check_output(
            ["rpmspec", "-P", str(ROOT / "packaging/rpm/xdg-desktop-portal-gnoblin.spec")], text=True
        )
        self.assertIn("BuildRequires:  pkgconfig(gtk4) >= 4.20.0", expanded)
        self.assertIn("Requires:       gtk4 >= 4.20.0", expanded)

    def test_geoclue_agent_authorization_is_an_optional_rpm_package(self):
        expanded = subprocess.check_output(["rpmspec", "-P", str(ROOT / "packaging/rpm/gnoblin.spec")], text=True)
        base_package, integration = expanded.split("%package -n gnoblin-geoclue-integration", 1)
        self.assertNotRegex(base_package, r"(?m)^Requires:\s+geoclue2(?:\s|$)")
        self.assertIn("Requires:       geoclue2 >= 2.7.2", integration)
        self.assertIn("/etc/geoclue/conf.d/50-gnoblin.conf", integration)
        whitelist = (ROOT / "packaging/geoclue/50-gnoblin.conf").read_text()
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

    def test_nix_package_has_no_gnome_shell_runtime_path(self):
        package = (ROOT / "nix/package.nix").read_text()
        flake = (ROOT / "flake.nix").read_text()
        lock = (ROOT / "flake.lock").read_text()
        for obsolete in ("gnoblinShell", "gnomeShellSrc", "gnoblin-shell-service", "gjs"):
            with self.subTest(obsolete=obsolete):
                self.assertNotIn(obsolete, package)
        self.assertNotIn('"gnome-shell-src"', flake)
        self.assertNotIn('"gnome-shell-src"', lock)
        self.assertNotIn("pkgs.gnome-shell", flake)

    def test_system_schemas_are_required_before_the_compositor(self):
        publisher = (ROOT / "scripts/publish-copr.sh").read_text()
        gnoblin = subprocess.check_output(["rpmspec", "-P", str(ROOT / "packaging/rpm/gnoblin.spec")], text=True)
        self.assertIn("BuildRequires: pkgconfig(gsettings-desktop-schemas) >= 49.1", gnoblin)
        self.assertIn("BuildRequires:  pkgconfig(lua)", gnoblin)
        self.assertIn("Requires:       gsettings-desktop-schemas >= 49.1", gnoblin)
        component_build = (ROOT / "cmake/component-build.cmake").read_text()
        self.assertIn('component_env_prepend(GI_GIR_PATH "${installed_prefix}/share/gir-1.0")', component_build)
        runtime_env = (ROOT / "src/tools/gnoblin-env.sh").read_text()
        self.assertNotIn("GI_TYPELIB_PATH", runtime_env)
        build_order = [
            publisher.index(f'build_in_supported_fedora_chroots "${name}"') for name in ("portal_srpm", "meta_srpm")
        ]
        self.assertEqual(build_order, sorted(build_order))

    def test_lua_build_dependency_belongs_to_the_gnoblin_runtime(self):
        gnoblin = subprocess.check_output(["rpmspec", "-P", str(ROOT / "packaging/rpm/gnoblin.spec")], text=True)
        self.assertIn("BuildRequires:  pkgconfig(lua)", gnoblin)

    def test_system_install_defaults_to_official_copr(self):
        installer = (ROOT / "scripts/install-system.sh").read_text()
        makefile = (ROOT / "Makefile").read_text()
        self.assertIn("SOURCE=copr", installer)
        self.assertIn('dnf copr enable -y "$copr"', installer)
        self.assertIn('dnf "$VERB" "${DNF_OPTIONS[@]}" --refresh "${copr_packages[@]}"', installer)
        self.assertIn('packages=("gnoblin:$META_VERSION"', installer)
        self.assertIn("gnoblin) project=gnoblin", installer)
        self.assertIn("install --refresh gnoblin", (ROOT / ".github/workflows/verify.yml").read_text())
        # No task passes a local RPM option to the COPR installer.
        self.assertNotIn("--local-rpms", makefile)
        self.assertIn("scripts/legacy/gnome-session@gnoblin.target.d.conf", installer)

    def test_system_installer_removes_only_the_exact_legacy_gnome_dropin(self):
        installer = (ROOT / "scripts/install-system.sh").read_text()
        session_installer = (ROOT / "cmake/install-session.cmake").read_text()

        self.assertNotIn("gnoblin-recovery.service", installer)
        self.assertIn('cmp -s "$LEGACY_GNOME_SESSION_DROPIN"', installer)
        self.assertIn('rm -- "$LEGACY_GNOME_SESSION_DROPIN_PATH"', installer)
        self.assertNotIn("gnome-session@gnoblin.target.d.conf", session_installer)

    def test_system_installer_removes_exact_legacy_dropin_after_dnf_success(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            dnf_log = base / "dnf.log"
            (fake_bin / "dnf").write_text(
                "#!/bin/sh\n"
                'printf "%s\\n" "$*" >> "$GNOBLIN_FAKE_DNF_LOG"\n'
                'if [ "$1" = repoquery ]; then printf "gnoblin-45-1.noarch\\n"; fi\n'
                "exit 0\n"
            )
            (fake_bin / "rpm").write_text(
                "#!/bin/sh\n"
                'if [ "$1" = -E ]; then\n'
                '  case "$2" in\n'
                '    %fedora) printf "45\\n" ;;\n'
                '    %_arch) printf "x86_64\\n" ;;\n'
                "  esac\n"
                "fi\n"
                "exit 0\n"
            )
            (fake_bin / "sudo").write_text('#!/bin/sh\nexec "$@"\n')
            (fake_bin / "systemctl").write_text("#!/bin/sh\nexit 0\n")
            for command in fake_bin.iterdir():
                command.chmod(0o755)

            legacy = ROOT / "scripts/legacy/gnome-session@gnoblin.target.d.conf"
            config_home = base / "config"
            dropin = config_home / "systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"
            dropin.parent.mkdir(parents=True)
            dropin.write_bytes(legacy.read_bytes())
            environment = os.environ | {
                "HOME": str(base / "home"),
                "XDG_CONFIG_HOME": str(config_home),
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
                "GNOBLIN_FAKE_DNF_LOG": str(dnf_log),
            }
            (base / "home").mkdir()

            result = subprocess.run(
                ["bash", str(ROOT / "scripts/install-system.sh")],
                cwd=ROOT,
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(dropin.exists())
            self.assertIn("Removed the obsolete managed GNOME session drop-in.", result.stdout)
            self.assertIn("install --refresh", dnf_log.read_text())

    def test_system_installer_preserves_custom_legacy_dropin_and_aborts(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            dnf_log = base / "dnf.log"
            (fake_bin / "dnf").write_text('#!/bin/sh\necho called >> "$GNOBLIN_FAKE_DNF_LOG"\n')
            for command in fake_bin.iterdir():
                command.chmod(0o755)
            config_home = base / "config"
            dropin = config_home / "systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf"
            dropin.parent.mkdir(parents=True)
            dropin.write_text("[Unit]\nWants=custom.service\n")
            home = base / "home"
            home.mkdir()
            environment = os.environ | {
                "HOME": str(home),
                "XDG_CONFIG_HOME": str(config_home),
                "PATH": f"{fake_bin}:{os.environ['PATH']}",
                "GNOBLIN_FAKE_DNF_LOG": str(dnf_log),
            }

            result = subprocess.run(
                ["bash", str(ROOT / "scripts/install-system.sh")],
                cwd=ROOT,
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )

            self.assertNotEqual(result.returncode, 0)
            self.assertIn("Move the custom Gnoblin override aside first:", result.stderr)
            self.assertEqual(dropin.read_text(), "[Unit]\nWants=custom.service\n")
            self.assertFalse(dnf_log.exists())


if __name__ == "__main__":
    unittest.main()
