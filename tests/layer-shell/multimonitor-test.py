#!/usr/bin/env python3
# Regression test for multi-monitor behaviour.
#
# Boots gnoblin-shell with TWO side-by-side virtual outputs and asserts:
#   1. both monitors exist, laid out side by side (Meta-0 @ x=0, Meta-1 @ x=1280);
#   2. `exec_per_output` spawned one topbar + one dock PER monitor (2 each);
#   3. maximizing a window fills only ITS monitor, not the whole 2560px desktop
#      (a classic multi-monitor bug would span both);
#   4. no compositor crash.
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

from gi.repository import Gio, GLib


def load(p):
    return Image.open(p).convert("RGB").load()


def changed(a, b, x, y, thr=10):
    return sum(abs(p - q) for p, q in zip(a[x, y], b[x, y])) > thr


def wait_for_per_output_clients(dk):
    for _ in range(20):
        topbars = len(dk.processes("gnoblin-topbar --output Meta-"))
        docks = len(dk.processes("gnoblin-dock --output Meta-"))
        if topbars == 2 and docks == 2:
            return topbars, docks
        time.sleep(0.5)
    return topbars, docks


def main():
    dk = dh.Devkit()
    try:
        dk.boot(monitors=["1280x800", "1280x800"])
        time.sleep(7)
        dc = Gio.DBusProxy.new_sync(
            dk.conn, Gio.DBusProxyFlags.NONE, None,
            "org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig",
            "org.gnome.Mutter.DisplayConfig", None)
        _, monitors, logical, _ = dc.call_sync(
            "GetCurrentState", None, Gio.DBusCallFlags.NONE, -1, None).unpack()
        xs = sorted(lm[0] for lm in logical)
        print(f"  monitors={len(monitors)} logical x-positions={xs}")
        if len(monitors) != 2 or xs != [0, 1280]:
            print("FAIL: expected two side-by-side monitors at x=0 and x=1280")
            return 1

        topbars, docks = wait_for_per_output_clients(dk)
        print(f"  topbars={topbars} docks={docks}")
        if topbars != 2 or docks != 2:
            print("FAIL: exec_per_output did not spawn one topbar+dock per monitor")
            return 1

        empty_png = "/tmp/gnoblin-multimon-empty.png"
        dk.shot(empty_png)
        empty = load(empty_png)
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        dk.dispatch("maximize")
        time.sleep(1.5)
        max_png = "/tmp/gnoblin-multimon-max.png"
        dk.shot(max_png)
        cur = load(max_png)
        # window's monitor (left, x<1280) must be covered; the OTHER monitor
        # (right, x>1280) must stay empty desktop — maximize must not span both.
        on_its_monitor = changed(empty, cur, 640, 400)
        on_other_monitor = changed(empty, cur, 1920, 400)
        print(f"  maximize: this-monitor={'win' if on_its_monitor else 'desk'} "
              f"other-monitor={'win' if on_other_monitor else 'desk'}")
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if not on_its_monitor:
            print("FAIL: maximized window not on its monitor")
            return 1
        if on_other_monitor:
            print("FAIL: maximized window spans onto the other monitor")
            return 1
        print("PASS: two per-output panels; maximize respects monitor boundaries")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
