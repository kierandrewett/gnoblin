#!/usr/bin/env python3
# Regression: the launcher must not merely open; pressing Return on a filtered
# result and clicking a filtered list row must launch the selected desktop app
# and close the launcher.
import importlib.util
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def launchers(dk):
    return dk.processes("gnoblin-launcher")


def foot_visible(dk):
    return any(w[2] == "foot" and not w[4] for w in dk.list_windows())


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


def wait_for_foot_and_launcher_close(dk, timeout=10):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if foot_visible(dk) and not launchers(dk):
            return True
        time.sleep(0.25)
    return foot_visible(dk) and not launchers(dk)


def run_return_check():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if dk.crashed():
            print(f"FAIL: compositor crashed on boot: {dk.crashed()}")
            return 1

        dk.send_combo("Super+Space")
        time.sleep(2)
        if len(launchers(dk)) != 1:
            print(f"FAIL: launcher did not open: {launchers(dk)}")
            return 1

        dk.type_text("foot")
        time.sleep(0.5)
        dk.send_combo("Return")
        deadline = time.time() + 8
        while time.time() < deadline:
            if foot_visible(dk):
                break
            time.sleep(0.25)

        if not foot_visible(dk):
            print(f"FAIL: Return in launcher did not map foot: windows={dk.list_windows()}")
            print(dk._tail(20))
            return 1
        if launchers(dk):
            print(f"FAIL: launcher stayed open after activation: {launchers(dk)}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("  ok  launcher Return activated foot")
        return 0
    finally:
        dk.teardown()


def run_list_click_check():
    dk = dh.Devkit()
    proc = None
    try:
        dk.boot(with_monitor=True)
        install_foot_desktop(dk)
        time.sleep(2)
        if dk.crashed():
            print(f"FAIL: compositor crashed before list click: {dk.crashed()}")
            return 1

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_LAUNCHER_QUERY"] = "foot"
        empty_data_dirs = dk.tmp / "empty-data-dirs"
        empty_data_dirs.mkdir()
        env["XDG_DATA_DIRS"] = str(empty_data_dirs)
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-launcher")],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        deadline = time.time() + 8
        while time.time() < deadline:
            if len(launchers(dk)) == 1:
                break
            if proc.poll() is not None:
                print(f"FAIL: launcher exited before list click rc={proc.returncode}")
                return 1
            time.sleep(0.25)
        if len(launchers(dk)) != 1:
            print(f"FAIL: launcher did not stay open for list click: {launchers(dk)}")
            return 1

        # 1280x800 default monitor: the Spotlight panel is 600 wide, centred on
        # x=640, anchored at y=0.16*800=128. Search field 60px + 1px hairline +
        # 4px pad, rows 52px → the first result row's centre is ~y=219.
        dk.click(640, 219)
        if not wait_for_foot_and_launcher_close(dk):
            print(f"FAIL: clicking launcher list row did not map foot/close launcher")
            print(f"windows={dk.list_windows()}")
            print(f"launchers={launchers(dk)}")
            print(dk._tail(20))
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed after launcher list click: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("  ok  launcher list row click activated foot")
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        dk.teardown()


def main():
    rc = run_return_check()
    if rc != 0:
        return rc
    rc = run_list_click_check()
    if rc != 0:
        return rc
    print("PASS: launcher Return and list-row click activate selected apps and close")
    return 0


if __name__ == "__main__":
    sys.exit(main())
