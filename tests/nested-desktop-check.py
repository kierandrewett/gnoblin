#!/usr/bin/env python3
"""Inspect/reload the private normal-config desktop, never the host bus."""

import ast
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from PIL import Image

root = Path(sys.argv[1])
assert str(root).startswith("/tmp/gnoblin-user-config.")
env = os.environ | json.loads((root / "session.json").read_text())
for key in ("DISPLAY", "WAYLAND_SOCKET", "DBUS_STARTER_ADDRESS", "DBUS_STARTER_BUS_TYPE"):
    env.pop(key, None)
env["GTK_A11Y"] = "none"
env["NO_AT_BRIDGE"] = "1"
assert env["WAYLAND_DISPLAY"].startswith("gnoblin-devkit-")


def run(command):
    return subprocess.check_output(command, env=env, text=True, timeout=20)


def evaluate(code):
    reply = run(
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
        ]
    )
    ok, value = ast.literal_eval(reply.replace("(true,", "(True,", 1).replace("(false,", "(False,", 1))
    assert ok, value
    return json.loads(value) if value else None


if __name__ == "__main__":
    if len(sys.argv) > 2 and sys.argv[2] == "--run":
        sys.exit(subprocess.call(sys.argv[3:], env=env))
    if len(sys.argv) > 2:
        print(json.dumps(evaluate(sys.argv[2]), indent=2))
        sys.exit(0)
    evaluate("import('resource:///org/gnome/shell/ui/main.js').then(m=>global.nestedMain=m);true")
    time.sleep(0.2)
    for cycle in range(4):
        if cycle:
            run(["gnoblinctl", "script", "reload"])
            run(["gnoblinctl", "config", "reload"])
            time.sleep(1)
        rows = evaluate(
            "global.get_window_actors().map(a=>{let w=a.meta_window,c=global.nestedMain.componentManager._allComponents.gnoblinControl._windowRules._actors.get(a)?.corners,f=w.get_frame_rect();return {app:w.get_wm_class(),title:w.title,frame:[f.x,f.y,f.width,f.height],layout:imports.gi.Meta.gnoblin_window_frame_get(w).recursiveUnpack(),uniforms:c?.effect?Object.fromEntries(c.effect.values):null};})"
        )
        (root / f"windows-{cycle}.json").write_text(json.dumps(rows, indent=2))
        print(
            "RELOAD",
            cycle,
            json.dumps(
                [
                    {"app": r["app"], "insets": r["uniforms"].get("csdSampleInsets") if r["uniforms"] else None}
                    for r in rows
                ]
            ),
            flush=True,
        )
        gtk = next(r for r in rows if r["title"] == "Corner fixture")
        assert gtk["uniforms"]["csdActive"] == [1], gtk
        assert max(gtk["uniforms"]["csdSampleInsets"]) > 2, gtk
        assert gtk["layout"]["mode"] != 2, "GTK client incorrectly forced to SSD"
        evaluate(
            "global.get_window_actors().find(a=>a.meta_window.title==='Corner fixture').meta_window.activate(global.get_current_time());true"
        )
        time.sleep(0.4)
        screenshot = root / f"corners-reload-{cycle}.png"
        run(["grim", str(screenshot)])
        x, y, w, h = gtk["frame"]
        image = Image.open(screenshot).convert("RGB")
        for px, py in [(x + 4, y + 4), (x + w - 5, y + 4), (x + 4, y + h - 5), (x + w - 5, y + h - 5)]:
            assert min(image.getpixel((px, py))) >= 245, (cycle, px, py, image.getpixel((px, py)))
    print("PASS: normal-config GTK CSD pixels and negotiation survive three script/config reloads")
