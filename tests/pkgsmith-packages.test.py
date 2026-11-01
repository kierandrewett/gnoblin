#!/usr/bin/env python3
"""Check the package definitions in pkgsmith/ against a prefix laid out the way ./build.sh --layout system lays it out.

The test runs pkgsmith, so it needs `pkgsmith` on PATH (scripts/install-pkgsmith.sh builds the pinned one) and is
skipped without it. It builds no package and needs no compiled Gnoblin: a few small files stand for the real ones.
"""

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
PKGSMITH = shutil.which("pkgsmith")

# A file of every kind that the packages split: runtime, portal backend, public entries and the two optional configs.
PREFIX_FILES = {
    "usr/lib/gnoblin/bin/gnoblin": "gnoblin",
    "usr/lib/gnoblin/lib/libmutter-51.so.0.0.0": "gnoblin",
    "usr/lib/gnoblin/share/glib-2.0/schemas/org.gnome.mutter.gschema.xml": "gnoblin",
    "usr/lib/gnoblin/share/locale/de/LC_MESSAGES/mutter.mo": "gnoblin",
    "usr/lib/gnoblin/share/licenses/gnoblin/COPYING": "gnoblin",
    "usr/share/wayland-sessions/gnoblin.desktop": "gnoblin",
    "usr/share/xdg-desktop-portal/gnoblin-portals.conf": "gnoblin",
    "usr/lib/systemd/user/gnoblin-session.target": "gnoblin",
    "usr/lib/gnoblin/libexec/xdg-desktop-portal-gnoblin": "xdg-desktop-portal-gnoblin",
    "usr/lib/gnoblin/lib/systemd/user/xdg-desktop-portal-gnoblin.service": "xdg-desktop-portal-gnoblin",
    "usr/lib/gnoblin/share/applications/xdg-desktop-portal-gnoblin.desktop": "xdg-desktop-portal-gnoblin",
    "usr/lib/gnoblin/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service": "xdg-desktop-portal-gnoblin",
    "usr/lib/gnoblin/share/glib-2.0/schemas/xdg-desktop-portal-gnome.gschema.xml": "xdg-desktop-portal-gnoblin",
    "usr/lib/gnoblin/share/licenses/xdg-desktop-portal-gnoblin/COPYING": "xdg-desktop-portal-gnoblin",
    "usr/lib/gnoblin/share/locale/de/LC_MESSAGES/xdg-desktop-portal-gnome.mo": "xdg-desktop-portal-gnoblin",
    "usr/lib/gnoblin/share/xdg-desktop-portal/portals/gnoblin.portal": "xdg-desktop-portal-gnoblin",
    "usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service": "xdg-desktop-portal-gnoblin",
    "usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service": "xdg-desktop-portal-gnoblin",
    "usr/share/xdg-desktop-portal/portals/gnoblin.portal": "xdg-desktop-portal-gnoblin",
    "etc/geoclue/conf.d/50-gnoblin.conf": "gnoblin-geoclue-integration",
    "etc/ananicy.d/99-gnoblin.rules": "gnoblin-ananicy-rules",
    "etc/ananicy.d/99-gnoblin.types": "gnoblin-ananicy-rules",
}
ARGUMENTS = {
    "VERSION": "0.1.13",
    "PORTAL_VERSION": "51.0.0",
    "NEEDED_GNOBLIN": "libc.so.6,libglib-2.0.so.0,libgtk-4.so.1,libinput.so.10,libwayland-server.so.0",
    "NEEDED_PORTAL": "libc.so.6,libgtk-4.so.1,libadwaita-1.so.0",
    "GLIBC": "2.43",
    "FEDORA": "45",
    "LANES": "all",
}


@unittest.skipUnless(PKGSMITH, "pkgsmith is not on PATH")
class PackageDefinitions(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.prefix = Path(tempfile.mkdtemp(prefix="pkgsmith-prefix-"))
        for relative in PREFIX_FILES:
            path = cls.prefix / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("x\n")
        (cls.prefix / "usr/bin").mkdir(parents=True)
        (cls.prefix / "usr/bin/gnoblin").symlink_to("/usr/lib/gnoblin/bin/gnoblin")

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.prefix, ignore_errors=True)

    def run_pkgsmith(self, command, script, *extra, **overrides):
        arguments = {**ARGUMENTS, **overrides}
        return subprocess.run(
            [
                PKGSMITH,
                command,
                str(ROOT),
                "-f",
                str(ROOT / "pkgsmith" / f"{script}.star"),
                "--prefix",
                str(self.prefix),
                *[f"--build-arg={key}={value}" for key, value in arguments.items()],
                *extra,
            ],
            capture_output=True,
            text=True,
            env={key: value for key, value in os.environ.items() if key != "PKGSMITH_PLUGIN_PATH"},
        )

    def plan(self, script, **overrides):
        result = self.run_pkgsmith("plan", script, "--json", **overrides)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_a_glibc_prefix_makes_rpm_deb_arch_and_tar_but_no_apk(self):
        formats = {row["format"] for script in ("runtime", "integrations") for row in self.plan(script)}
        self.assertEqual(formats, {"rpm", "deb", "archlinux", "tar"})

    def test_a_musl_prefix_makes_apk_only(self):
        formats = {row["format"] for row in self.plan("runtime", LIBC="musl")}
        self.assertEqual(formats, {"apk"})

    def test_four_packages_with_the_names_the_old_recipes_used(self):
        names = {row["values"]["name"] for script in ("runtime", "integrations") for row in self.plan(script)}
        self.assertEqual(
            names,
            {"gnoblin", "xdg-desktop-portal-gnoblin", "gnoblin-geoclue-integration", "gnoblin-ananicy-rules"},
        )

    def test_fedora_and_opensuse_have_their_own_rpm(self):
        rows = [row for row in self.plan("runtime") if row["format"] == "rpm" and row["values"]["name"] == "gnoblin"]
        self.assertEqual(
            sorted(row["file"] for row in rows),
            ["gnoblin-0.1.13-1.fc45.x86_64.rpm", "gnoblin-0.1.13-1.tumbleweed.x86_64.rpm"],
        )

    def test_the_fedora_rpm_is_named_for_the_release_that_built_the_prefix(self):
        for release in ("43", "44", "45"):
            with self.subTest(release=release):
                files = {row["file"] for row in self.plan("runtime", FEDORA=release, LANES="fedora")}
                self.assertEqual(
                    files,
                    {
                        f"gnoblin-0.1.13-1.fc{release}.x86_64.rpm",
                        f"xdg-desktop-portal-gnoblin-51.0.0-1013.fc{release}.x86_64.rpm",
                    },
                )
        geoclue = [row for row in self.plan("integrations", FEDORA="43", LANES="fedora") if row["format"] == "rpm"]
        self.assertEqual(
            {row["file"] for row in geoclue},
            {
                "gnoblin-geoclue-integration-0.1.13-1.fc43.noarch.rpm",
                "gnoblin-ananicy-rules-0.1.13-1.fc43.noarch.rpm",
            },
        )

    def test_the_other_lane_writes_no_fedora_rpm_and_needs_no_fedora_release(self):
        rows = self.plan("runtime", FEDORA="", LANES="other")
        self.assertNotIn("fedora", {row["distro"] for row in rows})
        self.assertEqual({row["format"] for row in rows}, {"rpm", "deb", "archlinux", "tar"})
        self.assertEqual(
            [row["file"] for row in rows if row["format"] == "rpm" and row["values"]["name"] == "gnoblin"],
            ["gnoblin-0.1.13-1.tumbleweed.x86_64.rpm"],
        )

    def test_the_fedora_lane_needs_a_release(self):
        for lanes in ("all", "fedora"):
            with self.subTest(lanes=lanes):
                result = self.run_pkgsmith("plan", "runtime", FEDORA="", LANES=lanes)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("FEDORA", result.stderr + result.stdout)

    def test_an_older_fedora_soname_is_for_the_fedora_rpm_only(self):
        needed = "libc.so.6,libdisplay-info.so.2"
        fedora = self.run_pkgsmith("plan", "runtime", NEEDED_GNOBLIN=needed, FEDORA="43", LANES="fedora")
        self.assertEqual(fedora.returncode, 0, fedora.stderr)
        result = self.run_pkgsmith("plan", "runtime", "-F", "deb", NEEDED_GNOBLIN=needed, LANES="other")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("libdisplay-info.so.2", result.stderr + result.stdout)

    def test_versions_are_spelled_for_each_format(self):
        files = {
            row["format"] + ":" + row["values"]["name"]: row["file"]
            for row in self.plan("runtime", VERSION="0.1.13-nightly.20261031.142.gabc1234")
            if row["distro"] in (None, "", "fedora")
        }
        self.assertEqual(files["rpm:gnoblin"], "gnoblin-0.1.13~nightly.20261031.142.gabc1234-1.fc45.x86_64.rpm")
        self.assertEqual(files["deb:gnoblin"], "gnoblin_0.1.13~nightly.20261031.142.gabc1234-1_amd64.deb")
        # Epoch 1 and pacman's spelling: no tilde, and no commit.
        self.assertEqual(files["archlinux:gnoblin"], "gnoblin-1:0.1.13nightly20261031.142-1-x86_64.pkg.tar.zst")

    def test_the_portal_keeps_its_gnome_version_and_numbers_its_release_by_gnoblin_release(self):
        files = {
            row["format"]: row["file"]
            for row in self.plan("runtime")
            if row["values"]["name"] == "xdg-desktop-portal-gnoblin" and row["distro"] in (None, "", "fedora")
        }
        self.assertEqual(files["deb"], "xdg-desktop-portal-gnoblin_51.0.0-1013_amd64.deb")
        newer = self.plan("runtime", VERSION="0.1.14")
        self.assertIn(
            "xdg-desktop-portal-gnoblin_51.0.0-1014_amd64.deb",
            [row["file"] for row in newer if row["format"] == "deb"],
        )

    def test_an_architecture_other_than_the_host_is_written_as_given(self):
        rows = self.plan("runtime", ARCH="arm64")
        self.assertEqual({row["arch"] for row in rows}, {"arm64"})
        self.assertIn("gnoblin_0.1.13-1_arm64.deb", [row["file"] for row in rows])

    def test_a_library_without_a_package_stops_the_script(self):
        result = self.run_pkgsmith("plan", "runtime", NEEDED_GNOBLIN="libc.so.6,libnew-thing.so.1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("libnew-thing.so.1", result.stderr + result.stdout)

    def test_every_file_is_in_exactly_one_package(self):
        found = {}
        for script in ("runtime", "integrations"):
            result = self.run_pkgsmith("files", script, "-F", "tar")
            self.assertEqual(result.returncode, 0, result.stderr)
            package = None
            for line in result.stdout.splitlines():
                if line.startswith("== "):
                    package = line.split("name=")[1].rstrip("]")
                elif line[:3] in ("644", "755", "777") and line[3:4] == " ":
                    path = line.split(" ", 2)[2].split(" -> ")[0]
                    found.setdefault(path, []).append(package)
        for relative, owner in PREFIX_FILES.items():
            with self.subTest(file=relative):
                self.assertEqual(found.get("/" + relative), [owner])
        self.assertEqual(found.get("/usr/bin/gnoblin"), ["gnoblin"])


if __name__ == "__main__":
    unittest.main()
