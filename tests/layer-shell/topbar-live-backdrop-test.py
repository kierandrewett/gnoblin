#!/usr/bin/env python3
# Regression: a running gnoblin-topbar must reload its glass backdrop when the
# configured wallpaper changes.
#
# The wallpaper client live-reloads the desktop background, but topbar/dock
# popouts also keep their own pre-blurred wallpaper image for glass surfaces.
# Those layer clients need to refresh that image too, otherwise the desktop and
# popout glass visibly disagree after a config edit.
import importlib.util
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

RED = (240, 20, 30)
CYAN = (20, 220, 230)
SAMPLE = (1080, 100)


def write_config(path, wall):
    path.write_text(
        "[appearance]\n"
        'background = "#202434"\n'
        f"wallpaper = {wall}\n"
        "wallpaper-style = stretched\n"
        "[startup]\n"
        "[roles]\n"
        "window-menu = gnoblin-window-menu\n"
        "[bind]\n"
        "[animations]\n"
        "enabled = false\n"
    )


def is_red_tinted(pixel):
    r, g, b = pixel
    return r > b + 12 and r > g + 12


def is_cyan_tinted(pixel):
    r, g, b = pixel
    return g > r + 12 and b > r + 12


def wait_for_process(dk, needle, proc, timeout=8):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if dk.processes(needle):
            return True
        if proc.poll() is not None:
            return False
        time.sleep(0.25)
    return False


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    try:
        red = dk.tmp / "topbar-red.png"
        cyan = dk.tmp / "topbar-cyan.png"
        Image.new("RGB", (64, 64), RED).save(red)
        Image.new("RGB", (64, 64), CYAN).save(cyan)
        dk.extra_conf = (
            "[appearance]\n"
            f"wallpaper = {red}\n"
            "wallpaper-style = stretched\n"
            "[animations]\n"
            "enabled = false\n"
        )
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_POPOUT"] = "cc"
        log_path = dk.tmp / "topbar-live-backdrop.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-topbar")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )
        if not wait_for_process(dk, "gnoblin-topbar", proc):
            print(f"FAIL: gnoblin-topbar did not stay running rc={proc.returncode}")
            return 1

        time.sleep(1.2)
        before_shot = dk.tmp / "topbar-backdrop-before.png"
        dk.shot(before_shot)
        before = Image.open(before_shot).convert("RGB").getpixel(SAMPLE)
        print(f"  before wallpaper edit sample={before}")
        if not is_red_tinted(before):
            print("FAIL: initial popout backdrop did not sample the red wallpaper")
            return 1

        conf = dk.tmp / "config" / "gnoblin" / "gnoblin.conf"
        write_config(conf, cyan)

        after = before
        deadline = time.time() + 6
        while time.time() < deadline:
            time.sleep(0.5)
            after_shot = dk.tmp / "topbar-backdrop-after.png"
            dk.shot(after_shot)
            after = Image.open(after_shot).convert("RGB").getpixel(SAMPLE)
            if is_cyan_tinted(after):
                break

        print(f"  after wallpaper edit sample={after}")
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1
        if not is_cyan_tinted(after):
            print("FAIL: topbar popout kept the stale wallpaper backdrop")
            return 1

        print("PASS: running topbar reloads popout backdrop after wallpaper config edit")
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        if logf:
            logf.close()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
