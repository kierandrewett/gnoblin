#!/usr/bin/env python3
# Visual verification for the rules-based effects system, software-rendered
# (llvmpipe) — fine for corner geometry and border presence:
#   1. circular vs squircle corner shape differ (whole-frame pixel diff)
#   2. a coloured LINE border renders (count the border-colour pixels)
#   3. the macOS LIP border renders (edge band differs from no-lip)
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image, ImageChops
    import numpy as np
except Exception:
    print("SKIP: no PIL/numpy")
    sys.exit(0)


def boot_capture(extra_conf, tag):
    dk = dh.Devkit()
    try:
        dk.extra_conf = extra_conf
        dk.boot(with_monitor=True)
        time.sleep(4)
        if not dk.spawn_and_wait("foot"):
            print(f"FAIL[{tag}]: foot did not map")
            return None
        time.sleep(2)
        out = f"/tmp/gnoblin-fxrules-{tag}.png"
        dk.shot(out)
        if dk.crashed():
            print(f"FAIL[{tag}]: crash {dk.crashed()}")
            return None
        return out
    finally:
        dk.teardown()


def main():
    # 1. squircle vs circular --------------------------------------------------
    circ = boot_capture(
        "[appearance]\nrounding = 30\n\n[effects]\ncorner-style = circular\n", "circular")
    sq = boot_capture(
        "[appearance]\nrounding = 30\n\n[effects]\ncorner-style = squircle\ncorner-smoothing = 1.0\n",
        "squircle")
    if not circ or not sq:
        return 1
    d = ImageChops.difference(Image.open(circ).convert("RGB"), Image.open(sq).convert("RGB"))
    diff = int((np.asarray(d).sum(axis=2) > 0).sum())
    print(f"  squircle vs circular differing px = {diff}")
    if diff < 40:
        print("FAIL: squircle corner does not differ from circular")
        return 1
    print("PASS: squircle corner shape differs from circular")

    # 2. coloured line border --------------------------------------------------
    border = boot_capture(
        "[window-rules]\n"
        "rule = class=foot | rounding 18, border 6 \"#ff0000ff\", border-style line\n", "line")
    if not border:
        return 1
    a = np.asarray(Image.open(border).convert("RGB")).astype(int)
    red = int(((a[:, :, 0] > 120) & (a[:, :, 1] < 90) & (a[:, :, 2] < 90)).sum())
    print(f"  red line-border px = {red}")
    if red < 500:
        print("FAIL: line border (red) not rendered")
        return 1
    print("PASS: coloured line border renders")

    # 3. lip border vs none ----------------------------------------------------
    nolip = boot_capture("[appearance]\nrounding = 24\n", "nolip")
    lip = boot_capture(
        "[appearance]\nrounding = 24\n\n[effects]\nborder-width = 4\nborder-style = lip\n"
        "border-color = \"#000000bb\"\n", "lip")
    if not nolip or not lip:
        return 1
    dl = ImageChops.difference(Image.open(nolip).convert("RGB"), Image.open(lip).convert("RGB"))
    changed = int((np.asarray(dl).sum(axis=2) > 14).sum())
    print(f"  lip vs no-lip changed px = {changed}")
    if changed < 50:
        print("FAIL: lip border barely changes the image (not rendering)")
        return 1
    print("PASS: lip border renders (differs from no-lip)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
