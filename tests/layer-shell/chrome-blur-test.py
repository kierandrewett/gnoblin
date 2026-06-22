#!/usr/bin/env python3
# Regression test for the default compositor frost on gnoblin's own layer-shell
# chrome (topbar/dock/...) and the layer-shell namespace stash it depends on.
#
# Booting the full shell WITH clients, the topbar (`gnoblin-topbar`) gets blur +
# rounding by default. This only fires when `gnoblin_rules_layer_namespace()`
# resolves the real namespace (the mutter layer-shell patch), AND the new
# content-behind blur composites — both of which we verify by diffing the topbar
# band against a run with `[effects] gnoblin-chrome-blur = off`. With blur on, the
# wallpaper/window behind the (now translucent) topbar bleeds through frosted, so
# the topbar band differs from the un-frosted run.
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


def boot_shot(extra_conf, tag):
    dk = dh.Devkit()
    dk.extra_conf = extra_conf
    try:
        dk.boot(with_monitor=True)
        time.sleep(6)
        dk.spawn_and_wait("foot")
        time.sleep(2.5)
        out = f"/tmp/gnoblin-chrome-{tag}.png"
        dk.shot(out)
        if dk.crashed():
            print(f"FAIL[{tag}]: crash {dk.crashed()}")
            return None
        return out
    finally:
        dk.teardown()


def topbar_band(png):
    a = np.asarray(Image.open(png).convert("RGB")).astype(int)
    return a[2:30, :, :]  # the top ~28px (the bar)


def main():
    on = boot_shot("", "blur-on")
    off = boot_shot("[effects]\ngnoblin-chrome-blur = off\n", "blur-off")
    if not on or not off:
        return 1

    b_on, b_off = topbar_band(on), topbar_band(off)
    # Per-pixel difference across the topbar band: the frost changes many pixels.
    diff = int((np.abs(b_on - b_off).sum(axis=2) > 12).sum())
    total = b_on.shape[0] * b_on.shape[1]
    print(f"  topbar band changed px = {diff} / {total}")
    if diff < total * 0.02:
        print("FAIL: chrome blur default did not visibly change the topbar "
              "(namespace inactive, or frost not compositing)")
        return 1
    print("PASS: default chrome frost changes the topbar (layer namespace active "
          "+ content-behind blur composites); overridable via gnoblin-chrome-blur")
    return 0


if __name__ == "__main__":
    sys.exit(main())
