#!/usr/bin/env python3
# Regression test for snap-region geometry.
#
# The `snap <region>` action places a window into a fraction of the monitor WORK
# AREA (built-in regions: left/right/top/bottom, the four corners, center). This
# guards that each region puts the window where it belongs — catching work-area
# regressions (e.g. ignoring the topbar strut) or wrong-region arithmetic.
#
# Method (robust, shadow-immune): screenshot the empty desktop, then for each
# region snap a real window and assert a point INSIDE the region changed (window
# present) while a point well OUTSIDE it did NOT (still desktop). Bottom-capable
# regions also sample inside the dock's exclusive band, away from centered dock
# icons, so a window placed behind the dock fails even if coarse region points
# still look right. Uses a low diff threshold because an empty terminal's
# background is close to the desktop colour (grim PNGs are lossless, so there's
# no compression noise to fear).
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

# region -> (in-region point, out-of-region point); work area is y:34..800.
REGIONS = {
    "left":         ((320, 400), (960, 400)),
    "right":        ((960, 400), (320, 400)),
    "top":          ((640, 150), (640, 650)),
    "bottom":       ((640, 650), (640, 150)),
    "top-left":     ((320, 150), (960, 650)),
    "top-right":    ((960, 150), (320, 650)),
    "bottom-left":  ((320, 650), (960, 150)),
    "bottom-right": ((960, 650), (320, 150)),
    "center":       ((640, 400), (100, 400)),
}

DOCK_GUARDS = {
    "left": [(100, 740)],
    "right": [(1180, 740)],
    "bottom": [(100, 740), (1180, 740)],
    "bottom-left": [(100, 740)],
    "bottom-right": [(1180, 740)],
}


def load(p):
    return Image.open(p).convert("RGB").load()


def changed(a, b, x, y, thr=10):
    return sum(abs(p - q) for p, q in zip(a[x, y], b[x, y])) > thr


def main():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        empty_png = "/tmp/gnoblin-snap-empty.png"
        dk.shot(empty_png)
        empty = load(empty_png)
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        bad = []
        for name, (inp, outp) in REGIONS.items():
            dk.dispatch("snap", name)
            time.sleep(1.0)
            shot = f"/tmp/gnoblin-snap-{name}.png"
            dk.shot(shot)
            cur = load(shot)
            in_win = changed(empty, cur, *inp)
            out_win = changed(empty, cur, *outp)
            dock_hits = [pt for pt in DOCK_GUARDS.get(name, []) if changed(empty, cur, *pt)]
            ok = in_win and not out_win and not dock_hits
            print(f"  snap {name:13s} in={'win' if in_win else 'desk'} "
                  f"out={'win' if out_win else 'desk'} "
                  f"dock={'win' if dock_hits else 'desk'} -> {'ok' if ok else 'BAD'}")
            if not ok:
                bad.append(name)
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if bad:
            print(f"FAIL: snap regions misplaced: {bad}")
            return 1
        print("PASS: all snap regions place the window correctly")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
