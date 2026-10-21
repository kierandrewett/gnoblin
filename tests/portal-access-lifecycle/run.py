#!/usr/bin/env python3
"""Build and probe generic Access on a private bus and Broadway display."""

import argparse
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
UPSTREAM = ROOT / "subprojects/xdg-desktop-portal-gnome"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--upstream", type=Path, default=UPSTREAM, help="prepared portal source tree before patch 49")
    parser.add_argument("--source", type=Path, help="override access.c in the prepared source tree")
    parser.add_argument("--baseline", action="store_true")
    args = parser.parse_args()
    upstream = args.upstream.resolve()
    source = args.source.resolve() if args.source else upstream / "src/access.c"
    work = args.output.resolve()
    work.mkdir(exist_ok=False)
    for name in ("runtime", "config", "data", "cache", "tmp"):
        (work / name).mkdir(mode=0o700)
    # No service directories: GTK cannot auto-start unrelated desktop services.
    (work / "bus.conf").write_text(
        "<busconfig><type>session</type><listen>unix:tmpdir=" + str(work / "runtime") + "</listen>"
        '<auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/>'
        '<allow receive_sender="*"/><allow own="*"/></policy></busconfig>'
    )
    shutil.copy2(source, work / "access.c")
    if not args.baseline:
        patch = (
            ROOT
            / "patches/xdg-desktop-portal-gnome/49-access-dialog-lifecycle/0001-release-dialogs-with-their-requests.patch"
        )
        subprocess.run(["patch", "--batch", "--forward", str(work / "access.c"), str(patch)], check=True)
    (work / "config.h").write_text('#define GETTEXT_PACKAGE "gnoblin"\n')
    subprocess.run(
        [
            "gdbus-codegen",
            "--generate-c-code",
            str(work / "xdg-desktop-portal-dbus"),
            "--interface-prefix",
            "org.freedesktop.impl.portal.",
            "--c-namespace",
            "XdpImpl",
            str(ROOT / "src/portal/interfaces/org.freedesktop.impl.portal.Request.xml"),
            str(ROOT / "src/portal/interfaces/org.freedesktop.impl.portal.Access.xml"),
            str(ROOT / "src/portal/interfaces/org.freedesktop.impl.portal.Screenshot.xml"),
        ],
        check=True,
    )
    flags = shlex.split(
        subprocess.check_output(["pkg-config", "--cflags", "--libs", "libadwaita-1", "gio-unix-2.0"], text=True)
    )
    command = [
        os.environ.get("CC", "clang"),
        "-g",
        "-Wall",
        "-Wextra",
        "-Werror",
        "-Wno-unused-parameter",
        "-Wno-deprecated-declarations",
        "-fsanitize=address,undefined",
        "-fno-sanitize=function",
        "-fno-omit-frame-pointer",
        "-I" + str(work),
        "-I" + str(ROOT / "src/portal/backend"),
        "-I" + str(upstream / "src"),
        "-I" + str(upstream / "subprojects/libgxdp/src"),
        str(HERE / "backend.c"),
        str(upstream / "src/request.c"),
        str(ROOT / "src/portal/backend/gnoblin-portal-identity.c"),
        str(work / "xdg-desktop-portal-dbus.c"),
        *flags,
        "-o",
        str(work / "backend"),
    ]
    subprocess.run(command, check=True)
    env = dict(
        os.environ,
        XDG_RUNTIME_DIR=str(work / "runtime"),
        XDG_CONFIG_HOME=str(work / "config"),
        XDG_DATA_HOME=str(work / "data"),
        XDG_CACHE_HOME=str(work / "cache"),
        TMPDIR=str(work / "tmp"),
        GDK_BACKEND="broadway",
        BROADWAY_DISPLAY=":37",
        GTK_A11Y="none",
        GIO_USE_VFS="local",
        GSETTINGS_BACKEND="memory",
        ASAN_OPTIONS="detect_leaks=0",
        PYTHONDONTWRITEBYTECODE="1",
    )
    (work / "build-command.txt").write_text(shlex.join(command) + "\n")
    with (work / "run.log").open("w") as log:
        result = subprocess.run(
            [
                "dbus-run-session",
                "--config-file=" + str(work / "bus.conf"),
                "--",
                sys.executable,
                str(HERE / "probe.py"),
                str(work),
                *(["--baseline"] if args.baseline else []),
            ],
            env=env,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
    print((work / "run.log").read_text())
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
