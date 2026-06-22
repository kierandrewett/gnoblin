#!/usr/bin/env python3
# Regression test for the visual window switcher (Phase 4 Alt+Tab).
#
# `switcher next/prev` opens a centred panel of live thumbnails with the
# candidate highlighted; it is modal (Enter/Escape/click) and, when opened by a
# held-modifier keybind, commits the moment the modifier is released. This boots
# with `Alt+Tab = switcher next`, spawns three terminals, and checks:
#   1. dispatching `switcher` draws the panel (pixel check) and toggles shut;
#   2. a real held Alt+Tab (injected via RemoteDesktop) moves the focus — i.e.
#      the open -> grab -> commit-on-release path works end to end.
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


def panel_fraction(png):
    """Fraction of a centre grid that is the switcher panel's neutral grey.
    The panel bg (~30,30,36) reads as a near-neutral dark grey with b slightly
    above r; window/wallpaper pixels there do not."""
    im = Image.open(png).convert("RGB")
    w, h = im.size
    px = im.load()
    hit = total = 0
    for yi in range(20, 44):
        y = yi * h // 64
        for xi in range(22, 42):
            x = xi * w // 64
            r, g, b = px[x, y]
            if 18 <= r <= 40 and 18 <= g <= 40 and 24 <= b <= 46 \
                    and abs(r - g) < 6 and 2 <= (b - r) <= 12:
                hit += 1
            total += 1
    return hit / total if total else 0.0


def focused_id(dk):
    return next((w[0] for w in dk.list_windows() if w[3]), None)


def main():
    dk = dh.Devkit()
    dk.extra_conf = "Alt+Tab = switcher next\nAlt+Shift+Tab = switcher prev\n"
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        for _ in range(3):
            if not dk.spawn_and_wait("foot"):
                print("FAIL: foot did not map")
                return 1
            time.sleep(0.5)
        time.sleep(1)

        dk.shot("/tmp/gnoblin-sw-closed1.png")
        closed = panel_fraction("/tmp/gnoblin-sw-closed1.png")

        dk.dispatch("switcher", "next")  # open (modal, no modifier held)
        time.sleep(0.8)
        dk.dispatch("switcher", "next")  # advance the highlight
        time.sleep(0.6)
        dk.shot("/tmp/gnoblin-sw-open.png")
        opened = panel_fraction("/tmp/gnoblin-sw-open.png")

        dk.send_combo("escape")  # dismiss the modal switcher
        time.sleep(0.8)
        dk.shot("/tmp/gnoblin-sw-closed2.png")
        reclosed = panel_fraction("/tmp/gnoblin-sw-closed2.png")

        print(f"  panel fraction: closed={closed:.2f} open={opened:.2f} reclosed={reclosed:.2f}")

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if opened < 0.20:
            print(f"FAIL: switcher panel not visible when open ({opened:.2f})")
            return 1
        if closed > 0.05 or reclosed > 0.05:
            print(f"FAIL: panel lingered while closed ({closed:.2f}/{reclosed:.2f})")
            return 1

        # Held Alt+Tab: open via the keybind, commit on Alt release -> focus moves.
        before = focused_id(dk)
        dk.send_combo("alt+tab")
        time.sleep(1.0)
        after = focused_id(dk)
        print(f"  focus before/after Alt+Tab: {before} -> {after}")
        if dk.crashed():
            print(f"FAIL: compositor crashed on Alt+Tab: {dk.crashed()}")
            return 1
        if before is None or after is None or before == after:
            print("FAIL: held Alt+Tab did not move the focus")
            return 1

        print("PASS: switcher draws, toggles shut, and Alt+Tab commits a focus change")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
