#!/usr/bin/env python3
# Regression: a running gnoblin-topbar must notice live `[animations] enabled`
# edits and update Slint's Theme.motion-scale.
#
# The failure mode is visible without a private test hook: start with animations
# enabled, edit config to disable them, then open a popout. If the running topbar
# kept the startup motion scale, opening the popout schedules a long train of
# Wayland frame callbacks; with motion scale 0 it settles in only a few frames.
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

FRAME_REQUEST = re.compile(r"wl_surface[@#]\d+\.frame\([^\n]*wl_callback[@#](\d+)\)")


def write_config(path, animations_enabled):
    path.write_text(
        "[appearance]\n"
        'background = "#202434"\n'
        "[startup]\n"
        "[roles]\n"
        "window-menu = gnoblin-window-menu\n"
        "[bind]\n"
        "[animations]\n"
        f"enabled = {'true' if animations_enabled else 'false'}\n"
    )


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
        dk.extra_conf = "[animations]\nenabled = true\n"
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["WAYLAND_DEBUG"] = "1"
        log_path = dk.tmp / "topbar-live-motion-wayland.log"
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

        time.sleep(1.0)
        conf = dk.tmp / "config" / "gnoblin" / "gnoblin.conf"
        write_config(conf, False)
        time.sleep(1.0)

        logf.flush()
        os.fsync(logf.fileno())
        baseline = log_path.stat().st_size

        dk.click(640, 17)
        time.sleep(0.8)

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

        segment = log_path.read_bytes()[baseline:].decode(errors="replace")
        frame_requests = FRAME_REQUEST.findall(segment)
        print(f"  frame callbacks after disabling animations: {len(frame_requests)}")
        if len(frame_requests) > 20:
            print("FAIL: topbar kept animating after [animations] enabled=false")
            return 1

        print("PASS: running topbar applies live motion-scale config changes")
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
