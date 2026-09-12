#!/usr/bin/env python3
"""Run only inside run-gnome-shell.sh's isolated session."""

import json
import os
from pathlib import Path
import subprocess
import time

root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
(root / "scripts").mkdir(parents=True, exist_ok=True)
report = root / "layer-report.json"
config = root / "init.lua"
probe = root / "scripts/layer-probe.js"
probe.write_text(
    """
import Meta from 'gi://Meta';
import GLib from 'gi://GLib';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
export default function () {
    if (Main.wm._layerTestInstalled) return;
    Main.wm._layerTestInstalled = true;
    const original = Main.wm._shouldAnimateActor;
    Main.wm._shouldAnimateActor = function (actor, types) {
        const anchor = Meta.gnoblin_layer_anchor(actor.meta_window);
        if (anchor >= 0 && !actor._layerTestInstalled) {
            actor._layerTestInstalled = true;
            const ease = actor.ease;
            actor.ease = function (params) {
                GLib.file_set_contents(REPORT, JSON.stringify({anchor,
                    x: this.translation_x, y: this.translation_y,
                    duration: params.duration, mode: params.mode,
                    endX: params.translation_x, endY: params.translation_y}));
                return ease.call(this, params);
            };
        }
        return original.call(this, actor, types);
    };
}
""".replace("REPORT", json.dumps(str(report)))
)
subprocess.run(["gnoblinctl", "script", "reload"], check=True)
qml = root / "layer.qml"
qml.write_text("""import Quickshell
import Quickshell.Wayland
PanelWindow {
 implicitWidth: 200; implicitHeight: 80; color: "#333333"
 anchors.bottom: true; anchors.right: true
 WlrLayershell.layer: WlrLayer.Top
 WlrLayershell.namespace: "gnoblin-animation-test"
}
""")
for duration in [180, 70]:
    config.write_text(f"""local g = require("gnoblin")
g.set({{shell = {{
    ["layer-animation"] = "slide",
    ["layer-duration"] = {duration},
    ["layer-easing"] = "ease-out-cubic",
}}}})
""")
    time.sleep(0.4)
    report.unlink(missing_ok=True)
    proc = subprocess.Popen(["qs", "-p", str(qml)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 5
        while not report.exists() and time.monotonic() < deadline:
            time.sleep(0.05)
        data = json.loads(report.read_text())
        assert data["anchor"] == 10, data
        assert data["x"] > 0 and data["y"] > 0, data
        assert data["endX"] == 0 and data["endY"] == 0, data
        assert data["duration"] == duration, data
        time.sleep(0.3)
        assert proc.poll() is None, "client disconnected during animation"
        print(f"PASS: corner layer slides from bottom-right with live duration {duration}")
    finally:
        proc.terminate()
        proc.wait(timeout=5)
