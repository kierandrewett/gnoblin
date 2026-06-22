#!/usr/bin/env python3
# Regression: notification history now lives under quick settings. The explicit
# topbar notification widget and the old calendar notification-card area must
# not recreate the removed right-side notification panel or steal input from
# Slint popouts after the shell sits idle.
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


def monitor_width():
    try:
        return int(os.environ.get("MONITOR", "1280x800").split("x", 1)[0])
    except Exception:
        return 1280


def wait_clients(dk):
    missing = []
    for needle in ("gnoblin-topbar", "gnoblin-dock", "gnoblin-notifyd", "gnoblin-wallpaper"):
        if not dk.processes(needle):
            missing.append(needle)
    return missing


def main():
    runtime = pathlib.Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    center = runtime / "gnoblin-notif-center"
    dnd = runtime / "gnoblin-dnd"
    center.unlink(missing_ok=True)
    dnd.unlink(missing_ok=True)

    dk = dh.Devkit()
    try:
        dk.extra_conf = (
            "\n[topbar]\n"
            "left = notifications\n"
            "center = clock\n"
            "right = status\n"
        )
        dk.boot(with_monitor=True)
        time.sleep(7)
        missing = wait_clients(dk)
        if missing:
            print(f"FAIL: missing autostart clients: {', '.join(missing)}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed before input: {dk.crashed()}")
            print(dk._tail())
            return 1

        w = monitor_width()
        notification_widget = (18, 17)
        clock = (w // 2, 17)
        old_calendar_notification_area = (w // 2, 475)
        quick_settings = (w - 48, 17)
        dnd_tile = (w - 280, 400)

        subprocess.run(
            ["notify-send", "Input probe", "history belongs under quick settings"],
            env=dk._env(),
            check=True,
        )
        time.sleep(0.8)

        dk.click(*notification_widget)
        time.sleep(0.8)
        if center.exists():
            print("FAIL: notification widget opened the removed notification center flag")
            return 1

        before = dnd.exists()
        dk.click(*dnd_tile)
        time.sleep(0.8)
        after = dnd.exists()
        if after == before:
            print("FAIL: notification widget did not open an interactive quick-settings popout")
            return 1

        dk.click(*clock)
        time.sleep(0.5)
        dk.click(*old_calendar_notification_area)
        time.sleep(0.8)
        if center.exists():
            print("FAIL: calendar still exposes the removed notification center")
            return 1

        # The reported failure mode was input dying only after the shell had sat
        # still. Leave the pointer and clients idle, then verify the status
        # cluster can still open quick settings and receive a tile click without
        # a motion event waking it up first.
        time.sleep(6.0)

        dk.click(*quick_settings)
        time.sleep(0.8)
        before = dnd.exists()
        dk.click(*dnd_tile)
        time.sleep(0.8)
        after = dnd.exists()
        if after == before:
            print("FAIL: quick settings popout did not receive DND tile click after idle")
            return 1

        if dk.crashed():
            print(f"FAIL: compositor crashed after input: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: notification history no longer uses a separate input-grabbing panel")
        return 0
    finally:
        center.unlink(missing_ok=True)
        dnd.unlink(missing_ok=True)
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
