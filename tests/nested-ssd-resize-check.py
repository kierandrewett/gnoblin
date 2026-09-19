#!/usr/bin/env python3
"""Exercise zero-border SSD resize input in the isolated normal-config desktop."""

import atexit
from pathlib import Path
import subprocess
import runpy
import time
import sys

from PIL import Image

check = runpy.run_path(str(Path(__file__).with_name("nested-desktop-check.py")), run_name="nested_checks")
evaluate, run, root = check["evaluate"], check["run"], check["root"]
if not evaluate('global.get_window_actors().some(a=>a.meta_window.title==="SSD fixture")'):
    fixture = root / "resize-fixture.qml"
    fixture.write_text("""import QtQuick
import Quickshell
ShellRoot { FloatingWindow { visible: true; title: "SSD fixture";
    implicitWidth: 400; implicitHeight: 300; color: "#ff8800" } }
""")
    client = subprocess.Popen(
        [check["env"].get("BINGUX_QUICKSHELL", "gnoblin-quickshell"), "-p", str(fixture)],
        env=check["env"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    def stop_client():
        client.terminate()
        client.wait(timeout=5)

    atexit.register(stop_client)
    deadline = time.monotonic() + 5
    while not evaluate('global.get_window_actors().some(a=>a.meta_window.title==="SSD fixture")'):
        assert time.monotonic() < deadline, "resize fixture did not appear"
        time.sleep(0.1)
evaluate(
    'global.resizeWindow=global.get_window_actors().find(a=>a.meta_window.title==="SSD fixture").meta_window;global.resizePointer=global.stage.context.get_backend().get_default_seat().create_virtual_device(imports.gi.Clutter.InputDeviceType.POINTER_DEVICE);true'
)


def move(x, y):
    evaluate(f"global.resizePointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(),{x},{y});true")
    time.sleep(0.08)


def button(down):
    evaluate(f"global.resizePointer.notify_button(imports.gi.GLib.get_monotonic_time(),1,{int(down)});true")
    time.sleep(0.08)


def state():
    return evaluate(
        "(()=>{let w=global.resizeWindow,r=w.get_frame_rect();return {rect:[r.x,r.y,r.width,r.height],layout:imports.gi.Meta.gnoblin_window_frame_get(w).recursiveUnpack()}})()"
    )


def cursor_matches(name, px, py):
    time.sleep(0.3)
    actual_path = root / f"resize-{name}-actual.png"
    expected_path = root / f"resize-{name}-expected.png"
    run(["grim", "-c", str(actual_path)])
    evaluate(f"global.display.set_cursor(imports.gi.Meta.Cursor.{name});true")
    time.sleep(0.1)
    run(["grim", "-c", str(expected_path)])
    crop = (px - 48, py - 48, px + 48, py + 48)
    cursor_ok = Image.open(actual_path).crop(crop).tobytes() == Image.open(expected_path).crop(crop).tobytes()
    evaluate("global.display.set_cursor(imports.gi.Meta.Cursor.DEFAULT);true")
    time.sleep(0.1)
    default_path = root / f"resize-{name}-default.png"
    run(["grim", "-c", str(default_path)])
    if name != "DEFAULT":
        assert Image.open(expected_path).crop(crop).tobytes() != Image.open(default_path).crop(crop).tobytes(), (
            "cursor capture cannot distinguish resize from default"
        )
    move(px, py)
    return cursor_ok


deadline = time.monotonic() + 5
while state()["layout"]["border"] != [36, 0, 0, 0]:
    assert time.monotonic() < deadline, "normal SSD configuration did not commit"
    time.sleep(0.1)

if len(sys.argv) > 2:
    renderer = sys.argv[2]
    assert renderer in ("native", "bingux")
    evaluate(
        f'imports.gi.Meta.gnoblin_window_frame_style(global.resizeWindow, new imports.gi.GLib.Variant("a{{sv}}", {{renderer:new imports.gi.GLib.Variant("s","{renderer}")}}));true'
    )
    deadline = time.monotonic() + 5
    while state()["layout"]["presentation"]["external"] != (renderer == "bingux"):
        assert time.monotonic() < deadline, "requested renderer did not present"
        time.sleep(0.1)

assert state()["layout"]["border"] == [36, 0, 0, 0], "test requires normal zero-border SSD configuration"

external_ssd = len(sys.argv) > 2 and sys.argv[2] != "native"

if external_ssd:
    evaluate(
        "global.ssdResizeFallbackFlashes=0;"
        "global.ssdResizeFallback=global.get_window_actors()"
        '.find(a=>a.meta_window.title==="SSD fixture").get_children()'
        '.find(c=>c.get_name()==="gnoblin-native-frame").get_children()[0];'
        'global.ssdResizeFallbackSignal=global.ssdResizeFallback.connect("notify::visible",()=>{'
        "if(global.ssdResizeFallback.visible)global.ssdResizeFallbackFlashes++;});true"
    )

failures = []
for name, fx, fy, dx, dy, action in [
    ("N", 0.5, 0, 0, -24, 5),
    ("NE", 1, 0, 24, -24, 6),
    ("E", 1, 0.5, 24, 0, 7),
    ("SE", 1, 1, 24, 24, 8),
    ("S", 0.5, 1, 0, 24, 9),
    ("SW", 0, 1, -24, 24, 10),
    ("W", 0, 0.5, -24, 0, 11),
    ("NW", 0, 0, -24, -24, 12),
]:
    evaluate(
        "global.resizeWindow.activate(global.get_current_time());global.resizeWindow.move_resize_frame(false,400,250,402,340);true"
    )
    time.sleep(0.2)
    if external_ssd:
        evaluate("global.ssdResizeFallbackFlashes=0;true")
    before = state()
    x, y, w, h = before["rect"]
    outside = len(sys.argv) > 3 and sys.argv[3] == "outside"
    px = x + (w // 2 if fx == 0.5 else (-4 if fx == 0 else w + 3) if outside else round((w - 1) * fx))
    py = y + (h // 2 if fy == 0.5 else (-4 if fy == 0 else h + 3) if outside else round((h - 1) * fy))
    move(x + w // 2, y + h // 2)
    move(px, py)
    hover = state()["layout"]["presentation"]["hover"]
    cursor_ok = cursor_matches(f"{name}_RESIZE", px, py)
    button(True)
    move(px + dx, py + dy)
    button(False)
    if external_ssd:
        flashes = evaluate("global.ssdResizeFallbackFlashes")
        assert flashes == 0, f"built-in SSD became visible during external resize ({flashes} flashes)"
    after = state()["rect"]
    expected = [x + min(dx, 0), y + min(dy, 0), w + abs(dx), h + abs(dy)]
    ok = hover == action and after == expected and cursor_ok
    print(
        f"{'PASS' if ok else 'FAIL'} {name}: hover={hover}/{action}, cursor={cursor_ok}, rect={after}/{expected}",
        flush=True,
    )
    if not ok:
        failures.append(name)
assert not failures, failures

# Leaving a resize edge must restore the normal titlebar cursor.
x, y, w, h = state()["rect"]
move(x + w // 2, y + 18)
assert state()["layout"]["presentation"]["hover"] == 1
assert cursor_matches("DEFAULT", x + w // 2, y + 18), "resize cursor remained on titlebar"
print("PASS: cursor resets on titlebar", flush=True)
evaluate("global.resizeWindow.maximize();true")
time.sleep(0.4)
x, y, w, h = state()["rect"]
move(x + w // 2, y)
assert state()["layout"]["presentation"]["hover"] < 5, "maximised titlebar exposes resize"
assert cursor_matches("DEFAULT", x + w // 2, y), "maximised window has resize cursor"
evaluate("global.resizeWindow.unmaximize();true")
time.sleep(0.4)
evaluate("global.resizeWindow.make_fullscreen();true")
time.sleep(0.4)
assert not state()["layout"]["presentation"]["visible"], "fullscreen retains resize frame"
evaluate("global.resizeWindow.unmake_fullscreen();true")
time.sleep(0.4)
print("PASS: maximised and fullscreen windows do not expose resize", flush=True)
evaluate("global.resizePointer.run_dispose();true")
