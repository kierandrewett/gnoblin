#!/usr/bin/env python3
# Regression test for the maximize/exclusive-zone strut bug.
#
# History: full_height bars (e.g. the topbar) anchored to ALL FOUR edges so their
# in-scene drop-downs could render below the bar. But an all-edges anchor makes a
# layer surface's exclusive_zone ambiguous (which edge?), so mutter reserved no
# strut and the work area stayed the full monitor — maximized windows and
# full-height snaps then OVERLAPPED the topbar (titlebar at y=0). Fix: such bars
# keep their real anchor (TOP|LEFT|RIGHT) + request a huge height the compositor
# clamps to the output, so the exclusive edge resolves and the top 34px is
# reserved.
#
# This test spawns a real window, maximizes it, and asserts through Mutter's
# frame geometry that the window stays inside the layer-shell work area — below
# the topbar and above the dock.
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

TOPBAR_H = 34          # exclusive_zone the topbar reserves
DOCK_H = 96            # exclusive_zone the visible dock band reserves


def main():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        w = dk.spawn_and_wait("foot")
        if not w:
            print("FAIL: foot did not map")
            return 1
        dk.dispatch("maximize")
        time.sleep(1.5)
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        frames = dk.list_window_frames()
        if not frames:
            print("FAIL: no normal window frames reported")
            return 1
        _, x, top, width, height = frames[0]
        bottom = top + height
        max_bottom = dh.MONITOR.split("x", 1)
        screen_h = int(max_bottom[1]) if len(max_bottom) == 2 else 800
        expected_bottom = screen_h - DOCK_H
        print(
            f"  maximized frame={x},{top} {width}x{height}; "
            f"work-area y={TOPBAR_H}..{expected_bottom}"
        )
        # The window must start at/below the reserved strip. Allow a few px slack
        # for shadow/AA; the bug had it at y=0.
        if top < TOPBAR_H - 6:
            print(f"FAIL: maximized window overlaps the topbar (top y={top} < {TOPBAR_H})")
            return 1
        if bottom > expected_bottom + 6:
            print(
                f"FAIL: maximized window overlaps the dock "
                f"(bottom y={bottom} > {expected_bottom})"
            )
            return 1
        print(
            f"PASS: maximized window respects topbar and dock exclusive zones "
            f"(top={top}, bottom={bottom})"
        )
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
