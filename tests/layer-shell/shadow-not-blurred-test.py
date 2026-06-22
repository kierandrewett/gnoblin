#!/usr/bin/env python3
# Regression for: drop-shadows must NOT be caught in / blurred by the background
# blur. The compositor blur frosts a surface by reading back the framebuffer
# BEHIND it; drop-shadows are sibling actors painted under each window, so a
# window's shadow falling behind a frosted surface used to smear into the frost.
#
# The fix renders shadows in a second pass the blur capture never sees (hidden on
# the stage `before-paint`, re-composited crisply on `after-paint`, clipped to not
# darken its own/upper windows). This test proves two things at once:
#
#  (1) a window's shadow does NOT contaminate the frosted gnoblin DOCK that sits
#      over it — the dock interior is ~the same whether the window casts a big
#      shadow or none; and
#  (2) the shadow STILL renders (on the wallpaper outside the dock), so we are not
#      passing by simply dropping shadows.
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
    import numpy as np
except Exception:
    print("SKIP: no PIL/numpy")
    sys.exit(0)


def boot(tag, shadow):
    dk = dh.Devkit()
    # A big symmetric halo shadow so the floating window's shadow reaches the
    # frosted dock at the bottom. Chrome (dock) frost stays at its default (on).
    sh = "shadow = 0 0 120px 40px rgba(0,0,0,0.98)\n" if shadow else "shadow =\n"
    dk.extra_conf = "[appearance]\n" + sh
    try:
        dk.boot(with_monitor=True)
        time.sleep(6)                 # let the dock chrome come up
        dk.spawn_and_wait("foot")
        time.sleep(1.0)
        # maximize then unmaximize -> a floating window (with shadow) near the dock
        dk.dispatch("maximize", "")
        time.sleep(1.2)
        dk.dispatch("maximize", "")
        time.sleep(1.0)
        dk.spawn_and_wait("foot")
        time.sleep(2.0)
        out = f"/tmp/gnoblin-shadow-frost-{tag}.png"
        dk.shot(out)
        if dk.crashed():
            print(f"FAIL[{tag}] crash {dk.crashed()}")
            return None
        return out
    finally:
        dk.teardown()


def main():
    sh = boot("shadow", True)
    no = boot("noshadow", False)
    if not sh or not no:
        return 1

    s = Image.open(sh).convert("RGB")
    n = Image.open(no).convert("RGB")
    W, H = s.size

    # (1) Dock interior: a band across the dock centre, near the bottom. With the
    #     fix the shadow is excluded from the frost so this band is ~unchanged.
    box = (W // 2 - 160, H - 80, W // 2 + 160, H - 30)
    a = np.asarray(s.crop(box)).astype(int)
    b = np.asarray(n.crop(box)).astype(int)
    d = np.abs(a - b).sum(axis=2)
    dock_diff = float(d.mean())
    dock_frac = float((d > 15).mean())
    print(f"  dock-interior diff (shadow vs no-shadow): mean={dock_diff:.2f} frac>15={dock_frac:.3f}")

    # (2) The shadow must still exist somewhere: compare a region just ABOVE the
    #     dock (wallpaper) — the shadow halo darkens it, so shadow!=no-shadow here.
    halo = (W // 2 - 200, H - 150, W // 2 + 200, H - 95)
    ha = np.asarray(s.crop(halo).convert("L")).astype(float)
    hn = np.asarray(n.crop(halo).convert("L")).astype(float)
    halo_drop = float(hn.mean() - ha.mean())  # shadow makes it darker -> positive
    print(f"  wallpaper-above-dock luminance drop from shadow: {halo_drop:.2f}")

    ok = True
    # Frost must be largely free of the shadow. With the bug this was ~0.20+;
    # with the fix it drops well under 0.10 (residual is the shadow legitimately
    # on wallpaper beside the dock, not inside the frost).
    if dock_frac > 0.12:
        print("FAIL: the window shadow is smeared into the dock frost "
              f"(frac>15={dock_frac:.3f} too high)")
        ok = False
    # And the shadow itself must still be drawn.
    if halo_drop < 1.5:
        print("FAIL: the drop shadow no longer renders at all "
              f"(luminance drop {halo_drop:.2f} too small)")
        ok = False

    if ok:
        print("PASS: the drop shadow renders crisply on the wallpaper but is kept "
              "out of the frosted dock (not blurred into the frost)")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
