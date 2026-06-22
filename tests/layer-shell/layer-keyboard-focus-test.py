#!/usr/bin/env python3
# Regression: a layer-shell surface using keyboard_interactivity=on_demand must
# become keyboard-focused through normal pointer interaction.
import importlib.util
import os
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def read_log(path):
    return path.read_text(errors="replace") if path.exists() else ""


def wait_for_text(path, needle, timeout=3.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        text = read_log(path)
        if needle in text:
            return text
        time.sleep(0.05)
    return read_log(path)


def main():
    client = os.environ.get("GNOBLIN_LAYER_KEYBOARD_FOCUS_CLIENT")
    if not client:
        print("FAIL: GNOBLIN_LAYER_KEYBOARD_FOCUS_CLIENT is not set")
        return 1
    mode = os.environ.get("GNOBLIN_LAYER_KEYBOARD_MODE", "on_demand")
    if mode not in ("on_demand", "exclusive"):
        print(f"FAIL: unsupported GNOBLIN_LAYER_KEYBOARD_MODE={mode}")
        return 1

    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    try:
        dk.boot(with_monitor=True)
        dk.start_remote_desktop(pointer=True)
        shift = dk.KEYS["shift"]
        dk._key(shift, True)
        dk._key(shift, False)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_LAYER_KEYBOARD_MODE"] = mode
        log_path = dk.tmp / "layer-keyboard-focus-client.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [client],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        text = wait_for_text(log_path, "MAPPED", timeout=4.0)
        if "MAPPED" not in text:
            print("FAIL: layer-keyboard-focus client did not map")
            print(text[-1000:])
            return 1

        if mode == "on_demand":
            dk.click(40, 40)
        text = wait_for_text(log_path, "KEYBOARD_ENTER", timeout=3.0)
        if "KEYBOARD_ENTER" not in text:
            if mode == "exclusive":
                print("FAIL: exclusive layer surface did not receive keyboard focus after map")
            else:
                print("FAIL: on_demand layer surface did not receive keyboard focus after click")
            print(text[-1000:])
            print(dk._tail(80))
            return 1

        dk.type_text("a")
        text = wait_for_text(log_path, "KEY 1", timeout=3.0)
        if "KEY 1" not in text:
            print(f"FAIL: focused {mode} layer surface did not receive key input")
            print(text[-1000:])
            print(dk._tail(80))
            return 1

        if proc.poll() is not None:
            print(f"FAIL: layer-keyboard-focus client exited early rc={proc.returncode}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print(f"PASS: {mode} layer-shell keyboard focus works")
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
