#!/usr/bin/env python3
# Regression: if Night Light is already enabled, a newly hotplugged output must
# receive a gamma-control object. The daemon previously only synchronized gamma
# controls on toggle/temperature changes, so late outputs stayed neutral until
# the user toggled Night Light off and on again.
import importlib.util
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def gamma_control_requests(path):
    if not path.exists():
        return 0
    return path.read_text(errors="replace").count("get_gamma_control")


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    try:
        dk.boot(with_monitor=False, per_output=False)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["WAYLAND_DEBUG"] = "1"
        pathlib.Path(env["XDG_RUNTIME_DIR"], "gnoblin-nightlight").write_text("")

        log_path = dk.tmp / "night-light-wayland.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-night-light")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )
        time.sleep(1.5)
        if proc.poll() is not None:
            print(f"FAIL: gnoblin-night-light exited before hotplug rc={proc.returncode}")
            return 1
        before = gamma_control_requests(log_path)

        if not dk.add_monitor_late(1280, 800):
            print("SKIP: virtual monitor never materialized")
            return 0

        deadline = time.time() + 5.0
        after = before
        while time.time() < deadline:
            if proc.poll() is not None:
                print(f"FAIL: gnoblin-night-light exited after hotplug rc={proc.returncode}")
                return 1
            after = gamma_control_requests(log_path)
            if after > before:
                break
            time.sleep(0.2)

        print(f"  get_gamma_control before={before} after={after}")
        if after <= before:
            print("FAIL: Night Light did not bind gamma control for late output")
            return 1
        print("PASS: Night Light binds gamma control for late hotplugged outputs")
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
