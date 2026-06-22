#!/usr/bin/env python3
# Regression: live `[topbar]` layout edits must move both visual widgets and
# their input/callback geometry. This guards the Firefox-style customisable
# topbar model where widgets can move between left/center/right zones.
import importlib.util
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def write_config(path, marker, *, status_side):
    if status_side == "left":
        left = "status"
        right = ""
    elif status_side == "right":
        left = ""
        right = "status"
    else:
        raise ValueError(status_side)

    path.write_text(
        "[appearance]\n"
        'background = "#202434"\n'
        "[startup]\n"
        "[roles]\n"
        "window-menu = gnoblin-window-menu\n"
        "[bind]\n"
        "[topbar] # trailing comments after section headers must match the compositor parser\n"
        f"left = {left}\n"
        "center = \n"
        f"right = {right}\n"
        f"control_centre = printf {status_side} > {marker}\n"
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


def wait_for_path(path, timeout=1.5):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            return True
        time.sleep(0.05)
    return path.exists()


def click_cc_settings(dk, popout_x):
    # ControlCentrePopout y = 34px topbar + 8px gap = 42.
    # Header starts at 16px inset; settings is the second 32px round button.
    dk.click(popout_x + 72, 74)


def fail_with_logs(message, dk, log_path):
    print(f"FAIL: {message}")
    if dk.crashed():
        print(f"  compositor: {dk.crashed()}")
        print(dk._tail())
    if log_path.exists():
        tail = "\n".join(log_path.read_text(errors="replace").splitlines()[-30:])
        print(f"  topbar log tail:\n{tail}")
    return 1


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    try:
        left_marker = dk.tmp / "topbar-layout-left"
        right_marker = dk.tmp / "topbar-layout-right"
        dk.extra_conf = (
            "[topbar] # trailing comments after section headers must match the compositor parser\n"
            "left = status\n"
            "center = \n"
            "right = \n"
            f"control_centre = printf left > {left_marker}\n"
        )
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        log_path = dk.tmp / "topbar-layout-live.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-topbar")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )
        if not wait_for_process(dk, "gnoblin-topbar", proc):
            return fail_with_logs(
                f"gnoblin-topbar did not stay running rc={proc.returncode}",
                dk,
                log_path,
            )

        time.sleep(0.8)

        # Left layout: status occupies x~=12..92, so x=50 must open CC.
        dk.click(50, 17)
        click_cc_settings(dk, 8)
        if not wait_for_path(left_marker):
            return fail_with_logs("left-side status widget did not activate settings", dk, log_path)
        if right_marker.exists():
            return fail_with_logs("right marker appeared before layout moved", dk, log_path)

        # Dismiss the open popout before testing stale/old hit areas.
        dk.click(640, 700)
        time.sleep(0.35)

        conf = dk.tmp / "config" / "gnoblin" / "gnoblin.conf"
        write_config(conf, right_marker, status_side="right")
        time.sleep(1.2)

        # Old left coordinate should be inert after the live layout rewrite.
        dk.click(50, 17)
        click_cc_settings(dk, 8)
        time.sleep(0.5)
        if right_marker.exists():
            return fail_with_logs(
                "old left status hit target still activated after moving widget right",
                dk,
                log_path,
            )

        # Right layout: status occupies x~=1188..1268, so x=1228 must open CC.
        dk.click(1228, 17)
        # With anchor near the right edge, 360px CC clamps to x=908 on 1280px.
        click_cc_settings(dk, 908)
        if not wait_for_path(right_marker):
            return fail_with_logs("right-side status widget did not activate after live layout edit", dk, log_path)

        if dk.crashed():
            return fail_with_logs("compositor crashed", dk, log_path)
        print("PASS: running topbar live-reloaded layout and moved status input/callback geometry")
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
