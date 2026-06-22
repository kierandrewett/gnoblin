#!/usr/bin/env python3
# Regression: `[topbar] appmenu-backend` must control the compositor's
# GetActiveWindowMenu result for KDE/DBusMenu addresses advertised via the
# org_kde_kwin_appmenu Wayland protocol.
import importlib.util
import os
import pathlib
import subprocess
import sys
import time

import gi
gi.require_version("Gio", "2.0")
from gi.repository import Gio

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

PROBE = pathlib.Path(os.environ.get("KDE_APPMENU_PROBE", "/tmp/gnoblin-kde-appmenu-probe"))
BUS = "org.gnoblin.TestAppMenu"
PATH = "/org/gnoblin/TestAppMenu"


def wait_for_ready(proc, timeout=8):
    deadline = time.time() + timeout
    lines = []
    while time.time() < deadline:
        line = proc.stdout.readline()
        if line:
            lines.append(line.rstrip())
            if line.startswith("READY "):
                return True, lines
        if proc.poll() is not None:
            return False, lines
        time.sleep(0.05)
    return False, lines


def active_menu(dk):
    return dk.shell_proxy().call_sync(
        "GetActiveWindowMenu",
        None,
        Gio.DBusCallFlags.NONE,
        -1,
        None,
    ).unpack()


def wait_for_menu(dk, want_kind, timeout=5):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        last = active_menu(dk)
        if last[0] == want_kind:
            return last
        time.sleep(0.2)
    return last


def run_case(backend):
    dk = dh.Devkit()
    proc = None
    try:
        dk.extra_conf = (
            "[topbar]\n"
            f"appmenu-backend = {backend}\n"
        )
        dk.boot(with_monitor=True)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        proc = subprocess.Popen(
            [str(PROBE), BUS, PATH],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )
        ready, lines = wait_for_ready(proc)
        if not ready:
            out = "\n".join(lines)
            print(f"FAIL: KDE appmenu probe did not publish address for backend={backend}")
            if out:
                print(out)
            if proc.poll() is not None:
                print(f"  probe rc={proc.returncode}")
            if dk.crashed():
                print(f"  compositor: {dk.crashed()}")
                print(dk._tail())
            return 1

        if backend == "off":
            got = wait_for_menu(dk, "", timeout=2)
            if got[0] != "":
                print(f"FAIL: appmenu-backend=off exposed menu tuple {got!r}")
                return 1
            print("  off -> empty menu tuple")
        else:
            got = wait_for_menu(dk, "dbusmenu")
            if got[0] != "dbusmenu" or got[1] != BUS or got[4] != PATH:
                print(f"FAIL: appmenu-backend={backend} returned {got!r}")
                return 1
            print(f"  {backend} -> {got!r}")

        if dk.crashed():
            print(f"FAIL: compositor crashed for backend={backend}: {dk.crashed()}")
            print(dk._tail())
            return 1
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        dk.teardown()


def main():
    if not PROBE.exists():
        print(f"SKIP: no KDE appmenu probe at {PROBE}")
        return 0

    for backend in ("kde", "off"):
        rc = run_case(backend)
        if rc != 0:
            return rc

    print("PASS: KDE appmenu protocol addresses honor topbar.appmenu-backend")
    return 0


if __name__ == "__main__":
    sys.exit(main())
