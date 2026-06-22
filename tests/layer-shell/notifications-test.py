#!/usr/bin/env python3
# Regression test for the notification pipeline: gnoblin-notifyd owns
# org.freedesktop.Notifications, renders transient toasts, writes history for
# the topbar quick-settings popout, and keeps the old gnoblin-notify-center
# command as a no-op compatibility shim.
#
# Asserts: notifyd is running; notify-send shows a top-right popup; the
# compatibility command does not resurrect the removed right-side panel; quick
# settings renders a collapsed notification history stack below the grid; the
# stack expands in place when clicked.
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

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)

if not subprocess.run(["sh", "-c", "command -v notify-send"],
                      capture_output=True).returncode == 0:
    print("SKIP: no notify-send")
    sys.exit(0)


def runtime_file(name):
    runtime = pathlib.Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    return runtime / name


FILES = [
    runtime_file("gnoblin-notif-center"),
    runtime_file("gnoblin-notif-pending"),
    runtime_file("gnoblin-notif-summary"),
    runtime_file("gnoblin-notif-history"),
]


def load(path):
    return Image.open(path).convert("RGB").load()


def changed(a, b, x, y, thr):
    return sum(abs(p - q) for p, q in zip(a[x, y], b[x, y])) > thr


def region_changed(a, b, xs, ys, thr, min_hits):
    hits = 0
    for x in xs:
        for y in ys:
            if changed(a, b, x, y, thr):
                hits += 1
    return hits >= min_hits


def clear_runtime_files():
    for path in FILES:
        path.unlink(missing_ok=True)


def open_quick_settings(dk):
    width = int(os.environ.get("MONITOR", "1280x800").split("x", 1)[0])
    dk.click(width - 48, 17)
    time.sleep(0.8)


def close_quick_settings(dk):
    width = int(os.environ.get("MONITOR", "1280x800").split("x", 1)[0])
    dk.click(width - 48, 17)
    time.sleep(0.5)


def run_compat_toggle(env):
    return subprocess.run(
        [str(dh.PREFIX / "bin" / "gnoblin-notify-center")],
        env=env,
        capture_output=True,
        text=True,
    )


def send_notification(env, summary, body):
    return subprocess.run(
        ["notify-send", summary, body],
        env=env,
        capture_output=True,
        text=True,
    )


def notifyd_alive(dk):
    procs = dk.processes("gnoblin-notifyd")
    if not procs:
        print("FAIL: gnoblin-notifyd exited")
        print(dk._tail())
        return False
    return True


def main():
    clear_runtime_files()
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(6)
        if not notifyd_alive(dk):
            return 1

        empty_png = "/tmp/gnoblin-notif-empty.png"
        dk.shot(empty_png)
        empty = load(empty_png)

        open_quick_settings(dk)
        clean_cc_png = "/tmp/gnoblin-notif-clean-cc.png"
        dk.shot(clean_cc_png)
        clean_cc = load(clean_cc_png)
        close_quick_settings(dk)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        result = send_notification(env, "Hello", "This is a test notification")
        if result.returncode != 0:
            print(f"FAIL: notify-send failed: {result.stderr.strip()}")
            return 1

        time.sleep(1.2)
        pop_png = "/tmp/gnoblin-notif-popup.png"
        dk.shot(pop_png)
        pop = load(pop_png)
        if not changed(empty, pop, 1090, 55, thr=18):
            print("FAIL: notification popup did not render (top-right toast)")
            return 1
        print("  ok  notification popup rendered")

        result = run_compat_toggle(env)
        if result.returncode != 0:
            print(f"FAIL: gnoblin-notify-center failed: {result.stderr.strip()}")
            return 1
        time.sleep(1.0)
        if FILES[0].exists():
            print("FAIL: compatibility command reopened the removed notification panel flag")
            return 1
        if not notifyd_alive(dk):
            return 1
        print("  ok  compatibility notification-center command is inert")

        for idx in range(2, 4):
            result = send_notification(
                env,
                f"Hello {idx}",
                f"This is stacked notification {idx}",
            )
            if result.returncode != 0:
                print(f"FAIL: notify-send {idx} failed: {result.stderr.strip()}")
                return 1
            time.sleep(0.25)

        history = FILES[3].read_text(encoding="utf-8") if FILES[3].exists() else ""
        if len([line for line in history.splitlines() if line.strip()]) < 3:
            print("FAIL: notification history did not record multiple entries for stacking")
            return 1

        open_quick_settings(dk)
        cc_png = "/tmp/gnoblin-notif-cc-history.png"
        dk.shot(cc_png)
        cc = load(cc_png)
        if not region_changed(clean_cc, cc, range(930, 1240, 35), range(530, 630, 20), 14, 8):
            print("FAIL: quick settings did not render notification history below the grid")
            return 1
        print("  ok  quick settings notification stack rendered")

        dk.click(1000, 535)
        time.sleep(0.8)
        expanded_png = "/tmp/gnoblin-notif-cc-history-expanded.png"
        dk.shot(expanded_png)
        expanded = load(expanded_png)
        if not region_changed(cc, expanded, range(930, 1240, 35), range(610, 700, 20), 10, 8):
            print("FAIL: notification stack did not expand to show additional rows")
            return 1
        print("  ok  quick settings notification stack expands in place")

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: notifications render as toast + quick-settings history")
        return 0
    finally:
        dk.teardown()
        clear_runtime_files()


if __name__ == "__main__":
    sys.exit(main())
