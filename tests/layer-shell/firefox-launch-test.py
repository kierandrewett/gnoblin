#!/usr/bin/env python3
# Regression: launching Firefox through the dock's desktop-entry path must not
# kill the compositor. This covers the real short-id resolver (`firefox` ->
# `org.mozilla.firefox`) and `gtk-launch`, but uses an isolated profile so the
# test cannot attach to or mutate the user's normal browser session.
import importlib.util
import pathlib
import shutil
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def firefox_windows(dk):
    out = []
    for window in dk.list_windows():
        title = (window[1] or "").lower()
        app_id = (window[2] or "").lower()
        if window[4]:
            continue
        if "firefox" in title or "firefox" in app_id or "mozilla" in app_id:
            out.append(window)
    return out


def main():
    firefox = shutil.which("firefox")
    if not firefox:
        print("SKIP: no firefox")
        return 0

    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    try:
        appdir = dk.tmp / "data" / "applications"
        appdir.mkdir(parents=True, exist_ok=True)
        profile = dk.tmp / "firefox-profile"
        profile.mkdir()
        (appdir / "org.mozilla.firefox.desktop").write_text(
            "[Desktop Entry]\n"
            "Type=Application\n"
            "Name=Firefox Test\n"
            f"Exec={firefox} --new-instance --profile {profile} about:blank\n"
            "Icon=firefox\n"
            "Terminal=false\n"
        )

        dk.boot(with_monitor=True)
        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["GNOBLIN_DOCK_LAUNCH"] = "firefox"
        env["MOZ_ENABLE_WAYLAND"] = "1"
        env["MOZ_DBUS_REMOTE"] = "0"
        log_path = dk.tmp / "dock-firefox-launch.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-dock")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )

        mapped = []
        deadline = time.time() + 35
        while time.time() < deadline:
            if proc.poll() is not None:
                print(f"FAIL: gnoblin-dock exited before Firefox mapped rc={proc.returncode}")
                print(log_path.read_text(errors="replace"))
                return 1
            crash = dk.crashed()
            if crash:
                print(f"FAIL: compositor crashed before Firefox mapped: {crash}")
                print(dk._tail(80))
                return 1
            mapped = firefox_windows(dk)
            if mapped:
                break
            time.sleep(0.5)

        if not mapped:
            print(f"FAIL: Firefox did not map: windows={dk.list_windows()}")
            print(log_path.read_text(errors="replace"))
            print(dk._tail(80))
            return 1

        # Keep the real Firefox window alive long enough to catch delayed
        # compositor faults from initial configure/map/activation handling.
        deadline = time.time() + 5
        while time.time() < deadline:
            crash = dk.crashed()
            if crash:
                print(f"FAIL: compositor crashed after Firefox mapped: {crash}")
                print(dk._tail(80))
                return 1
            time.sleep(0.5)

        text = log_path.read_text(errors="replace")
        if "desktop app 'firefox' resolved to 'org.mozilla.firefox'" not in text:
            print("FAIL: dock launch did not use the Firefox short-id resolver")
            print(text)
            return 1

        print(f"PASS: dock Firefox launch mapped and compositor survived: {mapped[0]}")
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
