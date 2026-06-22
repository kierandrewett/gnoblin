#!/usr/bin/env python3
# Regression test for Slint animation pacing in the shared layer-shell runtime.
#
# A datetime popout starts with Slint `animate` blocks active. The layer-shell
# runtime must keep drawing after the first frame without needing pointer input.
import importlib.util
import os
import pathlib
import re
import shutil
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

FRAME_REQUEST = re.compile(r"wl_surface[@#]\d+\.frame\([^\n]*wl_callback[@#](\d+)\)")
FRAME_DONE = re.compile(r"wl_callback[@#](\d+)\.done\(")
SURFACE_FRAME_REQUEST = re.compile(
    r"wl_surface[@#](\d+)\.frame\([^\n]*wl_callback[@#](\d+)\)"
)
SURFACE_COMMIT = re.compile(r"wl_surface[@#](\d+)\.commit\(")


def commit_before_frame_done(text):
    pending_by_surface = {}
    outstanding_by_surface = {}
    callback_surface = {}
    for line in text.splitlines():
        req = SURFACE_FRAME_REQUEST.search(line)
        if req:
            surface, callback = req.groups()
            pending_by_surface.setdefault(surface, set()).add(callback)
            callback_surface[callback] = surface
            continue
        commit = SURFACE_COMMIT.search(line)
        if commit:
            surface = commit.group(1)
            outstanding = outstanding_by_surface.get(surface, set())
            if outstanding:
                return surface, sorted(outstanding, key=int)
            pending = pending_by_surface.pop(surface, set())
            if pending:
                outstanding_by_surface.setdefault(surface, set()).update(pending)
            continue
        done = FRAME_DONE.search(line)
        if done:
            callback = done.group(1)
            surface = callback_surface.get(callback)
            if surface:
                outstanding_by_surface.get(surface, set()).discard(callback)
    return None


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
        env["GNOBLIN_POPOUT"] = "datetime"
        log_path = dk.tmp / "topbar-animation-wayland.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-topbar")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        time.sleep(1.2)
        if proc.poll() is not None:
            print(f"FAIL: gnoblin-topbar exited early rc={proc.returncode}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1

        proc.terminate()
        try:
            proc.wait(timeout=2)
        except Exception:
            proc.kill()
        proc = None
        logf.close()
        logf = None
        shutil.copy(log_path, "/tmp/gnoblin-slint-animation-wayland.log")
        text = log_path.read_text(errors="replace")
        frame_requests = FRAME_REQUEST.findall(text)
        frame_dones = FRAME_DONE.findall(text)
        print(f"  frame callbacks requested={len(frame_requests)} done={len(frame_dones)}")
        if len(frame_requests) < 6 or len(frame_dones) < 5:
            print("FAIL: Slint animation did not advance without pointer input")
            return 1
        overlap = commit_before_frame_done(text)
        if overlap:
            surface, outstanding = overlap
            print(
                "FAIL: committed wl_surface#"
                f"{surface} while frame callbacks {outstanding[:4]} still outstanding"
            )
            return 1

        print("PASS: Slint animations advance on frame callbacks")
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
