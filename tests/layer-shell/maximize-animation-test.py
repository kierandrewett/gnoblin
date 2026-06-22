#!/usr/bin/env python3
# Regression: maximize uses its own configurable frame-lerp animation instead
# of snapping directly to the final size or sharing the generic resize knob.
import importlib.util
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

MAX_MID = "/tmp/gnoblin-maximize-animation-mid.png"
MAX_END = "/tmp/gnoblin-maximize-animation-end.png"
RESTORE_MID = "/tmp/gnoblin-unmaximize-animation-mid.png"
RESTORE_END = "/tmp/gnoblin-unmaximize-animation-end.png"


def is_window_pixel(path, x=32, y=80):
    r, g, b = Image.open(path).convert("RGB").load()[x, y]
    # The default background is bluish (#202434). foot's window/body at this
    # coordinate is neutral grey/black, so blue stops dominating red.
    return (b - r) < 10


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    try:
        dk.extra_conf = (
            "[animations]\n"
            "enabled = true\n"
            "maximize = 2000, linear\n"
            "unmaximize = 2000, linear\n"
        )
        dk.boot(with_monitor=True)
        time.sleep(2)
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        time.sleep(1)

        dk.dispatch("maximize")
        time.sleep(0.15)
        dk.shot(MAX_MID)
        mid_window = is_window_pixel(MAX_MID)
        time.sleep(2.2)
        dk.shot(MAX_END)
        end_window = is_window_pixel(MAX_END)

        print(f"  top-left sample mid={mid_window} end={end_window}")
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1
        if mid_window:
            print("FAIL: maximize reached the final window footprint before configured animation time")
            return 1
        if not end_window:
            print("FAIL: maximized window did not reach the final footprint")
            return 1

        dk.dispatch("maximize")
        time.sleep(0.15)
        dk.shot(RESTORE_MID)
        restore_mid_window = is_window_pixel(RESTORE_MID)
        time.sleep(2.2)
        dk.shot(RESTORE_END)
        restore_end_window = is_window_pixel(RESTORE_END)

        print(f"  restore top-left sample mid={restore_mid_window} end={restore_end_window}")
        if not restore_mid_window:
            print("FAIL: unmaximize reached the restored footprint before configured animation time")
            return 1
        if restore_end_window:
            print("FAIL: unmaximized window did not leave the maximized footprint")
            return 1

        print("PASS: maximize/unmaximize animations lerp between restored and maximized frames")
        return 0
    finally:
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
