#!/usr/bin/env python3
# Regression: a modal full-screen window-menu layer must take input while it is
# mapped, then release that same stationary pointer position to the app below
# when it exits.
import importlib.util
import pathlib
import re
import subprocess
import sys
import time

from gi.repository import Gio, GLib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

BUTTON = re.compile(r"wl_pointer[@#]\d+\.button\(")
BUTTON_ARGS = re.compile(r"wl_pointer[@#]\d+\.button\(([^)]*)\)")


def button_count(path):
    return len(BUTTON.findall(path.read_text(errors="replace")))


def button_presses(path):
    presses = 0
    for args in BUTTON_ARGS.findall(path.read_text(errors="replace")):
        parts = [p.strip() for p in args.split(",")]
        if not parts:
            continue
        state = parts[-1].lower()
        if state in ("1", "pressed", "wl_pointer.button_state.pressed"):
            presses += 1
    return presses


def wait_for_presses(path, at_least, timeout=3.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        count = button_presses(path)
        if count >= at_least:
            return count
        time.sleep(0.05)
    return button_presses(path)


def pointer_button(dk, button=272):
    dk.start_remote_desktop(pointer=True)
    for pressed in (True, False):
        dk._rd_session.call_sync(
            "NotifyPointerButton",
            GLib.Variant("(ib)", (button, pressed)),
            Gio.DBusCallFlags.NONE,
            -1,
            None,
        )
        time.sleep(0.05)


def reliable_move(dk, x, y):
    dk.move(x, y)
    time.sleep(0.1)
    dk.move(x, y)
    time.sleep(0.1)


def foot_window_id(dk):
    for window_id, _title, app_id, _focused, minimized in dk.list_windows():
        if app_id == "foot" and not minimized:
            return window_id
    return None


def wait_for_foot(dk, proc, timeout=10.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if proc.poll() is not None:
            return None
        window_id = foot_window_id(dk)
        if window_id is not None:
            return window_id
        time.sleep(0.25)
    return None


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    foot_proc = None
    foot_log = None
    menu_proc = None
    menu_log = None
    try:
        dk.boot(with_monitor=True)
        dk.start_remote_desktop(pointer=True)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp

        foot_env = dict(env)
        foot_env["WAYLAND_DEBUG"] = "1"
        foot_log_path = dk.tmp / "foot-under-window-menu-wayland.log"
        foot_log = open(foot_log_path, "wb")
        foot_proc = subprocess.Popen(
            ["foot"],
            env=foot_env,
            stdout=foot_log,
            stderr=subprocess.STDOUT,
        )

        window_id = wait_for_foot(dk, foot_proc)
        if window_id is None:
            if foot_proc.poll() is not None:
                print(f"FAIL: foot exited before mapping rc={foot_proc.returncode}")
            else:
                print(f"FAIL: foot did not map: windows={dk.list_windows()}")
            return 1

        dk.dispatch("maximize")
        time.sleep(0.5)

        # Park the pointer outside the menu's future rectangle before mapping
        # the menu. The following button-only events must work without pointer
        # motion papering over stale focus.
        reliable_move(dk, 640, 400)

        menu_env = dict(env)
        menu_env["WAYLAND_DEBUG"] = "1"
        menu_log_path = dk.tmp / "window-menu-wayland.log"
        menu_log = open(menu_log_path, "wb")
        menu_proc = subprocess.Popen(
            [
                str(dh.PREFIX / "bin" / "gnoblin-window-menu"),
                "--role", "window-menu",
                "--window", str(window_id),
                "--output", "Meta-0",
                "--x", "100",
                "--y", "100",
                "--reason", "test",
            ],
            env=menu_env,
            stdout=menu_log,
            stderr=subprocess.STDOUT,
        )

        time.sleep(1.5)
        if menu_proc.poll() is not None:
            print(f"FAIL: gnoblin-window-menu exited before dismiss rc={menu_proc.returncode}")
            print(menu_log_path.read_text(errors="replace")[-1500:])
            return 1

        foot_before = button_count(foot_log_path)
        foot_presses_before = button_presses(foot_log_path)
        menu_before = button_count(menu_log_path)
        menu_presses_before = button_presses(menu_log_path)
        pointer_button(dk)

        deadline = time.time() + 4.0
        while time.time() < deadline and menu_proc.poll() is None:
            time.sleep(0.05)
        if menu_proc.poll() is None:
            print("FAIL: button-only outside click did not dismiss window menu")
            print(menu_log_path.read_text(errors="replace")[-1500:])
            return 1

        menu_after = button_count(menu_log_path)
        menu_presses_after = wait_for_presses(menu_log_path, menu_presses_before + 1, timeout=0.5)
        foot_after_dismiss = button_count(foot_log_path)
        foot_presses_after_dismiss = button_presses(foot_log_path)
        print(
            f"  dismiss: menu buttons {menu_before}->{menu_after} "
            f"presses {menu_presses_before}->{menu_presses_after}; "
            f"foot buttons {foot_before}->{foot_after_dismiss} "
            f"presses {foot_presses_before}->{foot_presses_after_dismiss}"
        )
        if menu_presses_after < menu_presses_before + 1:
            print("FAIL: window menu did not receive the dismiss pointer press")
            return 1
        if foot_presses_after_dismiss != foot_presses_before:
            print("FAIL: app below received the dismiss pointer press while window menu was mapped")
            return 1

        pointer_button(dk)
        foot_presses_after_release = wait_for_presses(foot_log_path, foot_presses_before + 1)
        foot_after_release = button_count(foot_log_path)
        print(
            f"  after menu exit: foot buttons {foot_before}->{foot_after_release} "
            f"presses {foot_presses_before}->{foot_presses_after_release}"
        )
        if foot_presses_after_release < foot_presses_before + 1:
            print("FAIL: app below did not receive pointer input after window menu exited")
            print(dk._tail(80))
            return 1

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: window-menu modal input is released to the app underneath")
        return 0
    finally:
        for proc in (menu_proc, foot_proc):
            if proc and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=2)
                except Exception:
                    proc.kill()
        for log in (menu_log, foot_log):
            if log:
                log.close()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
