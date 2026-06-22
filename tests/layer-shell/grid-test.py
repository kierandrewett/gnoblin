#!/usr/bin/env python3
# Regression test for the app grid (Phase 4): `gnoblin-launcher --grid` shows
# the same apps as the list launcher but in an icon grid. This boots, spawns the
# grid, and asserts it renders as a GRID (not the list): the blue selection
# highlight is a single square tile (~one column wide), not a full-width row.
# Then Escape closes it, and a second grid instance is clicked to launch a real
# app. Process- and pixel-based, no OCR.
import shutil
import subprocess
import sys, time, importlib.util, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

try:
    from PIL import Image
except Exception:
    print("SKIP: no PIL")
    sys.exit(0)


def launchers(dk):
    return dk.processes("gnoblin-launcher")


def foot_visible(dk):
    return any(w[2] == "foot" and not w[4] for w in dk.list_windows())


def widest_blue_run(png):
    """Width (px) of the widest horizontal run of selection-blue (#3584e4).
    A grid tile is ~one column (~120px); a list row spans the panel (~580px)."""
    im = Image.open(png).convert("RGB")
    w, h = im.size
    px = im.load()
    best = 0
    for y in range(0, h, 2):
        run = 0
        for x in range(w):
            r, g, b = px[x, y]
            if 40 <= r <= 72 and 112 <= g <= 152 and 205 <= b <= 245:
                run += 1
                best = max(best, run)
            else:
                run = 0
    return best


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


def run_render_and_escape_check():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)
        time.sleep(5)
        if dk.crashed():
            print(f"FAIL: compositor crashed on boot: {dk.crashed()}")
            return 1

        dk.dispatch("spawn", "gnoblin-launcher --grid")
        time.sleep(4)
        if len(launchers(dk)) != 1:
            print(f"FAIL: app grid did not spawn: {launchers(dk)}")
            return 1

        dk.shot("/tmp/gnoblin-grid-open.png")
        blue = widest_blue_run("/tmp/gnoblin-grid-open.png")
        print(f"  widest selection-blue run: {blue}px")

        # Warm the RemoteDesktop session before the close keystroke — the very
        # first injected key right after the session starts can be dropped.
        dk.start_remote_desktop()
        time.sleep(0.5)
        closed = launchers(dk)
        for _ in range(3):
            dk.send_combo("escape")
            time.sleep(1.5)
            closed = launchers(dk)
            if not closed:
                break

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            return 1
        # The grid highlight is a square tile: present, but narrow (one column).
        # A wide run would mean the list layout rendered instead.
        if blue < 60:
            print(f"FAIL: no grid selection tile visible ({blue}px)")
            return 1
        if blue > 280:
            print(f"FAIL: selection spans a full row — list layout, not a grid ({blue}px)")
            return 1
        if closed:
            print(f"FAIL: grid did not close on Escape: {closed}")
            return 1
        return 0
    finally:
        dk.teardown()


def run_click_launch_check():
    dk = dh.Devkit()
    proc = None
    try:
        dk.boot(with_monitor=True)
        install_foot_desktop(dk)
        time.sleep(5)
        if dk.crashed():
            print(f"FAIL: compositor crashed on boot before click: {dk.crashed()}")
            return 1

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_LAUNCHER_QUERY"] = "foot"
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-launcher"), "--grid"],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        deadline = time.time() + 8
        while time.time() < deadline:
            if len(launchers(dk)) == 1:
                break
            if proc.poll() is not None:
                print(f"FAIL: app grid exited before click rc={proc.returncode}")
                return 1
            time.sleep(0.25)
        if len(launchers(dk)) != 1:
            print(f"FAIL: app grid did not stay open for click: {launchers(dk)}")
            return 1

        # 1280x800 default monitor: grid panel is 660x560 centered at (310,120).
        # The first tile starts below the search field at y ~= 188 and has a
        # width ~= 126, so this lands in the middle of the filtered "foot" tile.
        dk.click(373, 244)
        deadline = time.time() + 10
        while time.time() < deadline:
            if foot_visible(dk) and not launchers(dk):
                break
            time.sleep(0.25)

        if not foot_visible(dk):
            print(f"FAIL: clicking app grid tile did not map foot: windows={dk.list_windows()}")
            print(dk._tail(20))
            return 1
        if launchers(dk):
            print(f"FAIL: app grid stayed open after tile activation: {launchers(dk)}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed after grid click: {dk.crashed()}")
            print(dk._tail())
            return 1
        print("  ok  app grid tile click launched foot")
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
    if shutil.which("foot") is None:
        print("SKIP: no foot")
        return 0
    rc = run_render_and_escape_check()
    if rc != 0:
        return rc
    rc = run_click_launch_check()
    if rc != 0:
        return rc
    print("PASS: app grid renders, closes on Escape, and launches a clicked tile")
    return 0


if __name__ == "__main__":
    sys.exit(main())
