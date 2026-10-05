#!/usr/bin/env python3
"""Verify repeated Lua window-rule reloads preserve a live compositor window."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import time

assert os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"), "Run inside a supervised Gnoblin session"
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
rule_file = root / "conf.d" / "lifecycle.lua"
rule_file.parent.mkdir(parents=True, exist_ok=True)
gnoblinctl = os.environ.get("GNOBLINCTL") or shutil.which("gnoblinctl") or "gnoblinctl"
title = "Lua rule lifecycle fixture"


def configure(enabled):
    if enabled:
        rule_file.write_text(
            "gnoblin.window_rule {\n"
            f'    match = {{title = "^{title}$"}},\n'
            '    corners = {radius = 14, mode = "force", shadow = false, '
            'border_width = 6, border_color = "#ff000080"},\n'
            "}\n"
        )
    else:
        rule_file.write_text("-- no window rules\n")
    result = subprocess.run([gnoblinctl, "config", "reload"], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"Lua config reload failed: {result.stdout}{result.stderr}")


qml = root / "rule-lifecycle-client.qml"
qml.write_text(
    """import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
    FloatingWindow {
        title: "Lua rule lifecycle fixture"
        implicitWidth: 320; implicitHeight: 240
        color: "white"
        Rectangle { anchors.fill: parent; color: "white"; radius: 40 }
    }
}
"""
)


def window_geometry():
    result = subprocess.run(
        [gnoblinctl, "--json", "window", "list", "--title", title],
        check=True,
        capture_output=True,
        text=True,
    )
    windows = json.loads(result.stdout)["windows"]
    frame = windows[0].get("frame", windows[0].get("geometry")) if windows else None
    return frame if frame and frame["width"] > 0 and frame["height"] > 0 else None


with (root / "rule-lifecycle-client.log").open("w") as log:
    process = subprocess.Popen(
        ["qs", "-p", str(qml)],
        stdout=log,
        stderr=log,
        env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"},
    )
    try:
        frame = None
        for _ in range(100):
            frame = window_geometry()
            if frame:
                break
            time.sleep(0.1)
        assert frame, "lifecycle fixture did not appear in the window list"

        for generation in range(4):
            configure(True)
            subprocess.run([gnoblinctl, "ping"], check=True, capture_output=True, text=True)
            frame = window_geometry()
            assert frame, f"window disappeared after Lua config reload {generation + 1}"

        configure(False)
        subprocess.run([gnoblinctl, "ping"], check=True, capture_output=True, text=True)
        assert window_geometry(), "removing a Lua window rule closed the window"
        runtime_log = Path(os.environ["GNOBLIN_DEVKIT_RUNTIME_LOG"]).read_text(errors="replace")
        assert "failed to match Gnoblin window animation rules" not in runtime_log, runtime_log
        print("PASS: four Lua window-rule reloads preserve the live compositor window")
    finally:
        process.terminate()
        process.wait(timeout=5)
