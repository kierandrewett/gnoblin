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
        self.assertNotIn("python3dist(argcomplete)", content)
        self.assertIn(
            "BuildRequires:  pkgconfig(glycin-2) >= 2.0.beta.2",
            (SPECS / "mutter.spec").read_text(),
        )
        self.assertIn(
            "BuildRequires:  pkgconfig(libdisplay-info) >= 0.2",
            (SPECS / "mutter.spec").read_text(),
        )

    def test_no_private_prefix_is_used_for_the_meson_build_tool(self):
        for spec_name in ("mutter.spec", "gnoblin-portal.spec"):
            with self.subTest(spec=spec_name):
                content = (SPECS / spec_name).read_text()
                self.assertNotIn("%{_bindir}/meson", content)
                self.assertIn("/usr/bin/meson setup build .", content)
                self.assertIn("/usr/bin/meson compile -C build", content)
                self.assertIn("/usr/bin/meson install -C build", content)

    def test_session_is_standalone_and_uses_gnoblin_names(self):
        content = (SPECS / "gnoblin.spec").read_text()
        normalized_content = " ".join(content.split())
        self.assertIn("/usr/share/wayland-sessions/gnoblin.desktop", content)
        self.assertIn("scripts/install-session.sh %{_prefix}", content)
        install = (ROOT / "scripts/install-session.sh").read_text()
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
        self.assertIn("mutter|gnoblin-portal|gnoblin", check)
        self.assertIn('rpmspec -q --buildrequires "$spec"', check)
        self.assertIn("install --dry-run --no-recommends", check)
        self.assertNotIn("--compat-runtime", check)
        self.assertIn("for attempt in 1 2 3", check)

    def test_build_chain_respects_internal_dependency_order(self):
        chain = (SPECS / "build-chain.sh").read_text()
        self.assertLess(
            chain.index("git fetch --force --tags origin"),
            chain.index('"$ROOT/scripts/make-tarball.sh"'),
        )
        self.assertLess(chain.index("build mutter.spec"), chain.index("build gnoblin-portal.spec"))
        self.assertLess(chain.index("build gnoblin-portal.spec"), chain.index("build gnoblin.spec"))
        self.assertLess(
            chain.index('check-buildrequires.sh" mutter --install'),
            chain.index('if [[ -n "$PREPARED_SOURCES" ]]'),
        )
        self.assertLess(
            chain.index('check-buildrequires.sh" gnoblin-portal --install'),
            chain.index("build gnoblin-portal.spec"),
        )
        self.assertLess(
            chain.index('check-buildrequires.sh" gnoblin --install'),
            chain.index("build gnoblin.spec"),
        )
        self.assertIn("--allow-unsigned-rpm", chain)


if __name__ == "__main__":
    unittest.main()
