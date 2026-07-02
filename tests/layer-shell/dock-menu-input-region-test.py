#!/usr/bin/env python3
# Regression: the dock context menu is a resident daemon, not a dock-owned
# headroom/input-region trick. The dock must trigger dev.gnoblin.Menu.Show, the
# menu and full-screen scrim must map from the same daemon pid, a scrim click
# must dismiss both, and the daemon must remain alive for the next show.
import importlib.util
import json
import os
import pathlib
import re
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def pid_from_process(lines):
    for line in lines:
        match = re.match(r"^(\d+)\s+", line)
        if match:
            return int(match.group(1))
    return None


def mapped(surface):
    buf = surface.get("buffer") or [0, 0, 0, 0]
    actor = surface.get("actor") or {}
    return bool(actor.get("mapped")) and len(buf) >= 4 and buf[2] > 0 and buf[3] > 0


def surfaces(dk, namespace):
    scene = dk.inspect_scene()
    return [s for s in scene.get("surfaces", []) if s.get("layer_ns") == namespace]


def wait_for_mapped(dk, namespace, timeout=6.0):
    deadline = time.time() + timeout
    last = []
    while time.time() < deadline:
        last = [s for s in surfaces(dk, namespace) if mapped(s)]
        if last:
            return last
        time.sleep(0.05)
    return last


def wait_gone(dk, namespace, timeout=6.0):
    deadline = time.time() + timeout
    last = []
    while time.time() < deadline:
        last = [s for s in surfaces(dk, namespace) if mapped(s)]
        if not last:
            return []
        time.sleep(0.05)
    return last


def element_texts(dk, pid):
    runtime = pathlib.Path(dk._env().get("XDG_RUNTIME_DIR", ""))
    path = runtime / "gnoblin-inspect" / f"elements-{pid}.json"
    if not path.exists():
        return []
    try:
        data = json.loads(path.read_text())
    except (OSError, ValueError):
        return []
    return [el.get("text") for el in data if el.get("text")]


def start_client(dk, argv, extra_env=None, log_name="client.log"):
    env = dk._env()
    env["WAYLAND_DISPLAY"] = dk.disp
    if extra_env:
        env.update(extra_env)
    log_path = dk.tmp / log_name
    logf = open(log_path, "wb")
    proc = subprocess.Popen(
        argv,
        env=env,
        stdout=logf,
        stderr=subprocess.STDOUT,
    )
    return proc, logf, log_path


def main():
    old_clients = dh.CLIENTS
    dh.CLIENTS = False
    menu_proc = dock_proc = None
    menu_log = dock_log = None
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True)

        menu_proc, menu_log, _ = start_client(
            dk,
            [str(dh.PREFIX / "bin" / "gnoblin-menu"), "--daemon"],
            log_name="menu-daemon.log",
        )
        deadline = time.time() + 5
        menu_pid = None
        while time.time() < deadline:
            menu_pid = pid_from_process(dk.processes("gnoblin-menu"))
            if menu_pid:
                break
            if menu_proc.poll() is not None:
                print(f"FAIL: gnoblin-menu daemon exited rc={menu_proc.returncode}")
                return 1
            time.sleep(0.05)
        if not menu_pid:
            print("FAIL: gnoblin-menu daemon did not stay running")
            return 1

        dock_proc, dock_log, _ = start_client(
            dk,
            [str(dh.PREFIX / "bin" / "gnoblin-dock")],
            {
                "GNOBLIN_DOCK_MENU": "foot",
                "GNOBLIN_DOCK_MENU_RUNNING": "1",
            },
            log_name="dock-menu-daemon.log",
        )
        time.sleep(1.0)
        if dock_proc.poll() is not None:
            print(f"FAIL: gnoblin-dock exited early rc={dock_proc.returncode}")
            return 1

        menu = wait_for_mapped(dk, "gnoblin-menu")
        scrim = wait_for_mapped(dk, "gnoblin-menu-scrim")
        if not menu or not scrim:
            print("FAIL: dock did not trigger resident menu+scrim")
            print(f"  menu={menu}")
            print(f"  scrim={scrim}")
            return 1
        if menu[0].get("pid") != menu_pid or scrim[0].get("pid") != menu_pid:
            print("FAIL: menu/scrim were not owned by the resident daemon pid")
            print(f"  daemon={menu_pid} menu={menu[0].get('pid')} scrim={scrim[0].get('pid')}")
            return 1

        buf = menu[0].get("buffer") or [0, 0, 0, 0]
        texts = element_texts(dk, menu_pid)
        print(f"  daemon pid={menu_pid}")
        print(f"  menu frame={menu[0].get('frame')} buffer={buf}")
        print(f"  scrim frame={scrim[0].get('frame')} buffer={scrim[0].get('buffer')}")
        print(f"  item texts={texts[:8]}")
        if len(buf) < 4 or buf[2] != 220 or buf[3] <= 12:
            print("FAIL: dock-triggered menu is not content-sized")
            return 1
        if not any(t in ("Open", "New Window", "Pin to Dock", "Unpin from Dock", "Quit") for t in texts):
            print("FAIL: dock-triggered menu did not render expected items")
            return 1

        dk.click(20, 20)
        if wait_gone(dk, "gnoblin-menu") or wait_gone(dk, "gnoblin-menu-scrim"):
            print("FAIL: scrim click did not dismiss menu surfaces")
            return 1
        if pid_from_process(dk.processes("gnoblin-menu")) != menu_pid:
            print("FAIL: gnoblin-menu daemon exited after dismiss")
            return 1

        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: dock triggers resident menu daemon and scrim dismisses it")
        return 0
    finally:
        for proc in (dock_proc, menu_proc):
            if proc and proc.poll() is None:
                proc.terminate()
                try:
                    proc.wait(timeout=2)
                except Exception:
                    proc.kill()
        for logf in (dock_log, menu_log):
            if logf:
                logf.close()
        dk.teardown()
        dh.CLIENTS = old_clients


if __name__ == "__main__":
    sys.exit(main())
