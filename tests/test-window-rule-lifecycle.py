#!/usr/bin/env python3
"""Exercise lock-style controller teardown with late script cleanup in a private shell."""

import json
import os
from pathlib import Path
import subprocess
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
repo = Path(__file__).resolve().parents[1]
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
qml = root / "lifecycle.qml"
qml.write_text("""import QtQuick
import Quickshell
ShellRoot { FloatingWindow { visible: true; title: "Lifecycle fixture";
 implicitWidth: 320; implicitHeight: 240; color: "white" } }
""")
with (root / "lifecycle-client.log").open("w") as log:
    client = subprocess.Popen(
        ["qs", "-p", str(qml)], stdout=log, stderr=log, env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"}
    )
    try:
        time.sleep(1)
        scripts = root / "scripts"
        scripts.mkdir(exist_ok=True)
        (scripts / "lifecycle.js").write_text((repo / "tests/window-rule-lifecycle-native.js").read_text())
        subprocess.run(
            [
                "gdbus",
                "call",
                "--session",
                "--dest",
                "org.gnoblin.Shell",
                "--object-path",
                "/org/gnoblin/Shell",
                "--method",
                "org.gnoblin.Shell.Reload",
            ],
            check=True,
        )
        result = root / "lifecycle-results.json"
        deadline = time.monotonic() + 12
        records = []
        while time.monotonic() < deadline:
            if result.exists():
                records = json.loads(result.read_text())
            if len(records) == 4:
                break
            time.sleep(0.1)
        assert len(records) == 4, records
        assert all(r == {"decorations": 2, "surfaceIsDecoration": False, "cornerEffects": 1} for r in records), records
        print("PASS: four controller generations retain exactly one border, shadow and corner effect")
    finally:
        client.terminate()
        client.wait(timeout=5)
