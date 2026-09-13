#!/usr/bin/env python3
"""Compare incremental window movement against a forced full repaint."""

import ast
import json
import os
from pathlib import Path
import subprocess
import time
from PIL import Image, ImageChops

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
root = Path(os.environ["XDG_CONFIG_HOME"])
probe = root / "frame-probe"
subprocess.run([str(Path(__file__).resolve().parents[1] / "scripts/build-frame-probe.sh"), str(probe)], check=True)
qs = os.environ.get("QS_TEST_BIN", "qs")


def evaluate(code):
    r = subprocess.check_output(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnome.Shell",
            "--object-path",
            "/org/gnome/Shell",
            "--method",
            "org.gnome.Shell.Eval",
            code,
        ],
        text=True,
    )
    ok, value = ast.literal_eval(r.replace("(true,", "(True,", 1).replace("(false,", "(False,", 1))
    assert ok, value
    return json.loads(value)


qml = root / "move-damage.qml"
qml.write_text("""import QtQuick
import Quickshell
import Quickshell.Wayland
ShellRoot {
 PanelWindow {anchors {top:true;bottom:true;left:true;right:true} color:"#708090";WlrLayershell.layer:WlrLayer.Background}
 FloatingWindow {visible:true;title:"Move damage fixture";implicitWidth:400;implicitHeight:280;color:"transparent"
 Rectangle {anchors.fill:parent;anchors.margins:20;color:"#303030";radius:8}}
}
""")
process = subprocess.Popen(
    [qs, "-p", str(qml)],
    env={**os.environ, "QT_WAYLAND_DISABLE_WINDOWDECORATION": "1"},
    stdout=(root / "move-client.log").open("w"),
    stderr=subprocess.STDOUT,
)
try:
    deadline = time.monotonic() + 6
    while time.monotonic() < deadline:
        if evaluate("global.get_window_actors().some(a=>a.meta_window.title==='Move damage fixture')"):
            break
        time.sleep(0.1)
    evaluate(
        "global.damageWindow=global.get_window_actors().find(a=>a.meta_window.title==='Move damage fixture').meta_window; true"
    )
    evaluate(
        "global.damageApplyCount=0; import('resource:///org/gnome/shell/ui/main.js').then(Main=>{const rules=Main.componentManager._allComponents.gnoblinControl._windowRules;const apply=rules._apply;rules._apply=function(actor){if(actor.meta_window===global.damageWindow)global.damageApplyCount++;return apply.call(this,actor);};});true"
    )
    time.sleep(0.2)
    evaluate("global.damageApplyCount=0;true")
    evaluate(
        "imports.gi.GIRepository.Repository.dup_default().prepend_search_path("
        + json.dumps(str(probe))
        + ");imports.gi.GIRepository.Repository.dup_default().prepend_library_path("
        + json.dumps(str(probe))
        + ");global.frameProbe=imports.gi.FrameProbe;true"
    )
    before = root / "incremental.png"
    after = root / "full.png"
    for i in range(20):
        if i == 19:
            evaluate("global.frameProbe.arm(global.stage," + json.dumps(str(before)) + ");true")
        evaluate(f"global.damageWindow.move_frame(false,{100 + i * 22},{100 + i * 8}); true")
        time.sleep(0.03)
    time.sleep(0.2)
    assert before.exists(), "No rendered frame captured"
    evaluate("global.frameProbe.arm(global.stage," + json.dumps(str(after)) + ");true")
    evaluate(
        "imports.gi.Clutter.add_debug_flags(0,imports.gi.Clutter.DrawDebugFlag.DISABLE_CLIPPED_REDRAWS,0);global.stage.queue_redraw();true"
    )
    time.sleep(0.2)
    assert after.exists(), "No full frame captured"
    a, b = Image.open(before).convert("RGB"), Image.open(after).convert("RGB")
    diff = ImageChops.difference(a, b)
    count = sum(max(p) > 5 for p in diff.getdata())
    a.save("/tmp/gnoblin-move-incremental.png")
    b.save("/tmp/gnoblin-move-full.png")
    diff.save("/tmp/gnoblin-move-difference.png")
    print("Window rule applications during 20 moves:", evaluate("global.damageApplyCount"))
    assert evaluate("global.damageApplyCount") <= 1, "pure movement rebuilt window effects"
    assert count < 20, ("incremental movement left stale pixels", count, diff.getbbox())
    print("PASS: moved window matches a full repaint;", count, "changed pixels")
finally:
    evaluate("global.frameProbe?.stop();true")
    process.terminate()
    process.wait(timeout=5)
    print((root / "move-client.log").read_text())
