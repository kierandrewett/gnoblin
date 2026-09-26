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
        self.assertIn(
            "BuildRequires:  pkgconfig(udev)",
            (SPECS / "mutter.spec").read_text(),
        )
        self.assertIn(
            "BuildRequires:  python3dist(argcomplete)",
            (SPECS / "mutter.spec").read_text(),
        )
        self.assertIn(
            "BuildRequires:  pkgconfig(hyprcursor) >= 0.1.11",
            (SPECS / "mutter.spec").read_text(),
        )
        self.assertIn(
            "BuildRequires:  pkgconfig(glycin-2) >= 2.0.beta.2",
            (SPECS / "mutter.spec").read_text(),
        )
        self.assertIn(
            "BuildRequires:  pkgconfig(libdisplay-info) >= 0.2",
            (SPECS / "mutter.spec").read_text(),
        )

    def test_shell_declares_gcr_and_girepository_source_floors(self):
        shell = (SPECS / "gnome-shell.spec").read_text()
        self.assertIn("BuildRequires:  pkgconfig(gcr-4) >= 3.90.0", shell)
        self.assertIn("BuildRequires:  pkgconfig(girepository-2.0) >= 2.86.0", shell)

    def test_no_private_prefix_is_used_for_the_meson_build_tool(self):
        for name in ("gsettings-desktop-schemas.spec", "mutter.spec", "gnome-shell.spec"):
            content = (SPECS / name).read_text()
            self.assertNotIn("%{_bindir}/meson", content)
        schemas = (SPECS / "gsettings-desktop-schemas.spec").read_text()
        self.assertIn("/usr/bin/meson setup build .", schemas)
        self.assertIn("/usr/bin/meson compile -C build", schemas)
        self.assertIn("/usr/bin/meson install -C build", schemas)

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

    def test_check_script_uses_the_default_private_stack_boundary_portably(self):
        check = (SPECS / "check-buildrequires.sh").read_text()
        self.assertNotIn("--without gnoblin_stack", check)
        self.assertIn('rpmspec -P "${rpmspec_args[@]}" "$spec"', check)
        self.assertIn('rpmspec -q --buildrequires "${rpmspec_args[@]}" "$spec"', check)
        self.assertIn("install --dry-run --no-recommends", check)
        self.assertIn("--compat-runtime", check)
        self.assertIn("_with_gnoblin_compat_runtime 1", check)

    def test_build_chain_respects_internal_dependency_order(self):
        chain = (SPECS / "build-chain.sh").read_text()
        self.assertIn('source "$ROOT/scripts/retry-command.sh"', chain)
        self.assertIn('gnoblin_retry_command git -C "$ROOT" submodule foreach', chain)
        self.assertLess(
            chain.index("git fetch --force --tags origin"),
            chain.index('"$ROOT/scripts/make-tarball.sh"'),
        )
        self.assertLess(
            chain.index("build gsettings-desktop-schemas.spec"),
            chain.index("build mutter.spec --with gnoblin_stack"),
        )
        self.assertLess(
            chain.index("build mutter.spec --with gnoblin_stack"),
            chain.index("build gnome-shell.spec --with gnoblin_stack"),
        )
        self.assertIn("--allow-unsigned-rpm", chain)
        self.assertLess(
            chain.index("\nbuild_compatibility_runtime\n"),
            chain.index("build gsettings-desktop-schemas.spec"),
        )
        self.assertIn("build compat-runtime.spec", chain)
        self.assertIn("gnoblin_compat_runtime", chain)
        self.assertIn('check-buildrequires.sh" --install --compat-runtime', chain)
        self.assertIn('mkdir -p "$ROOT/build"', chain)

    def test_private_compatibility_runtime_has_its_own_rpm_boundary(self):
        spec = (SPECS / "compat-runtime.spec").read_text()
        self.assertIn("Name:           gnoblin-compat-runtime", spec)
        self.assertIn("%global _compat_dir %{_prefix}/deps", spec)
        self.assertIn("%global __provides_exclude_from ^%{_compat_dir}/.*$", spec)
        self.assertIn("%global __requires_exclude ^%{_compat_dir}/.*$", spec)
        self.assertIn("cp -a deps %{buildroot}%{_prefix}/", spec)

    def test_compatibility_mode_uses_private_interfaces_for_mutter_and_shell(self):
        for name in ("mutter.spec", "gnome-shell.spec"):
            spec = (SPECS / name).read_text()
            self.assertIn("%bcond_with gnoblin_compat_runtime", spec)
            self.assertIn("BuildRequires:  gnoblin-compat-runtime >= %{version}", spec)
            self.assertIn("%{_prefix}/deps/lib64/pkgconfig", spec)
            self.assertIn("%{_prefix}/deps/lib64", spec)


if __name__ == "__main__":
    unittest.main()
