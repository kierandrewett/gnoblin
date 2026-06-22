#!/usr/bin/env python3
# Regression: a running gnoblin-topbar must notice live `[topbar]` command edits.
#
# The compositor watches gnoblin.conf and the shipped example says config edits
# are live. The topbar is a separate long-running layer-shell client, so its
# launcher/control-centre action callbacks must not keep startup commands forever.
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


def write_config(path, *, settings_marker, launcher_marker, account_marker, wired_marker):
    path.write_text(
        "[appearance]\n"
        'background = "#202434"\n'
        "[startup]\n"
        "[roles]\n"
        "window-menu = gnoblin-window-menu\n"
        "[bind]\n"
        "[topbar]\n"
        f"launcher = printf launcher > {launcher_marker}\n"
        f"account = printf account > {account_marker}\n"
        f"wired = printf wired > {wired_marker}\n"
        f"control_centre = printf 'topbar # refreshed' > {settings_marker} # trailing comment\n"
    )


def monitor_width():
    try:
        return int(os.environ.get("MONITOR", "1280x800").split("x", 1)[0])
    except Exception:
        return 1280


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
        old_marker = dk.tmp / "topbar-old-command"
        settings_marker = dk.tmp / "topbar-settings-command"
        launcher_marker = dk.tmp / "topbar-launcher-command"
        account_marker = dk.tmp / "topbar-account-command"
        wired_marker = dk.tmp / "topbar-wired-command"
        dk.extra_conf = (
            "[topbar]\n"
            f"launcher = printf old > {old_marker}\n"
            f"control_centre = printf old > {old_marker}\n"
        )
        dk.boot(with_monitor=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_POPOUT"] = "cc"
        log_path = dk.tmp / "topbar-live-commands.log"
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
        write_config(
            conf,
            settings_marker=settings_marker,
            launcher_marker=launcher_marker,
            account_marker=account_marker,
            wired_marker=wired_marker,
        )
        time.sleep(1.0)

        # Use the real default layout. The right zone is:
        # launcher, tray, status. With no tray items on a 1280px test monitor,
        # the launcher button sits around x=1160; scale that from the monitor
        # width so this still works under MONITOR=WxH.
        w = monitor_width()
        dk.click(w - 120, 17)
        time.sleep(0.4)

        # ControlCentrePopout is 360px wide at x=904 on the default 1280px
        # monitor. Header has 16px padding; the settings button is the second
        # 32px round button in the left cluster.
        dk.click(976, 74)
        # Account is the first round button in that same header.
        dk.click(936, 74)
        # Wired is the first primary connectivity tile.
        dk.click(960, 130)
        time.sleep(0.8)

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1
        missing = [
            name
            for name, marker in (
                ("launcher", launcher_marker),
                ("settings", settings_marker),
                ("account", account_marker),
                ("wired", wired_marker),
            )
            if not marker.exists()
        ]
        if not missing and not old_marker.exists():
            print("PASS: running topbar reloaded default-layout launcher and quick-settings commands")
            return 0
        if old_marker.exists():
            print("FAIL: topbar ran stale command after config changed")
            return 1
        if log_path.exists():
            tail = "\n".join(log_path.read_text(errors="replace").splitlines()[-20:])
            print(f"  topbar log tail:\n{tail}")
        print(f"FAIL: topbar command clicks missing markers: {', '.join(missing)}")
        return 1
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
