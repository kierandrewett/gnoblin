#!/usr/bin/env python3
# Regression: gnoblin-osd is a full-screen overlay layer, but its input region is
# intentionally empty. It must not block clicks to the topbar while visible.
import importlib.util
import os
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
REGION_CREATE = re.compile(r"create_region\([^\n]*wl_region[@#](\d+)\)")
REGION_ADD = re.compile(r"wl_region[@#](\d+)\.add\(")
SET_INPUT_REGION = re.compile(r"wl_surface[@#]\d+\.set_input_region\(wl_region[@#](\d+)\)")


def button_count(path):
    return len(BUTTON.findall(path.read_text(errors="replace")))


def wait_for_buttons(path, at_least, timeout=3.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        count = button_count(path)
        if count >= at_least:
            return count
        time.sleep(0.05)
    return button_count(path)


def empty_region_applied(path):
    text = path.read_text(errors="replace") if path.exists() else ""
    created = set(REGION_CREATE.findall(text))
    added = set(REGION_ADD.findall(text))
    for region_id in SET_INPUT_REGION.findall(text):
        if region_id in created and region_id not in added:
            return True
    return False


def wait_for_empty_region(path, proc, timeout=1.1):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if empty_region_applied(path):
            return True
        if proc.poll() is not None:
            return False
        time.sleep(0.03)
    return empty_region_applied(path)


def monitor_width():
    try:
        return int(os.environ.get("MONITOR", "1280x800").split("x", 1)[0])
    except Exception:
        return 1280


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    topbar_proc = None
    topbar_log = None
    osd_proc = None
    osd_log = None
    try:
        dk.boot(with_monitor=True)

        # Negotiate pointer injection before starting the short-lived OSD.
        dk.start_remote_desktop(pointer=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp

        topbar_env = dict(env)
        topbar_env["WAYLAND_DEBUG"] = "1"
        topbar_log_path = dk.tmp / "topbar-under-osd-wayland.log"
        topbar_log = open(topbar_log_path, "wb")
        topbar_proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-topbar")],
            env=topbar_env,
            stdout=topbar_log,
            stderr=subprocess.STDOUT,
        )

        deadline = time.time() + 8
        while time.time() < deadline:
            if topbar_proc.poll() is not None:
                print(f"FAIL: gnoblin-topbar exited before OSD check rc={topbar_proc.returncode}")
                return 1
            if dk.processes("gnoblin-topbar"):
                break
            time.sleep(0.1)
        time.sleep(1.0)
        if topbar_proc.poll() is not None:
            print(f"FAIL: gnoblin-topbar exited early rc={topbar_proc.returncode}")
            return 1

        osd_env = dict(env)
        osd_env["WAYLAND_DEBUG"] = "1"
        osd_env["GNOBLIN_OSD_LEVEL"] = "0.5"
        osd_log_path = dk.tmp / "osd-passthrough-wayland.log"
        osd_log = open(osd_log_path, "wb")
        osd_proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-osd"), "volume"],
            env=osd_env,
            stdout=osd_log,
            stderr=subprocess.STDOUT,
        )

        if not wait_for_empty_region(osd_log_path, osd_proc):
            print("FAIL: OSD did not commit an empty input region while alive")
            print(osd_log_path.read_text(errors="replace"))
            return 1
        if osd_proc.poll() is not None:
            print(f"FAIL: OSD exited before pass-through click rc={osd_proc.returncode}")
            return 1

        before_buttons = button_count(topbar_log_path)
        launcher_x = monitor_width() - 120
        dk.click(launcher_x, 17)
        after_buttons = wait_for_buttons(topbar_log_path, before_buttons + 2)
        osd_buttons = button_count(osd_log_path)

        print(
            f"  topbar buttons before={before_buttons} after={after_buttons}; "
            f"launcher_x={launcher_x}; "
            f"launcher={bool(dk.processes('gnoblin-launcher'))}; "
            f"osd buttons={osd_buttons}"
        )
        if after_buttons < before_buttons + 2:
            print("FAIL: topbar did not receive the click through the OSD overlay")
            return 1
        if not dk.processes("gnoblin-launcher"):
            print("FAIL: topbar launcher callback did not run through the OSD overlay")
            return 1
        if osd_buttons:
            print("FAIL: OSD received pointer buttons despite input_passthrough=true")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: full-screen OSD overlay passes pointer input through to the topbar")
        return 0
    finally:
        for proc in (osd_proc, topbar_proc):
            if proc and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=2)
                except Exception:
                    proc.kill()
        for log in (osd_log, topbar_log):
            if log:
                log.close()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
