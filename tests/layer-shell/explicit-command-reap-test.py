#!/usr/bin/env python3
# Regression: gnoblin-shell's explicit `-- COMMAND` launch path must not leave
# short-lived children as zombies. Autostart clients use child watches for
# respawn; this one-shot path should let GLib reap the child automatically.
import importlib.util
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def zombie_children_of(pid):
    zombies = []
    for proc in pathlib.Path("/proc").glob("[0-9]*"):
        try:
            parts = (proc / "stat").read_text().split()
            if len(parts) < 4:
                continue
            state = parts[2]
            ppid = int(parts[3])
            if ppid == pid and state == "Z":
                comm = (proc / "comm").read_text(errors="replace").strip()
                zombies.append((proc.name, comm))
        except Exception:
            pass
    return zombies


def main():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=True, command=["/bin/true"])
        time.sleep(1.0)
        if dk.crashed():
            print(f"FAIL: compositor crashed after explicit command: {dk.crashed()}")
            print(dk._tail())
            return 1

        zombies = zombie_children_of(dk.shell_proc.pid)
        if zombies:
            print(f"FAIL: zombie explicit command child observed: {zombies}")
            return 1

        print("PASS: short-lived explicit command child was reaped")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
