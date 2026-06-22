#!/usr/bin/env python3
# Regression test for the fullscreen-doesn't-cover-panels bug.
#
# History: layer-shell surfaces pin their MetaStackLayer and calculate_layer
# returned it verbatim, bypassing meta_window_get_default_layer's in_fullscreen
# handling. So a wlr `top` panel (the topbar/dock, mapped to META_LAYER_DOCK)
# stayed ABOVE a fullscreen window and obscured it. Fix (patch
# 30-layer-shell/0004): drop a pinned DOCK layer to BOTTOM while its monitor is
# in fullscreen, like docks already do.
#
# This test spawns a real window, makes it fullscreen, and asserts (via pixels)
# that the topbar band at the top of the screen is COVERED by the window — i.e.
# the top strip now matches the window body, not the panel chrome. Contrast with
# maximize-strut-test, which asserts maximize does NOT cover the topbar.
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

SHOT = "/tmp/gnoblin-fullscreen-cover-test.png"


def close(a, b, tol=14):
    return all(abs(x - y) <= tol for x, y in zip(a, b))


def main():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        dk.dispatch("fullscreen")
        time.sleep(1.5)
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        dk.shot(SHOT)
        im = Image.open(SHOT).convert("RGB")
        px = im.load()
        # Sample the top strip (where the topbar lives, y<34) and the window body
        # well below it. If the window covers the panel they match; if the panel
        # is still on top they differ (panel chrome != terminal bg). Sample at
        # x=1200 (topbar status-icon cluster) to be a strong signal.
        body = px[1200, 200]
        band = [px[1200, 5], px[1000, 5], px[640, 25]]
        covered = all(close(p, body) for p in band)
        print(f"  window body={body} top-band={band} covered={covered}")
        if not covered:
            print("FAIL: fullscreen window does not cover the topbar (panel still on top)")
            return 1
        print("PASS: fullscreen window covers the topbar/dock panels")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
