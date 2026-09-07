#!/usr/bin/env python3
"""Render a generated vector cursor theme inside the private test session."""
import os
import json
from pathlib import Path
import shutil
import subprocess
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"), "Use the private Gnoblin test session"
repo = Path(__file__).resolve().parent.parent
work = Path(os.environ["XDG_CONFIG_HOME"]) / "hyprcursor-test"
shape = work / "source/hyprcursors/arrow"
shape.mkdir(parents=True)
(work / "source/manifest.hl").write_text("name = GnoblinVectorTest\ndescription = Test fixture\nversion = 1\ncursors_directory = hyprcursors\n")
(shape / "meta.hl").write_text("resize_algorithm = bilinear\nhotspot_x = 0.25\nhotspot_y = 0.25\ndefine_override = default\ndefine_override = wait\ndefine_size = 24, first.svg, 35\ndefine_size = 24, second.svg, 70\n")
for filename, colour in [("first.svg", "#ff00ff"), ("second.svg", "#00ffff")]:
    (shape / filename).write_text(f'<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24"><rect width="24" height="24" fill="{colour}"/></svg>')
subprocess.run(["hyprcursor-util", "--create", str(work / "source"), "--output", str(work)], check=True)
icons = Path.home() / ".icons"
icons.mkdir(exist_ok=True)
compiled = next(work.glob("theme_*"))
shutil.copytree(compiled, icons / "GnoblinVectorTest")
subprocess.run([str(repo / "build/test-hyprcursor")], check=True)

# Exercise the real compositor loader as well as the bridge's pixel checks.
scripts = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin/scripts"
scripts.mkdir(parents=True, exist_ok=True)
report = work / "native-cursor.json"
(scripts / "hyprcursor-probe.js").write_text('''
import Clutter from "gi://Clutter";
import Gio from "gi://Gio";
import GLib from "gi://GLib";
export default function enable(api) {
    const settings = new Gio.Settings({schema_id: "org.gnome.desktop.interface"});
    settings.set_string("cursor-theme", "GnoblinVectorTest");
    settings.set_int("cursor-size", 48);
    const tracker = global.backend.get_cursor_tracker();
    const device = global.stage.context.get_backend().get_default_seat()
        .create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
    device.notify_absolute_motion(GLib.get_monotonic_time(), 200, 200);
    tracker.set_gnoblin_launch_cursor(true);
    const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 150, () => {
        const sprite = tracker.get_sprite();
        GLib.file_set_contents(REPORT, JSON.stringify({width: sprite?.get_width(),
            height: sprite?.get_height(), hotspot: tracker.get_hot(),
            active: tracker.get_gnoblin_launch_cursor()}));
        tracker.set_gnoblin_launch_cursor(false);
        return GLib.SOURCE_REMOVE;
    });
    api._disposers.push(() => {
        tracker.set_gnoblin_launch_cursor(false);
        device.run_dispose();
    });
}
'''.replace("REPORT", json.dumps(str(report))))
subprocess.run([str(repo / "src/tools/gnoblinctl"), "reload-scripts"], check=True)
deadline = time.monotonic() + 4
while not report.exists() and time.monotonic() < deadline:
    time.sleep(0.05)
actual = json.loads(report.read_text())
assert actual == {"width": 48, "height": 48, "hotspot": [12, 12], "active": True}, actual
print("NATIVE_HYPRCURSOR_PASSED: SVG theme reached Mutter's global cursor renderer")
