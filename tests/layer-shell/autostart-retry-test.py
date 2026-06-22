#!/usr/bin/env python3
# Regression: a failed `[startup]` spawn must not poison the autostart table.
# If the executable appears later and gnoblin.conf reloads, the compositor should
# retry it instead of treating the failed launch as already running.
import importlib.util
import pathlib
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parent.parent.parent
spec = importlib.util.spec_from_file_location("dh", ROOT / "scripts" / "devkit-harness.py")
dh = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dh)


def wait_for_marker(path, dk, timeout=6.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            return True
        if dk.crashed():
            print(f"FAIL: compositor crashed while waiting for autostart retry: {dk.crashed()}")
            print(dk._tail())
            return False
        time.sleep(0.2)
    return False


def run_case(kind):
    dk = dh.Devkit()
    client = dk.tmp / f"late-{kind}-client"
    marker = dk.tmp / f"late-{kind}-ran"
    key = "exec_per_output" if kind == "per-output" else "exec"
    try:
        dk.extra_conf = f"\n[startup]\n{key} = {client}\n"
        dk.boot(with_monitor=True)
        time.sleep(1.0)
        if dk.crashed():
            print(f"FAIL: compositor crashed before {kind} retry: {dk.crashed()}")
            print(dk._tail())
            return False

        client.write_text(
            "#!/usr/bin/env bash\n"
            f"printf '%s\\n' \"$*\" > {marker}\n"
            "sleep 30\n"
        )
        client.chmod(0o755)

        cfg = dk.tmp / "config" / "gnoblin" / "gnoblin.conf"
        cfg.write_text(cfg.read_text() + f"\n# trigger {kind} autostart retry\n")

        if not wait_for_marker(marker, dk):
            print(f"FAIL: missing {kind} autostart command was not retried after config reload")
            print(dk._tail(40))
            return False

        args = marker.read_text(errors="replace").strip()
        if kind == "per-output" and "--output Meta-0" not in args:
            print(f"FAIL: per-output retry did not receive output binding args: {args!r}")
            return False

        return True
    finally:
        dk.teardown()


def main():
    if not run_case("global"):
        return 1
    if not run_case("per-output"):
        return 1
    print("PASS: failed autostart entries are retried after config reload")
    return 0


if __name__ == "__main__":
    sys.exit(main())
