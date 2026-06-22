#!/usr/bin/env python3
# Regression: dock launch uses the shared desktop-id resolver. Legacy favorites
# like "firefox" should resolve to reverse-DNS desktop IDs, and nested desktop
# files like applications/RPCS3/RPCS3.desktop should resolve to the XDG ID
# RPCS3-RPCS3 before calling gtk-launch.
import importlib.util
import pathlib
import base64
import os
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


PNG_1X1_MAGENTA = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADUlEQVR4nGP4z8DwHwAFBQIA"
    "eVYV9QAAAABJRU5ErkJggg=="
)


def foot_windows(dk):
    return [w for w in dk.list_windows() if w[2] == "foot" and not w[4]]


def install_desktop_file(appdir, name, text):
    path = appdir / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    return path


def install_foot_fixture(dk):
    appdir = dk.tmp / "data" / "applications"
    icondir = dk.tmp / "data" / "icons" / "hicolor" / "48x48" / "apps"
    cfgdir = dk.tmp / "config" / "gnoblin"
    appdir.mkdir(parents=True, exist_ok=True)
    icondir.mkdir(parents=True, exist_ok=True)
    cfgdir.mkdir(parents=True, exist_ok=True)
    (icondir / "foot.png").write_bytes(PNG_1X1_MAGENTA)
    (cfgdir / "dock-favorites").write_text("foot\n")
    install_desktop_file(
        appdir,
        "foot.desktop",
        "[Desktop Entry]\n"
        "Type=Application\n"
        "Name=Foot\n"
        "Exec=foot\n"
        "Icon=foot\n"
        "Terminal=false\n",
    )


def wait_for_foot_count(dk, more_than, timeout=10):
    deadline = time.time() + timeout
    while time.time() < deadline:
        count = len(foot_windows(dk))
        if count > more_than:
            return count
        time.sleep(0.25)
    return len(foot_windows(dk))


def run_real_dock_click_check():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    try:
        dk.boot(with_monitor=True)
        install_foot_fixture(dk)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-dock")],
            env=env,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

        deadline = time.time() + 8
        while time.time() < deadline:
            if dk.processes("gnoblin-dock"):
                break
            if proc.poll() is not None:
                print(f"FAIL: gnoblin-dock exited before real click rc={proc.returncode}")
                return 1
            time.sleep(0.25)

        # One favourite means the dock pill is centered. Slot size is 64px and
        # pill padding is 8px, so the first slot center is monitor_center_x.
        # The bottom-anchored 296px surface starts at y=504; the dock slot center
        # is at 504 + 200 headroom + 8 pill inset + 8 row inset + 32 slot half.
        before = len(foot_windows(dk))
        dk.click(640, 752)
        count = wait_for_foot_count(dk, before)
        if count <= before:
            print(f"FAIL: real dock icon click did not launch foot: windows={dk.list_windows()}")
            print(dk._tail(20))
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed after real dock click: {dk.crashed()}")
            print(dk._tail())
            return 1
        print("  ok  real dock icon click launched foot")
        return 0
    finally:
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        dk.teardown()
        dh.CLIENTS = old_clients


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    try:
        appdir = dk.tmp / "data" / "applications"
        appdir.mkdir(parents=True, exist_ok=True)
        nested = appdir / "RPCS3"
        nested.mkdir()
        install_desktop_file(
            nested,
            "RPCS3.desktop",
            "[Desktop Entry]\n"
            "Type=Application\n"
            "Name=Fake RPCS3\n"
            "Exec=foot\n"
            "Icon=foot\n"
            "Terminal=false\n"
        )
        install_desktop_file(
            appdir,
            "org.example.DBusApp.desktop",
            "[Desktop Entry]\n"
            "Type=Application\n"
            "Name=Fake DBus App\n"
            "Exec=foot\n"
            "Icon=foot\n"
            "DBusActivatable=true\n"
            "Terminal=false\n"
        )
        install_desktop_file(
            appdir,
            "terminal.desktop",
            "[Desktop Entry]\n"
            "Type=Application\n"
            "Name=Fake Terminal App\n"
            "Exec=sh -c \"sleep 5\"\n"
            "Icon=foot\n"
            "Terminal=true\n"
        )
        fake_bin = dk.tmp / "bin"
        fake_bin.mkdir()
        fake_gtk_launch = fake_bin / "gtk-launch"
        fake_gtk_launch.write_text("#!/bin/sh\nexit 0\n")
        fake_gtk_launch.chmod(0o755)

        dk.boot(with_monitor=True)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_DOCK_LAUNCH"] = "RPCS3"
        log_path = dk.tmp / "dock-launch.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-dock")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        deadline = time.time() + 10
        while time.time() < deadline:
            if any(w[2] == "foot" and not w[4] for w in dk.list_windows()):
                break
            if proc.poll() is not None:
                print(f"FAIL: gnoblin-dock exited early rc={proc.returncode}")
                print(log_path.read_text(errors="replace"))
                return 1
            time.sleep(0.25)

        if not any(w[2] == "foot" and not w[4] for w in dk.list_windows()):
            print(f"FAIL: dock nested desktop ID did not launch foot: windows={dk.list_windows()}")
            print(log_path.read_text(errors="replace"))
            return 1

        logf.flush()
        text = log_path.read_text(errors="replace")
        if "desktop app 'RPCS3' resolved to 'RPCS3-RPCS3'" not in text:
            print("FAIL: dock launch did not use nested XDG desktop-id resolution")
            print(text)
            return 1

        before = len([w for w in dk.list_windows() if w[2] == "foot" and not w[4]])
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        if logf:
            logf.close()
            logf = None

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_DOCK_LAUNCH"] = "org.example.DBusApp"
        env["PATH"] = f"{fake_bin}{os.pathsep}{env.get('PATH', '')}"
        log_path = dk.tmp / "dock-dbus-launch.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-dock")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        deadline = time.time() + 10
        while time.time() < deadline:
            foot_count = len([w for w in dk.list_windows() if w[2] == "foot" and not w[4]])
            if foot_count > before:
                break
            if proc.poll() is not None:
                print(f"FAIL: gnoblin-dock exited early rc={proc.returncode}")
                print(log_path.read_text(errors="replace"))
                return 1
            time.sleep(0.25)

        foot_count = len([w for w in dk.list_windows() if w[2] == "foot" and not w[4]])
        if foot_count <= before:
            print("FAIL: DBusActivatable desktop entry used gtk-launch success without Exec fallback")
            print(f"windows={dk.list_windows()}")
            print(log_path.read_text(errors="replace"))
            return 1

        before = foot_count
        if proc and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=2)
            except Exception:
                proc.kill()
        if logf:
            logf.close()
            logf = None

        fake_gtk_launch.write_text("#!/bin/sh\nexit 1\n")
        fake_gtk_launch.chmod(0o755)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_DOCK_LAUNCH"] = "terminal"
        env["GNOBLIN_TERMINAL"] = "foot"
        env["PATH"] = f"{fake_bin}{os.pathsep}{env.get('PATH', '')}"
        log_path = dk.tmp / "dock-terminal-launch.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-dock")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        deadline = time.time() + 10
        while time.time() < deadline:
            foot_count = len([w for w in dk.list_windows() if w[2] == "foot" and not w[4]])
            if foot_count > before:
                break
            if proc.poll() is not None:
                print(f"FAIL: gnoblin-dock exited early rc={proc.returncode}")
                print(log_path.read_text(errors="replace"))
                return 1
            time.sleep(0.25)

        foot_count = len([w for w in dk.list_windows() if w[2] == "foot" and not w[4]])
        if foot_count <= before:
            print("FAIL: Terminal=true desktop entry did not map through terminal fallback")
            print(f"windows={dk.list_windows()}")
            print(log_path.read_text(errors="replace"))
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        rc = run_real_dock_click_check()
        if rc != 0:
            return rc
        print("PASS: dock launch resolves nested IDs, DBusActivatable apps use Exec, terminal apps use a terminal fallback, and real icon clicks launch")
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
