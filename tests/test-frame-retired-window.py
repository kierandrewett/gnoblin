#!/usr/bin/env python3
"""Race native window snapshots against Wayland window destruction."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[1]
GNOBLINCTL = Path(os.environ.get("GNOBLINCTL") or shutil.which("gnoblinctl") or ROOT / "build/ninja/gnoblinctl")
assert os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"), "Run inside a supervised Gnoblin session"

root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
qml = root / "retired.qml"
qml.write_text(
    """import QtQuick
import Quickshell
ShellRoot {
    FloatingWindow {
        visible: true
        title: "Retired frame query"
        implicitWidth: 320
        implicitHeight: 240
        color: "white"
    }
}
"""
)


def windows():
    result = subprocess.run(
        [str(GNOBLINCTL), "--json", "window", "list"],
        check=True,
        capture_output=True,
        text=True,
        timeout=5,
    )
    return json.loads(result.stdout)["windows"]


def target_window():
    return next((window for window in windows() if window["title"] == "Retired frame query"), None)


with (root / "retired-client.log").open("w") as log:
    client = subprocess.Popen([os.environ.get("QS_TEST_BIN", "qs"), "-p", str(qml)], stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and target_window() is None:
            time.sleep(0.03)
        assert target_window(), "Wayland test window did not appear in native snapshots"

        for _ in range(5):
            windows()
        client.terminate()
        client.wait(timeout=5)

        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and target_window() is not None:
            windows()
            time.sleep(0.02)
        assert target_window() is None, "Destroyed window remained in native snapshots"
        assert (
            subprocess.run(
                [str(GNOBLINCTL), "ping"], check=True, capture_output=True, text=True, timeout=5
            ).stdout.strip()
            == "pong"
        )
        print("PASS: native window snapshots tolerate Wayland window retirement")
    finally:
        if client.poll() is None:
            client.terminate()
            client.wait(timeout=5)
