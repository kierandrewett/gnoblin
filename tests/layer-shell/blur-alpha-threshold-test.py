#!/usr/bin/env python3
# Regression for the configurable blur alpha-threshold: the frost is applied only
# where the surface's OWN alpha is below `blur-alpha-threshold`. A translucent
# foot buffer (colors.alpha=0.55) over a busy maximized foot:
#   - threshold 1.0 (default)  -> frost the translucent body (backdrop frosted)
#   - threshold 0.0            -> gate the frost OFF everywhere (alpha>=0 always)
# So the body region must DIFFER between the two thresholds, proving the knob is
# wired through rules -> the blur effect -> the shader uniform.
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


def boot(tag, thr):
    dk = dh.Devkit()
    dk.extra_conf = (
        "[effects]\ngnoblin-chrome-blur = off\n\n"
        "[window-rules]\n"
        f"rule = class=foot | blur 44, rounding 0, blur-alpha-threshold {thr}\n"
    )
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        dk.spawn_and_wait("foot")            # busy backdrop
        time.sleep(0.8)
        dk.dispatch("maximize", "")
        time.sleep(1.2)
        # translucent BUFFER (alpha 0.55) upper window -> its own alpha < 1
        dk.spawn_and_wait("foot -o colors.alpha=0.55 -o background=101010")
        time.sleep(2.5)
        out = f"/tmp/gnoblin-blurthr-{tag}.png"
        dk.shot(out)
        if dk.crashed():
            print(f"FAIL[{tag}] crash {dk.crashed()}")
            return None
        frames = dk.list_window_frames()
        top = min(frames, key=lambda f: f[-2] * f[-1])
        return out, top
    finally:
        dk.teardown()


def body(png, fr):
    x, y, w, h = fr[-4:]
    a = np.asarray(Image.open(png).convert("RGB")).astype(int)
    return a[y + 40:y + h - 10, x + 10:x + w - 10]


def main():
    hi = boot("frost", 1.0)    # frost the translucent body
    lo = boot("gateoff", 0.0)  # frost gated off everywhere
    if not hi or not lo:
        return 1

    bh = body(*hi)
    bl = body(*lo)
    n0 = min(bh.shape[0], bl.shape[0])
    n1 = min(bh.shape[1], bl.shape[1])
    d = np.abs(bh[:n0, :n1] - bl[:n0, :n1]).sum(axis=2)
    changed = float((d > 6).mean())
    print(f"  translucent-body region changed (threshold 1.0 vs 0.0): frac={changed:.3f}")

    if changed < 0.02:
        print("FAIL: blur-alpha-threshold had no effect — the frost is identical "
              "with the gate open vs closed (uniform not wired?)")
        return 1
    print("PASS: blur-alpha-threshold gates the frost by the surface's own alpha "
          "(wired rules -> blur effect -> shader)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
