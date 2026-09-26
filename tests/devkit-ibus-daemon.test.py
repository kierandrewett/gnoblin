#!/usr/bin/env python3
"""Check the IBus daemon activates on the private Gnoblin test bus."""

from __future__ import annotations

import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
SERVICE_NAME = "org.freedesktop.IBus"
OBJECT_PATH = "/org/freedesktop/IBus"


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

        default_dir = root / "default"
        subprocess.run(
            [sys.executable, str(ROOT / "scripts/devkit_dbus.py"), str(default_dir), str(ROOT)],
            env=env,
            check=True,
            capture_output=True,
            text=True,
        )
        service_file = default_dir / "dbus-services" / f"{SERVICE_NAME}.service"
        if service_file.exists():
            raise RuntimeError("the default devkit D-Bus config unexpectedly exposes IBus")

        config = subprocess.check_output(
            [
                sys.executable,
                str(ROOT / "scripts/devkit_dbus.py"),
                str(root / "with-ibus"),
                str(ROOT),
                "--ibus-daemon",
            ],
            env=env,
            text=True,
        ).strip()
        result = subprocess.run(
            [
                "dbus-run-session",
                f"--config-file={config}",
                "--",
                "gdbus",
                "introspect",
                "--session",
                f"--dest={SERVICE_NAME}",
                f"--object-path={OBJECT_PATH}",
            ],
            env=env,
            capture_output=True,
            text=True,
            timeout=20,
        )
        if result.returncode != 0 or "node " not in result.stdout:
            raise RuntimeError(f"IBus daemon activation failed: {result.stderr.strip()}")
        print("IBus daemon D-Bus activation succeeded")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
