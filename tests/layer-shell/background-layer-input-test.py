#!/usr/bin/env python3
# Regression: the wallpaper background layer must be visually behind apps and
# must not intercept pointer input from a normal window above it.
import importlib.util
import pathlib
import re
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

BUTTON = re.compile(r"wl_pointer[@#]\d+\.button\(")


def button_count(path):
    return len(BUTTON.findall(path.read_text(errors="replace")))


def wait_for_buttons(path, at_least, timeout=4.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        count = button_count(path)
        if count >= at_least:
            return count
        time.sleep(0.05)
    return button_count(path)


def foot_visible(dk):
    return any(w[2] == "foot" and not w[4] for w in dk.list_windows())


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    wallpaper_proc = None
    wallpaper_log = None
    foot_proc = None
    foot_log = None
    try:
        dk.extra_appearance = 'background = "#101820"\n'
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        wallpaper_log = open(dk.tmp / "background-wallpaper.log", "wb")
        wallpaper_proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-wallpaper")],
            env=env,
            stdout=wallpaper_log,
            stderr=subprocess.STDOUT,
        )
        time.sleep(1.5)
        if wallpaper_proc.poll() is not None:
            print(f"FAIL: gnoblin-wallpaper exited early rc={wallpaper_proc.returncode}")
            return 1

        foot_env = dict(env)
        foot_env["WAYLAND_DEBUG"] = "1"
        foot_log_path = dk.tmp / "foot-wayland.log"
        foot_log = open(foot_log_path, "wb")
        foot_proc = subprocess.Popen(
            ["foot"],
            env=foot_env,
            stdout=foot_log,
            stderr=subprocess.STDOUT,
        )

        deadline = time.time() + 10
        while time.time() < deadline and not foot_visible(dk):
            if foot_proc.poll() is not None:
                print(f"FAIL: foot exited before mapping rc={foot_proc.returncode}")
                return 1
            time.sleep(0.25)
        if not foot_visible(dk):
            print(f"FAIL: foot did not map above wallpaper: windows={dk.list_windows()}")
            return 1

        dk.dispatch("maximize")
        time.sleep(0.5)
        before = button_count(foot_log_path)
        dk.click(640, 400)
        after = wait_for_buttons(foot_log_path, before + 2)
        print(f"  foot wl_pointer buttons before={before} after={after}")
        if after < before + 2:
            print("FAIL: click on app above wallpaper did not reach the app")
            return 1

        leaked = [
            w for w in dk.list_windows()
            if "wallpaper" in (w[1] or "").lower() or "wallpaper" in (w[2] or "").lower()
        ]
        if leaked:
            print(f"FAIL: wallpaper leaked into normal window list: {leaked}")
            return 1

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: background layer stays below apps and does not steal pointer input")
        return 0
    finally:
        for proc in (foot_proc, wallpaper_proc):
            if proc and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=2)
                except Exception:
                    proc.kill()
        for log in (foot_log, wallpaper_log):
            if log:
                log.close()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
