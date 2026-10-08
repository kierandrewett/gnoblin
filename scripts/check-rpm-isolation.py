#!/usr/bin/env python3
"""Reject Gnoblin RPMs that could replace system GNOME files or providers."""

import argparse
import json
from pathlib import Path, PurePosixPath
import re
import subprocess


PACKAGES = {
    "gnoblin",
    "gnoblin-portal",
}
MANIFEST = json.loads((Path(__file__).resolve().parents[1] / "packaging/native-packages.json").read_text())
GNOME_MAJOR = MANIFEST["release"]["gnomeMajor"]
ALLOWED_OBSOLETES = {
    f"gnoblin-session < {GNOME_MAJOR}",
    f"gnoblin-shell < {GNOME_MAJOR}",
    # The separate Mutter packages were merged into gnoblin. The session package replaces them on upgrade.
    "gnoblin-mutter < 52",
    "gnoblin-mutter-devel < 52",
}
PUBLIC_FILES = {
    "/usr/bin/gnoblin",
    "/usr/bin/gnoblinctl",
    "/usr/share/wayland-sessions/gnoblin.desktop",
    "/usr/lib/systemd/user/gnoblin-session.target",
    "/usr/lib/systemd/user/gnoblin-idle.service",
    "/usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy",
    "/usr/share/xdg-desktop-portal/gnoblin-portals.conf",
    "/usr/share/xdg-desktop-portal/portals/gnoblin.portal",
    "/usr/share/dbus-1/services/org.freedesktop.impl.portal.desktop.gnoblin.service",
    "/usr/lib/systemd/user/xdg-desktop-portal-gnoblin.service",
}


def validate(name, files, provides, conflicts, obsoletes):
    if name not in PACKAGES:
        raise ValueError(f"not a side-by-side Gnoblin package: {name}")
    if conflicts.strip():
        raise ValueError(f"{name} declares Conflicts")
    for obsolete in obsoletes.splitlines():
        if name != "gnoblin" or obsolete not in ALLOWED_OBSOLETES:
            raise ValueError(f"{name} declares an unexpected Obsoletes entry: {obsolete}")
    for capability in provides.splitlines():
        if re.match(
            r"(?:mutter|gnome-shell|libmutter|libshell-|libst-|pkgconfig\(|desktop-notification-daemon|PolicyKit-authentication-agent)",
            capability,
        ):
            raise ValueError(f"{name} provides a system capability: {capability}")
    for filename in files.splitlines():
        path = PurePosixPath(filename)
        if ".." in path.parts or not path.is_absolute():
            raise ValueError(f"invalid package path: {filename}")
        if filename in PUBLIC_FILES or filename in ("/usr/lib/gnoblin", "/usr/lib/.build-id"):
            continue
        if filename.startswith(
            ("/usr/lib/gnoblin/", "/usr/lib/.build-id/", f"/usr/share/licenses/{name}/", f"/usr/share/doc/{name}/")
        ):
            continue
        if filename in (f"/usr/share/licenses/{name}", f"/usr/share/doc/{name}"):
            continue
        raise ValueError(f"{name} installs outside Gnoblin's paths: {filename}")


def check_package(path):
    def query(*args):
        return subprocess.check_output(["rpm", "-qp", *args, str(path)], text=True).strip()

    validate(query("--qf", "%{NAME}"), query("--list"), query("--provides"), query("--conflicts"), query("--obsoletes"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("packages", nargs="+")
    args = parser.parse_args()
    for path in args.packages:
        try:
            check_package(path)
        except (ValueError, subprocess.CalledProcessError) as error:
            parser.exit(1, f"Refusing {path}: {error}\n")
    print("PASS: RPM names, file paths and dependency metadata are isolated")


if __name__ == "__main__":
    main()
