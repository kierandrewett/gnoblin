#!/usr/bin/env python3
# Regression: when a layer-shell client shrinks its input region under a
# stationary pointer, the next click at that old coordinate must not keep going
# to the stale focused client. Moving the pointer used to be required to recover.
import importlib.util
import os
import pathlib
import re
import subprocess
import sys
import time

from gi.repository import Gio, GLib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

BUTTON = re.compile(r"wl_pointer[@#]\d+\.button\(")
IDLE_GAP = float(os.environ.get("GNOBLIN_IDLE_INPUT_GAP", "6.0"))


def button_count(path):
    return len(BUTTON.findall(path.read_text(errors="replace")))


def wait_for_buttons(path, at_least, timeout=3.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        count = button_count(path)
        if count >= at_least:
            return count
        time.sleep(0.05)
    return button_count(path)


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
    # Mirrors Devkit.click(): the first absolute motion after stream setup can
    # be dropped while the screencast transform settles.
    dk.move(x, y)
    time.sleep(0.1)
    dk.move(x, y)
    time.sleep(0.1)


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    proc = None
    logf = None
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["WAYLAND_DEBUG"] = "1"
        env["GNOBLIN_DOCK_MENU"] = "foot"
        log_path = dk.tmp / "dock-menu-wayland.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-dock")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )
        time.sleep(2.0)
        if proc.poll() is not None:
            print(f"FAIL: gnoblin-dock exited early rc={proc.returncode}")
            return 1

        # Dock surface is bottom anchored: 800px monitor - 296px surface = y 504.
        # (100, 560) is in the headroom, outside the auto-opened menu. First
        # click dismisses the menu and shrinks input to the bottom band.
        reliable_move(dk, 100, 560)
        pointer_button(dk)
        after_dismiss = wait_for_buttons(log_path, 2)
        print(f"  after dismiss headroom buttons={after_dismiss}")
        if after_dismiss < 1:
            print("FAIL: open dock menu did not receive dismiss click")
            return 1

        # No motion here: this is the stale-focus case. The coordinate is no
        # longer in the dock input region, so the dock must not see more buttons.
        # Leave the session idle first; pointer motion used to be enough to paper
        # over stale focus/input problems.
        time.sleep(IDLE_GAP)
        pointer_button(dk)
        time.sleep(0.5)
        after_passthrough = button_count(log_path)
        print(f"  after idle stationary headroom buttons={after_passthrough}")
        if after_passthrough != after_dismiss:
            print("FAIL: closed dock still received a stationary headroom click")
            return 1

        # Moving into the real bottom band should focus the dock again, and a
        # later button-only click should still reach it after the session idles.
        reliable_move(dk, 100, 760)
        time.sleep(IDLE_GAP)
        pointer_button(dk)
        after_band = wait_for_buttons(log_path, after_passthrough + 1)
        print(f"  after idle dock band buttons={after_band}")
        if after_band <= after_passthrough:
            print("FAIL: dock band did not receive pointer button after refocus")
            return 1

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: layer-shell input-region shrink refreshes pointer focus after idle")
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
