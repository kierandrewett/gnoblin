#!/usr/bin/env python3
"""Static boundaries for the openSUSE Tumbleweed package adapter."""

from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parent.parent
SPECS = ROOT / "packaging" / "opensuse"


class OpenSUSEPackagingTests(unittest.TestCase):
    def test_private_runtime_and_tumbleweed_capabilities(self):
        content = (SPECS / "mutter.spec").read_text()
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
            "BuildRequires:  pkgconfig(hyprcursor) >= 0.1.13",
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

    def test_no_private_prefix_is_used_for_the_meson_build_tool(self):
        mutter = (SPECS / "mutter.spec").read_text()
        self.assertNotIn("%{_bindir}/meson", mutter)
        self.assertIn("/usr/bin/meson setup build .", mutter)
        self.assertIn("/usr/bin/meson compile -C build", mutter)
        self.assertIn("/usr/bin/meson install -C build", mutter)

    def test_session_is_standalone_and_uses_gnoblin_names(self):
        content = (SPECS / "gnoblin.spec").read_text()
        self.assertIn("/usr/share/wayland-sessions/gnoblin.desktop", content)
        self.assertIn("gnoblin-session.target", content)
        self.assertIn("does not require GNOME Shell or GJS", content)
        self.assertNotIn("org.gnome.Shell@wayland.service", content)
        self.assertFalse((SPECS / "gnoblin-shell.spec").exists())

    def test_meta_uses_tumbleweed_runtime_library_names(self):
        meta = (SPECS / "gnoblin.spec").read_text()
        self.assertIn("Requires:       libinput10 >= 1.31", meta)
        self.assertIn("Requires:       libwayland-client0 >= 1.26", meta)

    def test_check_script_keeps_the_probe_non_installing(self):
        check = (SPECS / "check-buildrequires.sh").read_text()
        self.assertIn("--without gnoblin_stack", check)
        self.assertIn("install --dry-run --no-recommends", check)

    def test_build_chain_respects_internal_dependency_order(self):
        chain = (SPECS / "build-chain.sh").read_text()
        self.assertLess(
            chain.index("git fetch --force --tags origin"),
            chain.index('"$ROOT/scripts/make-tarball.sh"'),
        )
        self.assertLess(chain.index("build mutter.spec"), chain.index("build gnoblin-portal.spec"))
        self.assertLess(chain.index("build gnoblin-portal.spec"), chain.index("build gnoblin.spec"))
        self.assertIn("--allow-unsigned-rpm", chain)


if __name__ == "__main__":
    unittest.main()
