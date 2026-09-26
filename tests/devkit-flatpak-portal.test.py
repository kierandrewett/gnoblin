#!/usr/bin/env python3
"""Check Flatpak's session portal activates on Gnoblin's isolated D-Bus."""

from __future__ import annotations

import os
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
SERVICE_NAME = "org.freedesktop.portal.Flatpak"
OBJECT_PATH = "/org/freedesktop/portal/Flatpak"


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="gnoblin-flatpak-portal-") as temporary:
        root = pathlib.Path(temporary)
        dbus_dir = root / "dbus"
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
        if (default_dir / "dbus-services" / f"{SERVICE_NAME}.service").exists():
            raise RuntimeError("the default devkit D-Bus config unexpectedly exposes Flatpak's portal")

        config = subprocess.check_output(
            [sys.executable, str(ROOT / "scripts/devkit_dbus.py"), str(dbus_dir), str(ROOT), "--flatpak-portal"],
            env=env,
            text=True,
        ).strip()
        result = subprocess.run(
            [
                "dbus-run-session",
                f"--config-file={config}",
                "--",
                "gdbus",
                "call",
                "--session",
                f"--dest={SERVICE_NAME}",
                f"--object-path={OBJECT_PATH}",
                "--method=org.freedesktop.DBus.Properties.Get",
                SERVICE_NAME,
                "version",
            ],
            env=env,
            capture_output=True,
            text=True,
            timeout=20,
        )
        if result.returncode != 0:
            raise RuntimeError(f"Flatpak portal activation failed: {result.stderr.strip()}")
        match = re.search(r"<uint32 (\d+)>", result.stdout)
        if not match or int(match.group(1)) < 1:
            raise RuntimeError(f"Flatpak portal returned an invalid version: {result.stdout.strip()}")
        print(f"Flatpak portal D-Bus activation returned version {match.group(1)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
