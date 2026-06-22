#!/usr/bin/env python3
# Regression test for per-output wallpaper binding.
#
# Boots two virtual outputs with no other clients, then starts two wallpaper
# clients manually with different GNOBLIN_CONFIG files and explicit
# `--output Meta-0` / `--output Meta-1`. Each monitor must show only its own
# wallpaper colour; if --output resolution falls back to "compositor picks", this
# tends to paint the wrong monitor or stack both wallpapers on one output.
import importlib.util, pathlib, subprocess, sys, time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

LEFT = (218, 32, 44)
RIGHT = (32, 92, 226)
SHOT = "/tmp/gnoblin-wallpaper-output-test.png"


def close_enough(actual, expected):
    return all(abs(a - b) <= 8 for a, b in zip(actual, expected))


def write_wallpaper(root, name, color):
    img = root / f"{name}.png"
    cfg = root / f"{name}.conf"
    Image.new("RGB", (64, 64), color).save(img)
    cfg.write_text(
        "[appearance]\n"
        f"wallpaper = {img}\n"
        "wallpaper-style = stretched\n"
        'background = "#000000"\n'
    )
    return cfg


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    procs = []
    logs = []
    try:
        dk.boot(monitors=["1280x800", "1280x800"])
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp

        left_cfg = write_wallpaper(dk.tmp, "left-wallpaper", LEFT)
        right_cfg = write_wallpaper(dk.tmp, "right-wallpaper", RIGHT)
        wallpaper = str(dh.PREFIX / "bin" / "gnoblin-wallpaper")

        for output, cfg in (("Meta-0", left_cfg), ("Meta-1", right_cfg)):
            proc_env = dict(env, GNOBLIN_CONFIG=str(cfg))
            log = open(dk.tmp / f"{output}-wallpaper.log", "wb")
            logs.append(log)
            procs.append(
                subprocess.Popen(
                    [wallpaper, "--output", output],
                    env=proc_env,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                )
            )

        time.sleep(2.5)
        for proc in procs:
            if proc.poll() is not None:
                print(f"FAIL: wallpaper client exited early rc={proc.returncode}")
                return 1

        dk.shot(SHOT)
        px = Image.open(SHOT).convert("RGB").load()
        left_px = px[640, 400]
        right_px = px[1920, 400]
        print(f"  left monitor pixel={left_px} expected={LEFT}")
        print(f"  right monitor pixel={right_px} expected={RIGHT}")

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if not close_enough(left_px, LEFT):
            print("FAIL: Meta-0 did not show its wallpaper")
            return 1
        if not close_enough(right_px, RIGHT):
            print("FAIL: Meta-1 did not show its wallpaper")
            return 1

        print("PASS: wallpaper --output binds independently on two monitors")
        return 0
    finally:
        for proc in procs:
            if proc.poll() is None:
                proc.terminate()
        for proc in procs:
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        for log in logs:
            log.close()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
