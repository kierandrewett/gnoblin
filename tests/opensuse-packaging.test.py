#!/usr/bin/env python3
"""Static boundaries for the openSUSE Tumbleweed package adapter."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "packaging" / "opensuse"


class OpenSUSEPackagingTests(unittest.TestCase):
    def test_private_runtime_and_tumbleweed_capabilities(self):
        content = (SPECS / "gnoblin.spec").read_text()
        self.assertIn("%global _prefix /usr/lib/gnoblin", content)
        self.assertIn("%global __provides_exclude_from ^%{_prefix}/.*$", content)
        self.assertIn("pkgconfig(", content)
        self.assertNotIn("mesa-libEGL-devel", content)
        self.assertIn(
            "BuildRequires:  pkgconfig(udev)",
            (SPECS / "gnoblin.spec").read_text(),
        )
        self.assertNotIn("python3dist(argcomplete)", content)
        self.assertIn(
            "BuildRequires:  pkgconfig(glycin-2) >= 2.0.beta.2",
            (SPECS / "gnoblin.spec").read_text(),
        )
        self.assertIn(
            "BuildRequires:  pkgconfig(libdisplay-info) >= 0.2",
            (SPECS / "gnoblin.spec").read_text(),
        )

    def test_gnoblin_uses_the_unified_cmake_build(self):
        content = (SPECS / "gnoblin.spec").read_text()
        self.assertNotIn("%{_bindir}/meson", content)
        self.assertIn("./build.sh --layout system --without-portal", content)
        self.assertIn("--destdir %{_builddir}/gnoblin-stage", content)

    def test_portal_uses_the_system_meson_build_tool(self):
        content = (SPECS / "xdg-desktop-portal-gnoblin.spec").read_text()
        self.assertNotIn("%{_bindir}/meson", content)
        self.assertIn("/usr/bin/meson setup build .", content)
        self.assertIn("/usr/bin/meson compile -C build", content)
        self.assertIn("/usr/bin/meson install -C build", content)

    def test_session_is_standalone_and_uses_gnoblin_names(self):
        content = (SPECS / "gnoblin.spec").read_text()
        normalized_content = " ".join(content.split())
        self.assertIn("/usr/share/wayland-sessions/gnoblin.desktop", content)
        self.assertIn("/usr/lib/systemd/user/gnoblin-session.target", content)
        self.assertIn("/usr/lib/systemd/user/gnoblin-idle.service", content)
        self.assertNotIn("gnoblin-recovery", content)
        self.assertIn("BuildRequires:  pkgconfig(xkbcommon)", content)
        self.assertIn("./build.sh --layout system", content)
        install = (ROOT / "cmake/install-session.cmake").read_text()
        self.assertIn("systemd-user/gnoblin-session.target", install)
        self.assertIn("does not require GNOME Shell or GJS", normalized_content)
        self.assertNotIn("org.gnome.Shell@wayland.service", content)
        self.assertFalse((SPECS / "gnoblin-shell.spec").exists())

    def test_meta_uses_tumbleweed_runtime_library_names(self):
        meta = (SPECS / "gnoblin.spec").read_text()
        self.assertIn("Requires:       libinput10 >= 1.30", meta)
        self.assertIn("Requires:       libwayland-client0 >= 1.25", meta)

    def test_check_script_keeps_the_probe_non_installing(self):
        check = (SPECS / "check-buildrequires.sh").read_text()
        self.assertIn("xdg-desktop-portal-gnoblin|gnoblin", check)
        self.assertIn(
            'rpmspec -q --buildrequires --define "gnoblin_version $gnoblin_version" "$spec"',
            check,
        )
        self.assertIn("install --dry-run --no-recommends", check)
        self.assertNotIn("--compat-runtime", check)
        self.assertIn("for attempt in 1 2 3", check)
        self.assertIn('--define "gnoblin_version $gnoblin_version"', check)

    def test_build_chain_respects_internal_dependency_order(self):
        chain = (SPECS / "build-chain.sh").read_text()
        self.assertIn('install -m 0644 -- "$gnoblin_source" "$SOURCES/"', chain)
        self.assertIn('"$ROOT/scripts/build-source-bundle.sh"', chain)
        self.assertIn('"$ROOT/scripts/make-gsettings-desktop-schemas-tarball.sh" "$SOURCES"', chain)
        self.assertIn('"$SOURCES/gsettings-desktop-schemas-$schemas_version.tar.xz"', chain)
        self.assertLess(
            chain.index("git fetch --force --tags origin"),
            chain.index('"$ROOT/scripts/make-tarball.sh"'),
        )
        self.assertLess(chain.index("build xdg-desktop-portal-gnoblin.spec"), chain.index("build gnoblin.spec"))
        self.assertLess(
            chain.index('check-buildrequires.sh" xdg-desktop-portal-gnoblin --install'),
            chain.index("build xdg-desktop-portal-gnoblin.spec"),
        )
        self.assertLess(
            chain.index('check-buildrequires.sh" gnoblin --install'),
            chain.index("build gnoblin.spec"),
        )
        self.assertIn("--allow-unsigned-rpm", chain)

    def test_openSUSE_session_spec_uses_repository_version(self):
        spec = (SPECS / "gnoblin.spec").read_text()
        chain = (SPECS / "build-chain.sh").read_text()
        self.assertIn("Version:        %{gnoblin_version}", spec)
        self.assertIn('build gnoblin.spec --define "gnoblin_version $gnoblin_version"', chain)


if __name__ == "__main__":
    unittest.main()
