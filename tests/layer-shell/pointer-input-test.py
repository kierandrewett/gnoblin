#!/usr/bin/env python3
# Regression: layer-shell surfaces must receive pointer enter/motion/button
# events and route clicks into Slint callbacks.
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
ENTER = re.compile(r"wl_pointer[@#]\d+\.enter\(")


def monitor_width():
    try:
        return int(os.environ.get("MONITOR", "1280x800").split("x", 1)[0])
    except Exception:
        return 1280


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    proc = None
    logf = None
    dk = dh.Devkit()
    try:
        dk.extra_conf = "[topbar]\nleft = launcher\ncenter =\nright =\n"
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["WAYLAND_DEBUG"] = "1"
        log_path = dk.tmp / "topbar-pointer-wayland.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-topbar")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )
        time.sleep(2.0)
        if proc.poll() is not None:
            print(f"FAIL: gnoblin-topbar exited early rc={proc.returncode}")
            return 1

        # Test layout starts with the launcher/search widget in the left zone.
        # Click it so we can observe both wl_pointer traffic and the resulting
        # Slint callback (it spawns gnoblin-launcher).
        dk.click(20, 17)
        time.sleep(1.0)

        if proc.poll() is not None:
            print(f"FAIL: gnoblin-topbar exited after click rc={proc.returncode}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        proc.terminate()
        try:
            proc.wait(timeout=2)
        except Exception:
            proc.kill()
        proc = None
        logf.close()
        logf = None
        text = log_path.read_text(errors="replace")
        enters = len(ENTER.findall(text))
        buttons = len(BUTTON.findall(text))
        print(f"  wl_pointer enter={enters} button={buttons}")
        if enters < 1 or buttons < 2:
            print("FAIL: topbar did not receive pointer enter/button events")
            return 1
        if not dk.processes("gnoblin-launcher"):
            print("FAIL: topbar launcher click did not spawn gnoblin-launcher")
            return 1

        print("PASS: layer-shell pointer input reaches Slint callbacks")
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
