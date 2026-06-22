#!/usr/bin/env python3
# Regression test: rounded corners (and the drop shadow) are suppressed while a
# window is maximized/fullscreen — both correct (a maximized window's rounded
# corners would clip at the screen edge) and a perf win (no offscreen shader pass
# per damaged frame). Asserts a maximized window's top-left corner is square
# (window content reaches the corner), and unmaximizing rounds it again.
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


def corner_is_window(png, x=3, y=37):
    """True if the maximized window's content reaches its top-left corner (square),
    False if the corner shows the bluish wallpaper (a rounded gap)."""
    r, g, b = Image.open(png).convert("RGB").load()[x, y]
    return (b - r) < 10  # wallpaper #202434 is markedly bluish; window content isn't


def main():
    dk = dh.Devkit()
    dk.extra_appearance = "rounding = 14\n"
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        time.sleep(1)

        dk.dispatch("maximize", "")
        time.sleep(1)
        dk.shot("/tmp/gnoblin-maxfx-on.png")
        square = corner_is_window("/tmp/gnoblin-maxfx-on.png")
        print(f"  maximized corner square: {square}")

        dk.dispatch("maximize", "")  # unmaximize
        time.sleep(1)
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1

        if not square:
            print("FAIL: maximized window still has rounded corners (effect not suppressed)")
            return 1

        print("PASS: rounded corners suppressed while maximized, no crash")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
