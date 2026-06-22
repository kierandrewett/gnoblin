#!/usr/bin/env python3
# Regression: full-screen shell-ui clients must use the configured layer-surface
# height when output metadata is not populated yet. A previous fallback used
# 800px unconditionally, so on a 1280x600 output the power menu was vertically
# centered around y=400 instead of y=300.
import importlib.util
import pathlib
import subprocess
import sys
import time

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def changed_bbox(path):
    im = Image.open(path).convert("RGB")
    bg = im.getpixel((10, 10))
    xs = []
    ys = []
    for y in range(im.height):
        for x in range(im.width):
            pix = im.getpixel((x, y))
            if sum(abs(a - b) for a, b in zip(pix, bg)) > 45:
                xs.append(x)
                ys.append(y)
    if not xs:
        return None
    return min(xs), min(ys), max(xs), max(ys)


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    try:
        dk.boot(monitors=["1280x600"], per_output=False)
        time.sleep(2)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_POWER_DRYRUN"] = "1"
        power_menu = dh.PREFIX / "bin" / "gnoblin-power-menu"
        proc = subprocess.Popen(
            [str(power_menu)],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        time.sleep(2)

        shot = "/tmp/gnoblin-power-menu-output-size.png"
        dk.shot(shot)
        bbox = changed_bbox(shot)
        if bbox is None:
            print("FAIL: power menu did not render")
            return 1

        x1, y1, x2, y2 = bbox
        cx = (x1 + x2) / 2.0
        cy = (y1 + y2) / 2.0
        print(f"  power menu bbox=({x1},{y1})-({x2},{y2}) center=({cx:.1f},{cy:.1f})")

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if abs(cx - 640) > 30:
            print("FAIL: power menu is not horizontally centered on the output")
            return 1
        if abs(cy - 300) > 35:
            print("FAIL: power menu used the wrong output height for centering")
            return 1

        print("PASS: power menu uses the configured output size before output metadata settles")
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
