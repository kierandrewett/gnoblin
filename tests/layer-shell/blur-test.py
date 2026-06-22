#!/usr/bin/env python3
# Regression test for blur-behind (Phase 6).
#
# `[appearance] blur = <radius>` makes the Overview draw a Gaussian-blurred clone
# of the live desktop behind a translucent tint (instead of the default opaque
# backdrop). The blur is a ClutterShaderEffect, so it runs on the software
# (llvmpipe) devkit too. This boots with blur on, opens the Overview, and asserts
# the backdrop shows the desktop through the tint — i.e. the blur-behind clone
# composited — rather than the opaque dark of the no-blur path. (Pixel-exact
# blurriness is eyeballed via `just devkit`; here we validate the path + no crash.)
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


def left_strip_rgb(png):
    """Mean colour of a left-edge strip (left of the thumbnail grid) — backdrop."""
    im = Image.open(png).convert("RGB")
    w, h = im.size
    px = im.load()
    vals = [px[x, y] for x in range(5, max(6, w // 36), 2)
            for y in range(int(h * 0.28), int(h * 0.70), 4)]
    n = len(vals) or 1
    return tuple(sum(c[i] for c in vals) / n for i in range(3))


def main():
    dk = dh.Devkit()
    dk.extra_appearance = "blur = 24\n"
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        for _ in range(2):
            if not dk.spawn_and_wait("foot"):
                print("FAIL: foot did not map")
                return 1
            time.sleep(0.5)
        time.sleep(1)

        dk.dispatch("overview", "")
        time.sleep(1.5)
        dk.shot("/tmp/gnoblin-blur-overview.png")
        r, g, b = left_strip_rgb("/tmp/gnoblin-blur-overview.png")
        print(f"  overview backdrop strip rgb: ({r:.0f}, {g:.0f}, {b:.0f})")

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        # Opaque backdrop would be ~(16,16,20); the blurred wallpaper showing
        # through the tint lifts the blue channel well above that.
        if b < 28:
            print(f"FAIL: backdrop looks opaque ({b:.0f}) — blur-behind not composited")
            return 1

        print("PASS: blur-behind composites the desktop into the Overview backdrop, no crash")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
