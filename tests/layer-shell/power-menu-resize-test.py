#!/usr/bin/env python3
# Regression: full-screen shell-ui clients must recompute app-level geometry when
# the layer surface is reconfigured after map. A previous runtime resized the
# Slint window but left the power menu's cached screen size at the old monitor
# dimensions, so after 1280x800 -> 1024x600 the menu stayed around (640, 400).
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


def changed_bbox(path, width, height):
    im = Image.open(path).convert("RGB")
    bg = im.getpixel((10, 10))
    xs = []
    ys = []
    for y in range(min(height, im.height)):
        for x in range(min(width, im.width)):
            pix = im.getpixel((x, y))
            if sum(abs(a - b) for a, b in zip(pix, bg)) > 40:
                xs.append(x)
                ys.append(y)
    if not xs:
        return None
    return min(xs), min(ys), max(xs), max(ys)


def bbox_center(path, width, height):
    bbox = changed_bbox(path, width, height)
    if bbox is None:
        return None
    x1, y1, x2, y2 = bbox
    return bbox, (x1 + x2) / 2.0, (y1 + y2) / 2.0


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    try:
        dk.boot(with_monitor=False, per_output=False)
        if not dk.add_monitor_late(1280, 800):
            print("SKIP: virtual monitor never materialized")
            return 0

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

        before = "/tmp/gnoblin-power-menu-before-resize.png"
        dk.shot(before)
        before_result = bbox_center(before, 1280, 800)
        if before_result is None:
            print("FAIL: power menu did not render before resize")
            return 1
        before_bbox, before_cx, before_cy = before_result
        print(
            f"  before resize bbox={before_bbox} center=({before_cx:.1f},{before_cy:.1f})"
        )

        if not dk.resize_storm([(1024, 600)], dk._sc_node):
            print(f"FAIL: compositor crashed during resize: {dk.crashed()}")
            return 1
        time.sleep(3)
        if proc.poll() is not None:
            print("FAIL: power menu exited during monitor resize")
            return 1

        after = "/tmp/gnoblin-power-menu-after-resize.png"
        dk.shot(after)
        after_result = bbox_center(after, 1024, 600)
        if after_result is None:
            print("FAIL: power menu did not render after resize")
            return 1
        after_bbox, after_cx, after_cy = after_result
        print(
            f"  after resize bbox={after_bbox} center=({after_cx:.1f},{after_cy:.1f})"
        )

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if abs(before_cx - 640) > 30 or abs(before_cy - 400) > 35:
            print("FAIL: power menu was not initially centered on 1280x800")
            return 1
        if abs(after_cx - 512) > 30 or abs(after_cy - 300) > 35:
            print("FAIL: power menu did not recenter after layer-surface resize")
            return 1

        print("PASS: power menu recenters when the output is resized after map")
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
