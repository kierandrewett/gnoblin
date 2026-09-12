#!/usr/bin/env python3
"""Reject Gnoblin RPMs that could replace system GNOME files or providers."""

import argparse
from pathlib import PurePosixPath
import re
import subprocess


PACKAGES = {"gnoblin-mutter", "gnoblin-mutter-devel", "gnoblin-shell", "gnoblin-session"}
PUBLIC_FILES = {
    "/usr/bin/gnoblinctl",
    "/usr/share/wayland-sessions/gnoblin.desktop",
    "/usr/share/gnome-session/sessions/gnoblin.session",
    "/usr/lib/systemd/user/org.gnoblin.Shell.target",
    "/usr/lib/systemd/user/org.gnoblin.Shell@wayland.service",
    "/usr/lib/systemd/user/gnome-session@gnoblin.target.d",
    "/usr/lib/systemd/user/gnome-session@gnoblin.target.d/gnoblin.conf",
    "/usr/share/polkit-1/actions/org.gnoblin.mutter.backlight-helper.policy",
}


def validate(name, files, provides, conflicts, obsoletes):
    if name not in PACKAGES:
        raise ValueError(f"not a side-by-side Gnoblin package: {name}")
    if conflicts.strip() or obsoletes.strip():
        raise ValueError(f"{name} declares Conflicts or Obsoletes")
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
