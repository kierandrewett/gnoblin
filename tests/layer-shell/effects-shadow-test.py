#!/usr/bin/env python3
# Regression test for the compositor effects layer (drop shadows).
#
# gnoblin draws a drop shadow around toplevel windows (the `[appearance] shadow`
# CSS, applied as a ClutterEffect on the window actor). This spawns a floating
# window and asserts the desktop just BELOW it is darkened by the shadow (the
# configured shadow is offset downward, so the strongest darkening is below the
# window). Rounded corners (`rounding`) are also rendered but are harder to
# pixel-assert for a small radius; the shadow is the robust signal that the
# effects layer is alive.
import sys, time, importlib.util, pathlib, subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

CX = 640   # a floating window is centred horizontally


def main():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        empty_png = "/tmp/gnoblin-fx-empty.png"
        dk.shot(empty_png)
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        time.sleep(1.5)
        win_png = "/tmp/gnoblin-fx-win.png"
        dk.shot(win_png)
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        e = Image.open(empty_png).convert("RGB").load()
        w = Image.open(win_png).convert("RGB").load()
        # Find the window's bottom edge: scan down the centre column from inside
        # the window for the transition from neutral terminal grey (|b-r| small)
        # to the bluish desktop/shadow.
        bottom = None
        for y in range(250, 700):
            r, g, b = w[CX, y]
            if not (abs(b - r) < 9 and abs(g - r) < 9):
                bottom = y
                break
        if bottom is None:
            print("FAIL: could not locate the floating window")
            return 1
        samples = []
        for off in (0, 4, 8, 12, 16, 24, 36, 48):
            dy = bottom + off
            samples.append(sum(e[CX, dy]) - sum(w[CX, dy]))
        strong = sum(1 for v in samples[:5] if v > 6)
        fades = samples[0] > samples[2] > samples[4] >= samples[6] >= samples[7]
        print(f"  window bottom y={bottom}; shadow darkening samples={samples}")
        if strong < 4 or not fades:
            print("FAIL: no drop shadow detected below the window")
            return 1
        print("PASS: window drop shadow renders (effects layer alive)")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
