#!/usr/bin/env python3
# Regression: ext-foreign-toplevel-list-v1 actually streams the window list.
#
# The compositor advertising the global isn't enough — a taskbar/pager needs the
# `toplevel` + `app_id` events to flow. This boots gnoblin-shell, opens a known
# app (foot), runs the foreign-toplevel probe, and asserts the probe sees `foot`.
# The probe binary path comes from FT_PROBE (built by run-foreign-toplevel.sh).
import os, sys, time, importlib.util, pathlib, subprocess

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

PROBE = os.environ.get("FT_PROBE", "/tmp/gnoblin-ft-probe")


def main():
    if not pathlib.Path(PROBE).exists():
        print(f"SKIP: no foreign-toplevel probe at {PROBE}")
        return 0
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if not dk.spawn_and_wait("foot"):
            print("FAIL: foot did not map")
            return 1
        time.sleep(1)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        r = subprocess.run([PROBE], env=env, capture_output=True, text=True, timeout=15)
        out = r.stdout.strip()
        print("  probe output:", out.replace("\n", " | ") or "(none)")
        if r.returncode != 0:
            print(f"FAIL: probe failed (rc={r.returncode}): {r.stderr.strip()}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        if "app_id=foot" not in out:
            print("FAIL: foreign-toplevel list did not report the foot window")
            return 1
        print("PASS: ext-foreign-toplevel-list streams the window list (saw foot)")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
