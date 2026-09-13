#!/usr/bin/env python3
"""Frame queries must be safe while a destroyed Wayland window is retained."""

import os
from pathlib import Path
import subprocess
import time
import json

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
repo = Path(__file__).resolve().parents[1]
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
scripts = root / "scripts"
scripts.mkdir(parents=True, exist_ok=True)
(scripts / "retired-window.js").write_text("""import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
export default function(api) {
 const path = GLib.build_filenamev([GLib.get_user_config_dir(),'gnoblin','retired-window.json']);
 const timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 20, () => {
  const window = global.get_window_actors().map(a => a.meta_window).find(w => w.title === 'Retired frame query');
  if (!window) return GLib.SOURCE_CONTINUE;
  let duringSupported = null;
  const unmanaging = window.connect('unmanaging', () => {
   duringSupported = Meta.gnoblin_window_frame_get(window).recursiveUnpack().supported;
  });
  const signal = window.connect('unmanaged', () => {
   window.disconnect(unmanaging);
   window.disconnect(signal);
   GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
    const layout = Meta.gnoblin_window_frame_get(window).recursiveUnpack();
    GLib.file_set_contents(path, JSON.stringify({retired:true,duringSupported,supported:layout.supported}));
    return GLib.SOURCE_REMOVE;
   });
  });
  GLib.file_set_contents(path, JSON.stringify({watching:true}));
  return GLib.SOURCE_REMOVE;
 });
}
""")
subprocess.run([str(repo / "src/tools/gnoblinctl"), "script", "reload"], check=True)
qml = root / "retired.qml"
qml.write_text("""import QtQuick
import Quickshell
ShellRoot { FloatingWindow { visible:true; title:"Retired frame query";
 implicitWidth:320; implicitHeight:240; color:"white" } }
""")
result = root / "retired-window.json"


def wait_for(key):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        if result.exists():
            record = json.loads(result.read_text())
            if record.get(key):
                return record
        time.sleep(0.05)
    raise AssertionError("Missing retired-window result: " + key)


with (root / "retired-client.log").open("w") as log:
    client = subprocess.Popen([os.environ.get("QS_TEST_BIN", "qs"), "-p", str(qml)], stdout=log, stderr=log)
    try:
        wait_for("watching")
        client.terminate()
        client.wait(timeout=5)
        record = wait_for("retired")
        assert record["supported"] is False and record["duringSupported"] is False, record
        print("PASS: frame query after Wayland surface destruction returns unsupported")
    finally:
        if client.poll() is None:
            client.terminate()
            client.wait(timeout=5)
