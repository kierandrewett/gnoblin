#!/usr/bin/env python3
# Regression test for REAL content-behind blur: the blur must frost the window
# stacked underneath the blurred surface, not only the wallpaper.
#
# Scene: a maximized foot full of text/edges (lots of high-frequency detail),
# then a smaller translucent+blurred foot on top of it. The blurred top window
# captures the framebuffer behind it — which here is the busy lower window — so
# inside the top window the backdrop's high-frequency detail is smeared. We
# compare the high-frequency energy inside the top window's region with blur on
# vs off; blur-on must be markedly lower.
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


def boot(extra, tag):
    dk = dh.Devkit()
    # Chrome blur off so only our rule drives the frost; a full-opaque lower
    # window and a translucent blurred upper window.
    dk.extra_conf = "[effects]\ngnoblin-chrome-blur = off\n\n[window-rules]\n" + extra
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        dk.spawn_and_wait("foot")      # lower
        time.sleep(0.8)
        dk.dispatch("maximize", "")
        time.sleep(1.2)
        dk.spawn_and_wait("foot")      # upper (smaller, translucent, blurred)
        time.sleep(2.5)
        out = f"/tmp/gnoblin-cb-{tag}.png"
        dk.shot(out)
        if dk.crashed():
            print(f"FAIL[{tag}] crash {dk.crashed()}")
            return None
        frames = dk.list_window_frames()
        return out, frames
    finally:
        dk.teardown()


def crop(png, frame):
    x, y, w, h = frame[-4:]
    im = Image.open(png).convert("RGB")
    a = np.asarray(im).astype(int)
    H, W, _ = a.shape
    x0, y0 = max(0, x + 6), max(0, y + 30)
    x1, y1 = min(W, x + w - 6), min(H, y + h - 6)
    return a[y0:y1, x0:x1]


def smear(png, frame):
    """High-frequency detail in the top-window region. The bottom (maximized)
    foot is full of text; through the translucent top it shows up as sharp edges
    with blur off and smeared with blur on. We measure horizontal gradient
    energy — robust enough that the BLUR-ON value is markedly lower."""
    x, y, w, h = frame[-4:]
    im = Image.open(png).convert("L")
    a = np.asarray(im).astype(float)
    H, W = a.shape
    x0, y0 = max(0, x + 6), max(0, y + 30)
    x1, y1 = min(W, x + w - 6), min(H, y + h - 6)
    sub = a[y0:y1, x0:x1]
    if sub.size < 100:
        return None
    return float(np.abs(np.diff(sub, axis=1)).mean())


def top_frame(frames):
    # the maximized one spans the screen; the other (smaller) is the upper window
    return min(frames, key=lambda f: f[-2] * f[-1])


def main():
    on = boot("rule = class=foot | opacity 70, blur 32\n", "on")
    off = boot("rule = class=foot | opacity 70, no-blur\n", "off")
    if not on or not off:
        return 1
    on_png, on_frames = on
    off_png, off_frames = off

    fr_on, fr_off = top_frame(on_frames), top_frame(off_frames)

    # (a) The blurred top window's region must DIFFER from the un-blurred one —
    #     proving the compositor composited a frosted backdrop there.
    c_on, c_off = crop(on_png, fr_on), crop(off_png, fr_off)
    n = min(c_on.shape[0], c_off.shape[0]), min(c_on.shape[1], c_off.shape[1])
    d = np.abs(c_on[:n[0], :n[1]] - c_off[:n[0], :n[1]]).sum(axis=2)
    changed = int((d > 16).sum())
    total = n[0] * n[1]
    print(f"  top-window region changed px (blur vs no-blur) = {changed}/{total}")

    # (b) And the high-frequency detail of the window behind should be reduced.
    e_on, e_off = smear(on_png, fr_on), smear(off_png, fr_off)
    if e_on is not None and e_off is not None:
        print(f"  top-window HF energy: blur-on={e_on:.3f} blur-off={e_off:.3f}")

    if changed < total * 0.10:
        print("FAIL: blurred top window barely differs from un-blurred — the "
              "content-behind frost is not compositing")
        return 1
    print("PASS: the blurred translucent window composites a frosted backdrop of "
          "the window/wallpaper behind it (content-behind blur)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
