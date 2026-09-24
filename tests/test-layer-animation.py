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
import * as Config from 'resource:///org/gnome/shell/ui/components/gnoblinConfig.js';
export default function () {
    if (Main.wm._layerTestInstalled) return;
    Main.wm._layerTestInstalled = true;
    const original = Main.wm._shouldAnimateActor;
    Main.wm._shouldAnimateActor = function (actor, types) {
        const anchor = Meta.gnoblin_layer_anchor(actor.meta_window);
        if (anchor >= 0 && !actor._layerTestInstalled) {
            actor._layerTestInstalled = true;
            let x = actor.translation_x, y = actor.translation_y, opacity = actor.opacity;
            const publish = () => {
                const config = Config.layerAnimation(Config.windowProperties(actor.meta_window), true);
                GLib.file_set_contents(REPORT, JSON.stringify({anchor, x, y, opacity,
                    duration: config.duration, animation: config.animation}));
            };
            for (const [property, read, write] of [
                ['translation_x', () => x, value => { x = value; }],
                ['translation_y', () => y, value => { y = value; }],
                ['opacity', () => opacity, value => { opacity = value; }],
            ]) {
                Object.defineProperty(actor, property, {
                    configurable: true, get: read,
                    set(value) { write(value); publish(); },
                });
            }
        }
        return original.call(this, actor, types);
    };
}
""".replace("REPORT", json.dumps(str(report)))
)
subprocess.run(["gnoblinctl", "reload"], check=True)
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
        data = None
        while time.monotonic() < deadline:
            time.sleep(0.05)
            if report.exists():
                data = json.loads(report.read_text())
                if data["x"] > 0 and data["y"] > 0:
                    break
        assert data is not None, "animation engine did not write an initial frame"
        assert data["anchor"] == 10, data
        assert data["x"] > 0 and data["y"] > 0, data
        assert data["duration"] == duration, data
        assert data["animation"] == "slide", data
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            data = json.loads(report.read_text())
            if data["x"] == 0 and data["y"] == 0 and data["opacity"] == 255:
                break
            time.sleep(0.03)
        assert data["x"] == 0 and data["y"] == 0 and data["opacity"] == 255, data
        assert proc.poll() is None, "client disconnected during animation"
        print(f"PASS: corner layer uses shared slide frames and live duration {duration}")
    finally:
        proc.terminate()
        proc.wait(timeout=5)
