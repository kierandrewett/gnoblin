#!/usr/bin/env python3
"""Count layout invalidation while an exclusive-zone panel animates privately."""

import json
import os
from pathlib import Path
import subprocess
import time

import gi

gi.require_version("Gtk", "4.0")
from gi.repository import Gio, GLib, Gtk  # noqa: E402 - Select GI versions before importing their modules.

if not os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"):
    raise SystemExit("Run through scripts/run-gnome-shell.sh on its private bus")
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
pid = bus.call_sync(
    "org.freedesktop.DBus",
    "/org/freedesktop/DBus",
    "org.freedesktop.DBus",
    "GetConnectionUnixProcessID",
    GLib.Variant("(s)", ("org.gnome.Shell",)),
    None,
    Gio.DBusCallFlags.NONE,
    5000,
    None,
).unpack()[0]


def evaluate(code):
    ok, value = bus.call_sync(
        "org.gnome.Shell",
        "/org/gnome/Shell",
        "org.gnome.Shell",
        "Eval",
        GLib.Variant("(s)", (code,)),
        None,
        Gio.DBusCallFlags.NONE,
        5000,
        None,
    ).unpack()
    assert ok, "Requires GNOBLIN_TEST_UNSAFE_MODE=1 on the private compositor: " + value
    return json.loads(value)


def spin(seconds):
    deadline = time.monotonic() + seconds
    context = GLib.MainContext.default()
    while time.monotonic() < deadline:
        while context.pending():
            context.iteration(False)
        time.sleep(0.002)


def cpu_ticks():
    fields = Path(f"/proc/{pid}/stat").read_text().split(") ")[1].split()
    return int(fields[11]) + int(fields[12])


fixture = Path(os.environ["XDG_CONFIG_HOME"]) / "layer-commit-benchmark.qml"
fixture.write_text("""import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
 PanelWindow {
  anchors { top: true; left: true; right: true }
  implicitHeight: 36
  exclusiveZone: 36
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "layer-commit-benchmark"
  color: "#203040"
  Rectangle {
   width: 80; height: 36; color: "#eeeeee"
   SequentialAnimation on x {
    loops: Animation.Infinite
    NumberAnimation { from: 0; to: 1200; duration: 2500 }
    NumberAnimation { from: 1200; to: 0; duration: 2500 }
   }
  }
 }
}
""")
Gtk.init()
windows = []
for i in range(8):
    window = Gtk.Window(title=f"Layer commit fixture {i}")
    window.set_default_size(600, 400)
    window.set_child(Gtk.Label(label=f"Window {i}"))
    window.present()
    windows.append(window)
    spin(0.1)
log_path = fixture.with_suffix(".log")
with log_path.open("w") as log:
    proc = subprocess.Popen([os.environ.get("GNOBLIN_QS", "qs"), "-p", str(fixture)], stdout=log, stderr=log)
    observing = False
    try:
        spin(2)
        assert proc.poll() is None, log_path.read_text()
        evaluate("""global.__gnoblinLayerPerf = {workareas: 0, frames: 0};
global.__gnoblinLayerPerf.workareaId = global.display.connect('workareas-changed',
 () => global.__gnoblinLayerPerf.workareas++);
global.__gnoblinLayerPerf.frameId = global.stage.connect('after-paint',
 () => global.__gnoblinLayerPerf.frames++); true""")
        observing = True
        before = cpu_ticks()
        start = time.monotonic()
        spin(float(os.environ.get("GNOBLIN_BENCH_SECONDS", "5")))
        elapsed = time.monotonic() - start
        result = evaluate(
            "({workareas: global.__gnoblinLayerPerf.workareas, frames: global.__gnoblinLayerPerf.frames})"
        )
        result.update(
            cpuPercent=round((cpu_ticks() - before) / os.sysconf("SC_CLK_TCK") / elapsed * 100, 2),
            seconds=round(elapsed, 3),
        )
        print(json.dumps(result), flush=True)
        if os.environ.get("GNOBLIN_LAYER_BENCH_REPORT"):
            Path(os.environ["GNOBLIN_LAYER_BENCH_REPORT"]).write_text(json.dumps(result, indent=2) + "\n")
        assert result["frames"] > 30, "Panel animation did not render"
        if os.environ.get("GNOBLIN_LAYER_REQUIRE_STABLE_WORKAREA") == "1":
            assert result["workareas"] == 0, "Unchanged panel commits invalidated work areas"
    finally:
        try:
            if observing:
                evaluate("""global.display.disconnect(global.__gnoblinLayerPerf.workareaId);
global.stage.disconnect(global.__gnoblinLayerPerf.frameId);
delete global.__gnoblinLayerPerf; true""")
        finally:
            proc.terminate()
            proc.wait(timeout=5)
            for window in windows:
                window.destroy()
