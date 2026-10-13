#!/usr/bin/env python3
"""Build and run isolated portal screenshot fixtures; never install or start a desktop."""

import argparse
import os
from pathlib import Path
import shlex
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
SOURCE = ROOT / "src/portal/backend"
UPSTREAM = ROOT / "subprojects/xdg-desktop-portal-gnome"


def run(*args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def flags(*packages):
    return shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", *packages], text=True))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sanitize", action="store_true")
    parser.add_argument("--keep", type=Path)
    parser.add_argument(
        "--integration", action="store_true", help="also run the real frontend and private PipeWire checks"
    )
    args = parser.parse_args()
    temporary = tempfile.TemporaryDirectory(prefix="gnoblin-142-")
    build = args.keep.resolve() if args.keep else Path(temporary.name)
    build.mkdir(parents=True, exist_ok=True)
    (build / "tmp").mkdir(exist_ok=True)
    (build / "runtime").mkdir(mode=0o700, exist_ok=True)
    env = dict(os.environ, TMPDIR=str(build / "tmp"), GIO_USE_VFS="local", XDG_RUNTIME_DIR=str(build / "runtime"))
    common = [
        os.environ.get("CC", "cc"),
        "-g",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-Wno-unused-parameter",
        "-Wno-deprecated-declarations",
        "-I" + str(SOURCE),
        "-I" + str(build),
    ]
    if args.sanitize:
        common += ["-fsanitize=address,undefined", "-fno-omit-frame-pointer"]
    pixel_flags = flags("gio-unix-2.0", "gdk-pixbuf-2.0", "libpipewire-0.3")
    run(*common, HERE / "pixels.c", *pixel_flags, "-o", build / "pixels")
    run(build / "pixels", env=env)
    run(
        *common,
        HERE / "capture-client.c",
        SOURCE / "gnoblin-screenshot-capture.c",
        *pixel_flags,
        "-o",
        build / "capture",
    )
    run("dbus-run-session", "--", "python3", HERE / "transport.py", build / "capture", env=env)
    run(
        "gdbus-codegen",
        "--generate-c-code",
        build / "xdg-desktop-portal-dbus",
        "--interface-prefix",
        "org.freedesktop.impl.portal.",
        "--c-namespace",
        "XdpImpl",
        ROOT / "src/portal/interfaces/org.freedesktop.impl.portal.Request.xml",
        ROOT / "src/portal/interfaces/org.freedesktop.impl.portal.Screenshot.xml",
        ROOT / "src/portal/interfaces/org.freedesktop.impl.portal.Access.xml",
    )
    (build / "config.h").write_text('#define GETTEXT_PACKAGE "gnoblin"\n')
    run(
        *common,
        "-I" + str(UPSTREAM / "src"),
        "-I" + str(UPSTREAM / "subprojects/libgxdp/src"),
        HERE / "backend-fixture.c",
        SOURCE / "gnoblin-portal-identity.c",
        UPSTREAM / "src/request.c",
        build / "xdg-desktop-portal-dbus.c",
        *flags("gtk4", "gio-unix-2.0"),
        *[
            "-Wl,--wrap=" + name
            for name in (
                "gdk_display_get_default",
                "gdk_display_get_monitors",
                "gdk_monitor_get_geometry",
                "gdk_display_flush",
            )
        ],
        "-o",
        build / "backend",
    )
    run("dbus-run-session", "--", "python3", HERE / "backend.py", build / "backend", env=env)
    if args.integration:
        run("python3", HERE / "frontend.py", build / "backend", env=env)
        for mode in ("success", "cancel", "closed"):
            run("dbus-run-session", "--", "python3", HERE / "pipewire.py", build / "capture", "--mode", mode, env=env)
    temporary.cleanup()


if __name__ == "__main__":
    main()
