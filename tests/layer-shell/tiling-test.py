#!/usr/bin/env python3
# Regression test for keyboard tile-cycling + gaps (Phase 2 richer snap).
#
# `tile <dir>` cycles a window through half -> two-thirds -> one-third on that
# side (KDE-like), and `[appearance] gaps` insets tiled windows. This boots with
# a gap, tiles a window left three times, and asserts (via the window's detected
# edges) that the gap is honoured, the width cycles, and the tile stays inside
# the topbar/dock work area.
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

GAP = 12
TOPBAR_H = 34
DOCK_H = 96


def edges(png, y=400):
    """(left, right) x of the tiled window: the terminal body is neutral grey
    (|b-r| small), the desktop is bluish (b-r large)."""
    px = Image.open(png).convert("RGB").load()
    left = None
    for x in range(0, 640):
        r, g, b = px[x, y]
        if abs(b - r) < 10 and abs(g - r) < 10:
            left = x
            break
    right = None
    for x in range(max(60, (left or 60)), 1280):
        r, g, b = px[x, y]
        if abs(b - r) < 10 and abs(g - r) < 10:
            right = x
        elif right and x > right + 3 and (b - r) > 12:
            break
    return left, right


def vertical_edges(png, x=320):
    im = Image.open(png).convert("RGB")
    px = im.load()
    _, h = im.size

    top = None
    for y in range(0, h // 2):
        r, g, b = px[x, y]
        if r > 80 and g > 80 and b > 80:
            top = y
            break

    bottom = None
    for y in range(h - 1, h // 2, -1):
        r, g, b = px[x, y]
        if abs(b - r) < 9 and abs(g - r) < 9 and r > 20:
            bottom = y
            break

    return top, bottom, h


def main():
    dk = dh.Devkit()
    # This test's vertical edge detector intentionally uses simple pixel
    # thresholds. Keep the background dark and flat so a bright photo wallpaper
    # cannot be mistaken for the top of the tiled terminal.
    dk.extra_appearance = (
        'background = "#202434"\n'
        "wallpaper = /tmp/gnoblin-tiling-test-missing-wallpaper\n"
        f"gaps = {GAP}\n"
    )
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        time.sleep(1)
        widths = []
        lefts = []
        tops = []
        bottoms = []
        for i in range(3):
            dk.dispatch("tile", "left")
            time.sleep(0.8)
            shot = f"/tmp/gnoblin-tile-{i}.png"
            dk.shot(shot)
            l, r = edges(shot)
            t, b, h = vertical_edges(shot)
            print(f"  tile left #{i + 1}: left={l} right={r} top={t} bottom={b}")
            if l is None or r is None or t is None or b is None:
                print("FAIL: could not detect the tiled window")
                return 1
            lefts.append(l)
            widths.append(r - l)
            tops.append(t)
            bottoms.append(b)
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        # Gap honoured: the left edge sits ~GAP from the screen edge, not at 0.
        if not all(abs(l - GAP) <= 6 for l in lefts):
            print(f"FAIL: gap not applied (left edges {lefts}, expected ~{GAP})")
            return 1
        expected_top = TOPBAR_H + GAP
        max_bottom = h - DOCK_H - GAP
        if not all(t >= expected_top - 6 for t in tops):
            print(f"FAIL: tile overlapped topbar/gap (top edges {tops}, expected >= {expected_top})")
            return 1
        if not all(b <= max_bottom + 6 for b in bottoms):
            print(f"FAIL: tile overlapped dock/gap (bottom edges {bottoms}, expected <= {max_bottom})")
            return 1
        # Cycle: #1 half, #2 wider (two-thirds), #3 narrower (one-third).
        if not (widths[1] > widths[0] > widths[2]):
            print(f"FAIL: tile cycle widths not half>... ({widths})")
            return 1
        print("PASS: tile-cycling (half/two-thirds/one-third) + gaps work")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
