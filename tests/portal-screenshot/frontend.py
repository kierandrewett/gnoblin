#!/usr/bin/env python3
"""Real xdg-desktop-portal frontend -> production backend with synthetic pixels.

All services, permission data, runtime files and bus live in a temporary test
session. This is not a VM or GUI proof.
"""

import argparse
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
from gi.repository import Gio, GLib

HERE = Path(__file__).resolve().parent


def inside(backend):
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    processes = []
    work = Path(os.environ["XDG_RUNTIME_DIR"])
    log = (work / "frontend.log").open("w+")

    def wait_owner(name):
        for _ in range(200):
            reply = bus.call_sync(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                "NameHasOwner",
                GLib.Variant("(s)", (name,)),
                None,
                Gio.DBusCallFlags.NONE,
                1000,
                None,
            )
            if reply.unpack()[0]:
                return
            time.sleep(0.025)
        log.seek(0)
        raise AssertionError((name, log.read()))

    try:
        for command, name in (
            (["/usr/libexec/xdg-permission-store"], "org.freedesktop.impl.portal.PermissionStore"),
            ([str(backend)], "org.freedesktop.impl.portal.desktop.gnoblin"),
        ):
            processes.append(subprocess.Popen(command, stdout=log, stderr=log))
            wait_owner(name)
        # This is the temporary store on this private bus, not the user's.
        bus.call_sync(
            "org.freedesktop.impl.portal.PermissionStore",
            "/org/freedesktop/impl/portal/PermissionStore",
            "org.freedesktop.impl.portal.PermissionStore",
            "SetPermission",
            GLib.Variant("(sbssas)", ("screenshot", True, "screenshot", "", ["yes"])),
            None,
            Gio.DBusCallFlags.NONE,
            2000,
            None,
        )
        processes.append(subprocess.Popen(["/usr/libexec/xdg-desktop-portal", "--verbose"], stdout=log, stderr=log))
        wait_owner("org.freedesktop.portal.Desktop")
        for name, extra in (("success", []), ("closed", ["--expect", "closed", "--close-after-ms", "50"])):
            before = set(Path(os.environ["TMPDIR"]).glob("gnoblin-screenshot-*"))
            result = subprocess.run(
                [sys.executable, str(HERE / "request.py"), str(work / name), *extra],
                capture_output=True,
                text=True,
                timeout=12,
            )
            if result.returncode:
                log.seek(0)
                raise AssertionError((result.stdout, result.stderr, log.read()))
            if name == "closed":
                assert set(Path(os.environ["TMPDIR"]).glob("gnoblin-screenshot-*")) == before, (
                    "cancelled request left a PNG"
                )
            print("PASS public Screenshot/Request", name, result.stdout)
    finally:
        for process in reversed(processes):
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=3)
        log.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("backend", type=Path)
    parser.add_argument("--inside", action="store_true")
    args = parser.parse_args()
    if args.inside:
        inside(args.backend)
        return
    with tempfile.TemporaryDirectory(prefix="gnoblin-portal-frontend-") as tmp:
        root = Path(tmp)
        for name in ("runtime", "config", "data", "cache", "portals"):
            (root / name).mkdir(mode=0o700)
        (root / "portals/gnoblin.portal").write_text(
            "[portal]\nDBusName=org.freedesktop.impl.portal.desktop.gnoblin\nInterfaces=org.freedesktop.impl.portal.Screenshot;org.freedesktop.impl.portal.Access;\nUseIn=gnoblin\n"
        )
        (root / "portals/portals.conf").write_text(
            "[preferred]\ndefault=none\norg.freedesktop.impl.portal.Screenshot=gnoblin\norg.freedesktop.impl.portal.Access=gnoblin\n"
        )
        env = dict(
            os.environ,
            XDG_RUNTIME_DIR=str(root / "runtime"),
            XDG_CONFIG_HOME=str(root / "config"),
            XDG_DATA_HOME=str(root / "data"),
            XDG_CACHE_HOME=str(root / "cache"),
            XDG_DESKTOP_PORTAL_DIR=str(root / "portals"),
            XDG_CURRENT_DESKTOP="gnoblin",
            GIO_USE_VFS="local",
            TMPDIR=tmp,
        )
        subprocess.run(
            ["dbus-run-session", "--", sys.executable, __file__, str(args.backend.resolve()), "--inside"],
            env=env,
            check=True,
            timeout=30,
        )


if __name__ == "__main__":
    main()
