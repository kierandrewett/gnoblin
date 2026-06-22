#!/usr/bin/env python3
# Regression: per-output autostart clients that exit because their output was
# removed must not be respawned for the dead connector.
import importlib.util
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)

PER_OUTPUT = ("gnoblin-topbar", "gnoblin-dock", "gnoblin-wallpaper")


def per_output_processes(dk):
    lines = []
    for needle in PER_OUTPUT:
        lines.extend(line for line in dk.processes(needle) if "--output Meta-0" in line)
    return sorted(lines)


def main():
    dk = dh.Devkit()
    try:
        dk.boot(with_monitor=False, per_output=True)
        if not dk.add_monitor_late(1280, 800):
            print("SKIP: virtual monitor never materialized")
            return 0

        deadline = time.time() + 8
        while time.time() < deadline:
            live = per_output_processes(dk)
            if all(any(name in line for line in live) for name in PER_OUTPUT):
                break
            time.sleep(0.25)
        else:
            print(f"FAIL: per-output clients did not start on Meta-0: {per_output_processes(dk)}")
            print(dk._tail(30))
            return 1

        for consumer in dk._consumers:
            consumer.terminate()
        for consumer in dk._consumers:
            try:
                consumer.wait(timeout=2)
            except Exception:
                consumer.kill()
        dk._consumers.clear()
        dk._sc_session.call_sync("Stop", None, dh.Gio.DBusCallFlags.NONE, -1, None)

        deadline = time.time() + 8
        while time.time() < deadline:
            if not per_output_processes(dk):
                break
            time.sleep(0.25)
        else:
            print("FAIL: per-output clients were still alive or respawned after output removal:")
            print("\n".join(per_output_processes(dk)))
            print(dk._tail(40))
            return 1

        time.sleep(2)
        log = dk.shell_log.read_text(errors="replace")
        if "--output 'Meta-0' not found" in log or "available outputs: []" in log:
            print("FAIL: compositor respawned a per-output client for removed Meta-0")
            print(dk._tail(60))
            return 1
        if dk.crashed():
            print(f"FAIL: compositor crashed: {dk.crashed()}")
            print(dk._tail())
            return 1

        print("PASS: per-output autostart clients are not respawned for removed outputs")
        return 0
    finally:
        dk.teardown()


if __name__ == "__main__":
    sys.exit(main())
