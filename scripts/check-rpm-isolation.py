#!/usr/bin/env python3
"""Reject Gnoblin RPMs that could replace system GNOME files or providers."""

import argparse
import json
from pathlib import Path, PurePosixPath
import re
import stat
import subprocess


PACKAGES = {
    "gnoblin",
    "xdg-desktop-portal-gnoblin",
    "gnoblin-geoclue-integration",
    "gnoblin-ananicy-rules",
}
# The two configuration packages ship one directory of files each.
CONFIG_FILES = {
    "gnoblin-geoclue-integration": ("/etc/geoclue/conf.d/",),
    "gnoblin-ananicy-rules": ("/etc/ananicy.d/",),
}
GNOME_MAJOR = json.loads((Path(__file__).resolve().parents[1] / "src/gnome-versions.json").read_text())["major"]
ALLOWED_OBSOLETES = {
    f"gnoblin-session < {GNOME_MAJOR}",
    f"gnoblin-shell < {GNOME_MAJOR}",
    # The separate Mutter packages were merged into gnoblin. The session package replaces them on upgrade.
    "gnoblin-mutter < 52",
    "gnoblin-mutter-devel < 52",
}
# The manual pages of the gnoblin package. The package build compresses them, and the extension depends on the
# distribution: .gz from brp-compress, .zst or .xz elsewhere.
MAN_PAGE = re.compile(r"^/usr/share/man/man1/gnoblin(?:ctl)?\.1(?:\.(?:gz|zst|xz|bz2))?$")
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


def validate(name, files, provides, conflicts, obsoletes, requires=""):
    if name not in PACKAGES:
        raise ValueError(f"not a side-by-side Gnoblin package: {name}")
    # A pkgconfig() requirement names a -devel package. The private runtime ships Mutter's .pc files, and RPM would
    # otherwise add one for every library they list, so installing Gnoblin would pull development packages onto the system.
    for requirement in requires.splitlines():
        if requirement.startswith("pkgconfig("):
            raise ValueError(f"{name} requires a development package through {requirement}")
    if conflicts.strip():
        raise ValueError(f"{name} declares Conflicts")
    for obsolete in obsoletes.splitlines():
        # The openSUSE spec obsoletes every earlier gnoblin-session build, so its version is the package's own.
        if name != "gnoblin" or (obsolete not in ALLOWED_OBSOLETES and not obsolete.startswith("gnoblin-session <= ")):
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
        if name == "gnoblin" and MAN_PAGE.match(filename):
            continue
        if filename.startswith(
            ("/usr/lib/gnoblin/", "/usr/lib/.build-id/", f"/usr/share/licenses/{name}/", f"/usr/share/doc/{name}/")
            + CONFIG_FILES.get(name, ())
        ):
            continue
        if filename in tuple(prefix.rstrip("/") for prefix in CONFIG_FILES.get(name, ())):
            continue
        if filename in (f"/usr/share/licenses/{name}", f"/usr/share/doc/{name}"):
            continue
        raise ValueError(f"{name} installs outside Gnoblin's paths: {filename}")


def check_package(path):
    def query(*args):
        return subprocess.check_output(["rpm", "-qp", *args, str(path)], text=True).strip()

    # The files of the package. A folder that the package shares with the system, such as /usr/share/dbus-1, is not a
    # file, and the stock package that owns it owns it too.
    files = "\n".join(
        line.split(" ", 1)[1]
        for line in query("--qf", "[%{FILEMODES} %{FILENAMES}\\n]").splitlines()
        if not stat.S_ISDIR(int(line.split(" ", 1)[0]))
    )
    validate(
        query("--qf", "%{NAME}"),
        files,
        query("--provides"),
        query("--conflicts"),
        query("--obsoletes"),
        query("--requires"),
    )


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
