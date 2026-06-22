#!/usr/bin/env python3
# Regression: short-lived [roles] children must be reaped. The window-menu role
# is often an on-demand shell client that exits after a pick/dismiss; spawning it
# with G_SPAWN_DO_NOT_REAP_CHILD but no child watch leaves zombies under
# gnoblin-shell.
import importlib.util
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def child_zombies(ppid):
    zombies = []
    for proc in pathlib.Path("/proc").iterdir():
        if not proc.name.isdigit():
            continue
        try:
            status = (proc / "status").read_text(errors="replace")
            cmd = (proc / "cmdline").read_bytes().replace(b"\0", b" ").decode(
                errors="replace"
            ).strip()
        except (FileNotFoundError, PermissionError, ProcessLookupError):
            continue

        fields = {}
        for line in status.splitlines():
            if ":" in line:
                key, value = line.split(":", 1)
                fields[key] = value.strip()
        if fields.get("PPid") == str(ppid) and fields.get("State", "").startswith("Z"):
            zombies.append((proc.name, fields.get("Name", ""), cmd))
    return zombies


def main():
    dk = dh.Devkit()
    try:
        dk.extra_conf = "\n[roles]\nwindow-menu = /bin/true\n"
        dk.boot(with_monitor=True)
        time.sleep(2)

        windows = dk.spawn_and_wait("foot", timeout=8)
        if not windows:
            print("FAIL: no test window mapped")
            return 1

        shell_pid = dk.shell_proc.pid
        before = child_zombies(shell_pid)
        dk.dispatch("window-menu")
        time.sleep(1.0)
        leaked = [z for z in child_zombies(shell_pid) if z not in before]
        if leaked:
            print(f"FAIL: role child was not reaped: {leaked}")
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: short-lived role child was reaped")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
