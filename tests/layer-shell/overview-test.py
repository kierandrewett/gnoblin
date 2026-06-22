#!/usr/bin/env python3
# Regression test for the Overview (Phase 3 Activities).
#
# `overview` toggles a full-screen, opaque dark backdrop holding a live
# ClutterClone thumbnail of every window on the active workspace. This boots,
# spawns three terminals, opens the Overview and asserts (via pixel check) that
# the opaque backdrop covers most of the screen, then toggles it shut and
# asserts the desktop is restored. Real windows must be untouched throughout.
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

# The Overview backdrop colour (gnoblin-overview.cpp: COGL_COLOR_INIT(16,16,20,255)).
BACKDROP = (16, 16, 20)


def backdrop_fraction(png):
    """Fraction of a sampled grid that is the opaque Overview backdrop colour."""
    im = Image.open(png).convert("RGB")
    w, h = im.size
    px = im.load()
    hit = total = 0
    for yi in range(8, 40):
        y = yi * h // 48
        for xi in range(2, 62):
            x = xi * w // 64
            r, g, b = px[x, y]
            if abs(r - BACKDROP[0]) < 8 and abs(g - BACKDROP[1]) < 8 and abs(b - BACKDROP[2]) < 8:
                hit += 1
            total += 1
    return hit / total if total else 0.0


def main():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        for _ in range(3):
            if not dk.spawn_and_wait("foot"):
                print("FAIL: foot did not map")
                return 1
            time.sleep(0.5)
        time.sleep(1)

        # list_windows() rows are (id, title, app_id, focused, minimized).
        before = dk.list_windows()
        n_before = len([w for w in before if w[2] == "foot"])
        if n_before < 3:
            print(f"FAIL: expected 3 foot windows, saw {n_before}: {before}")
            return 1

        # Closed: backdrop colour should be essentially absent (bluish wallpaper).
        dk.shot("/tmp/gnoblin-ov-closed1.png")
        closed_frac = backdrop_fraction("/tmp/gnoblin-ov-closed1.png")
        print(f"  closed backdrop fraction: {closed_frac:.2f}")

        # Open the Overview.
        dk.dispatch("overview", "")
        time.sleep(1.0)
        dk.shot("/tmp/gnoblin-ov-open.png")
        open_frac = backdrop_fraction("/tmp/gnoblin-ov-open.png")
        print(f"  open backdrop fraction:   {open_frac:.2f}")

        # Toggle shut again; the desktop should be restored.
        dk.dispatch("overview", "")
        time.sleep(1.0)
        dk.shot("/tmp/gnoblin-ov-closed2.png")
        reclosed_frac = backdrop_fraction("/tmp/gnoblin-ov-closed2.png")
        print(f"  reclosed backdrop fraction: {reclosed_frac:.2f}")

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1

        # The Overview must cover a large part of the screen when open and
        # almost none of it when closed.
        if open_frac < 0.30:
            print(f"FAIL: Overview backdrop not visible when open ({open_frac:.2f})")
            return 1
        if closed_frac > 0.10 or reclosed_frac > 0.10:
            print(f"FAIL: backdrop lingered while closed "
                  f"(closed {closed_frac:.2f}, reclosed {reclosed_frac:.2f})")
            return 1

        # The real windows must be untouched by opening/closing the Overview.
        after = dk.list_windows()
        n_after = len([w for w in after if w[2] == "foot"])
        if n_after != n_before:
            print(f"FAIL: window count changed across Overview ({n_before} -> {n_after})")
            return 1

        print("PASS: Overview opens with an opaque backdrop, toggles shut, no crash")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
