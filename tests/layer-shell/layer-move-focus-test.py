#!/usr/bin/env python3
# Regression: moving a layer-shell surface under a stationary pointer must
# refresh compositor hit testing. Otherwise clicks keep going to the old focused
# surface until the user moves the mouse.
import importlib.util
import os
import pathlib
import subprocess
import sys
import time

from gi.repository import Gio, GLib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def wait_for_text(path, needle, timeout=3.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        text = path.read_text(errors="replace") if path.exists() else ""
        if needle in text:
            return text
        time.sleep(0.05)
    return path.read_text(errors="replace") if path.exists() else ""


def button_count(path):
    text = path.read_text(errors="replace") if path.exists() else ""
    return sum(1 for line in text.splitlines() if line.startswith("BUTTON "))


def pointer_button(dk, button=272):
    dk.start_remote_desktop(pointer=True)
    for pressed in (True, False):
        dk._rd_session.call_sync(
            "NotifyPointerButton",
            GLib.Variant("(ib)", (button, pressed)),
            Gio.DBusCallFlags.NONE,
            -1,
            None,
        )
        time.sleep(0.05)


def reliable_move(dk, x, y):
    dk.move(x, y)
    time.sleep(0.1)
    dk.move(x, y)
    time.sleep(0.1)


def main():
    client = os.environ.get("GNOBLIN_LAYER_MOVE_FOCUS_CLIENT")
    if not client:
        print("FAIL: GNOBLIN_LAYER_MOVE_FOCUS_CLIENT is not set")
        return 1

    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    try:
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        log_path = dk.tmp / "layer-move-focus-client.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [client],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        text = wait_for_text(log_path, "MAPPED", timeout=4.0)
        if "MAPPED" not in text:
            print("FAIL: layer-move-focus client did not map")
            print(text[-1000:])
            return 1

        reliable_move(dk, 40, 40)
        pointer_button(dk)
        text = wait_for_text(log_path, "MOVED", timeout=3.0)
        first = button_count(log_path)
        print(f"  after first click buttons={first}")
        if "MOVED" not in text or first != 1:
            print("FAIL: first click did not reach and move the layer surface")
            print(text[-1000:])
            return 1

        # No pointer motion here. The surface moved from y=0 to y=220, so a
        # second click at the old coordinate must not be delivered.
        pointer_button(dk)
        time.sleep(0.5)
        after_old = button_count(log_path)
        print(f"  after stationary old-position click buttons={after_old}")
        if after_old != first:
            print("FAIL: moved layer surface still received a stale-position click")
            print(log_path.read_text(errors="replace")[-1000:])
            return 1

        reliable_move(dk, 40, 260)
        pointer_button(dk)
        deadline = time.time() + 3.0
        while time.time() < deadline and button_count(log_path) <= after_old:
            time.sleep(0.05)
        after_new = button_count(log_path)
        print(f"  after moved-position click buttons={after_new}")
        if after_new <= after_old:
            print("FAIL: moved layer surface did not receive a click at its new position")
            print(log_path.read_text(errors="replace")[-1000:])
            return 1

        if proc.poll() is not None:
            print(f"FAIL: layer-move-focus client exited early rc={proc.returncode}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: layer-shell geometry moves refresh pointer focus")
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        if logf:
            logf.close()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
