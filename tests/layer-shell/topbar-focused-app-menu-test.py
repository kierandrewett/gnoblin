#!/usr/bin/env python3
# Regression: the focused-app widget in the topbar opens the same per-app
# context menu as the dock. Prove it through the pointer path: focus a real app,
# click the topbar app label, click the menu's Quit row, and assert the app closes.
import importlib.util
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def install_foot_desktop(dk):
    appdir = dk.tmp / "data" / "applications"
    appdir.mkdir(parents=True, exist_ok=True)
    (appdir / "foot.desktop").write_text(
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=Foot\n"
        "Exec=foot\n"
        "Icon=foot\n"
        "Terminal=false\n"
    )


def visible_foot_windows(dk):
    return [w for w in dk.list_windows() if w[2] == "foot" and not w[4]]


def wait_for_focused_foot(dk, timeout=10):
    deadline = time.time() + timeout
    while time.time() < deadline:
        windows = dk.list_windows()
        if any(w[2] == "foot" and w[3] and not w[4] for w in windows):
            return True
        time.sleep(0.25)
    return False


def wait_for_no_visible_foot(dk, timeout=8):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if not visible_foot_windows(dk):
            return True
        time.sleep(0.25)
    return not visible_foot_windows(dk)


def main():
    dk = dh.Devkit()
    try:
        install_foot_desktop(dk)
        dk.boot(with_monitor=True)
        time.sleep(2.0)
        if dk.crashed():
            print(f"FAIL: compositor crashed on boot: {dk.crashed()}")
            print(dk._tail())
            return 1

        dk.dispatch("spawn", "foot")
        if not wait_for_focused_foot(dk):
            print(f"FAIL: foot did not map and focus: windows={dk.list_windows()}")
            print(dk._tail(20))
            return 1

        # Use the real default topbar layout: workspaces, focused-app, appmenu,
        # spring | clock | launcher, tray, status. The focused-app x position
        # moves slightly with workspace count, so probe only the normal left
        # app-label band rather than replacing the layout under test.
        for x in (90, 110, 130, 150, 170):
            dk.click(x, 17)
            time.sleep(0.25)

            # Running-app menu with no desktop actions:
            # All Windows, sep, New Window, sep, Pin/Unpin, sep, App Details,
            # sep, Quit. ContextMenu y=34, top padding=8, so Quit row centre
            # is around y=239.
            dk.click(x, 239)
            if wait_for_no_visible_foot(dk, timeout=1.5):
                break

        if not wait_for_no_visible_foot(dk):
            print("FAIL: focused-app menu did not activate Quit")
            print(f"windows={dk.list_windows()}")
            print(dk._tail(25))
            return 1

        if dk.crashed():
            print(f"FAIL: compositor crashed after focused-app menu activation: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: focused-app widget opens the shared app context menu and activates Quit")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
