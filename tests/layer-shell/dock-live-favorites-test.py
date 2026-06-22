#!/usr/bin/env python3
# Regression: a running gnoblin-dock must notice live `[dock] favorites` edits.
#
# The compositor watches gnoblin.conf and the shipped example says config edits
# are live. The dock is a separate long-running layer-shell client, so it must
# reload its own favorites instead of only reading them at startup. This test
# starts a bare compositor, runs one dock with favorite `alpha`, rewrites
# gnoblin.conf to favorite `beta`, then clicks the single centered dock slot. If
# the dock did not reload, the click launches alpha instead of beta.
import importlib.util
import base64
import pathlib
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

PNG_1X1_MAGENTA = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGP4z/AfAAQAAf8iCjrw"
    "AAAAAElFTkSuQmCC"
)


def write_config(path, favorite):
    path.write_text(
        "[appearance]\n"
        'background = "#202434"\n'
        "[startup]\n"
        "[roles]\n"
        "window-menu = gnoblin-window-menu\n"
        "[bind]\n"
        "[dock]\n"
        f"favorites = {favorite}\n"
    )


def install_app(dk, app_id, marker):
    appdir = dk.tmp / "data" / "applications"
    icondir = dk.tmp / "data" / "icons" / "hicolor" / "48x48" / "apps"
    appdir.mkdir(parents=True, exist_ok=True)
    icondir.mkdir(parents=True, exist_ok=True)
    (icondir / f"{app_id}.png").write_bytes(PNG_1X1_MAGENTA)
    (appdir / f"{app_id}.desktop").write_text(
        "[Desktop Entry]\n"
        "Type=Application\n"
        f"Name={app_id}\n"
        f"Exec=sh -c \"printf {app_id} > {marker}\"\n"
        f"Icon={app_id}\n"
        "Terminal=false\n"
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


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    dk = dh.Devkit()
    proc = None
    logf = None
    try:
        alpha_marker = dk.tmp / "alpha-launched"
        beta_marker = dk.tmp / "beta-launched"
        install_app(dk, "alpha", alpha_marker)
        install_app(dk, "beta", beta_marker)
        dk.extra_conf = "[dock]\nfavorites = alpha\n"
        dk.boot(with_monitor=True)

        fake_bin = dk.tmp / "bin"
        fake_bin.mkdir()
        gtk_launch = fake_bin / "gtk-launch"
        gtk_launch.write_text("#!/bin/sh\nexit 1\n")
        gtk_launch.chmod(0o755)

        env = dk._env()
        env["WAYLAND_DISPLAY"] = dk.disp
        env["PATH"] = f"{fake_bin}:{env['PATH']}"
        log_path = dk.tmp / "dock-live-favorites.log"
        logf = open(log_path, "wb")
        proc = subprocess.Popen(
            [str(dh.PREFIX / "bin" / "gnoblin-dock")],
            env=env,
            stdout=logf,
            stderr=subprocess.STDOUT,
        )
        if not wait_for_process(dk, "gnoblin-dock", proc):
            print(f"FAIL: gnoblin-dock did not stay running rc={proc.returncode}")
            return 1

        time.sleep(1.0)
        conf = dk.tmp / "config" / "gnoblin" / "gnoblin.conf"
        write_config(conf, "beta")

        # One favorite means the sole slot is centered at x=640, y=752.
        deadline = time.time() + 6
        while time.time() < deadline and not beta_marker.exists() and not alpha_marker.exists():
            dk.click(640, 752)
            time.sleep(0.5)

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1
        if beta_marker.exists():
            print("PASS: running dock reloaded [dock] favorites after config edit")
            return 0
        if alpha_marker.exists():
            print("FAIL: dock launched stale favorite alpha after config changed to beta")
            return 1
        if log_path.exists():
            tail = "\n".join(log_path.read_text(errors="replace").splitlines()[-20:])
            print(f"  dock log tail:\n{tail}")
        print("FAIL: dock click launched neither favorite")
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
