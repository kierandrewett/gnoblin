#!/usr/bin/env python3
"""Check IBus stays owned by the private Gnoblin test session bus."""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
SERVICE_NAME = "org.freedesktop.IBus"


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="gnoblin-ibus-daemon-") as temporary:
        root = pathlib.Path(temporary)
        runtime_dir = root / "run"
        runtime_dir.mkdir(mode=0o700)
        env = os.environ.copy()
        env.pop("DBUS_SESSION_BUS_ADDRESS", None)
        env.update(
            {
                "HOME": str(root / "home"),
                "XDG_CACHE_HOME": str(root / "cache"),
                "XDG_CONFIG_HOME": str(root / "config"),
                "XDG_DATA_HOME": str(root / "data"),
                "XDG_RUNTIME_DIR": str(runtime_dir),
                "XDG_STATE_HOME": str(root / "state"),
                "GNOBLIN_PREFIX": str(root / "no-prefix"),
            }
        )
        for key in ("HOME", "XDG_CACHE_HOME", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_STATE_HOME"):
            pathlib.Path(env[key]).mkdir()

        dbus_dir = root / "dbus"
        config = subprocess.check_output(
            [sys.executable, str(ROOT / "scripts/devkit_dbus.py"), str(dbus_dir), str(ROOT)],
            env=env,
            text=True,
        ).strip()
        service_file = dbus_dir / "dbus-services" / f"{SERVICE_NAME}.service"
        if service_file.exists():
            raise RuntimeError("the private test bus must not transiently activate IBus")

        helper = ROOT / "scripts/gnoblin-test-ibus.sh"
        pid_file = root / "ibus.pid"
        log_file = root / "ibus.log"
        duplicate_log = root / "duplicate.log"
        script = f"""
source {str(helper)!r}
gnoblin_test_ibus_start {str(pid_file)!r} {str(log_file)!r}
cleanup() {{ gnoblin_test_ibus_stop {str(pid_file)!r}; }}
trap cleanup EXIT
owner_before=$(gdbus call --session --dest=org.freedesktop.DBus \\
    --object-path=/org/freedesktop/DBus \\
    --method=org.freedesktop.DBus.GetNameOwner {SERVICE_NAME})
ibus-daemon --panel disable >{str(duplicate_log)!r} 2>&1 &
duplicate_pid=$!
for _ in $(seq 1 50); do
    kill -0 "$duplicate_pid" 2>/dev/null || break
    sleep 0.1
done
if kill -0 "$duplicate_pid" 2>/dev/null; then
    echo "duplicate IBus daemon did not exit" >&2
    kill "$duplicate_pid" 2>/dev/null || true
    wait "$duplicate_pid" 2>/dev/null || true
    exit 1
fi
wait "$duplicate_pid" 2>/dev/null || true
sleep 1
owner_after=$(gdbus call --session --dest=org.freedesktop.DBus \\
    --object-path=/org/freedesktop/DBus \\
    --method=org.freedesktop.DBus.GetNameOwner {SERVICE_NAME})
if [[ "$owner_before" != "$owner_after" ]]; then
    echo "IBus owner changed after duplicate daemon startup: $owner_before -> $owner_after" >&2
    exit 1
fi
echo "IBus daemon retained owner $owner_after across duplicate startup"
"""
        result = subprocess.run(
            [
                "dbus-run-session",
                f"--config-file={config}",
                "--",
                "bash",
                "-c",
                script,
            ],
            env=env,
            capture_output=True,
            text=True,
            timeout=20,
        )
        if result.returncode != 0 or "retained owner" not in result.stdout:
            raise RuntimeError(f"private IBus startup failed: {result.stderr.strip()}\n{result.stdout}")
        print(result.stdout.strip())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
