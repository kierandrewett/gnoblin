#!/usr/bin/env python3
"""Native snapping input test.

Run via GNOBLIN_TEST_DBUS_CLIENT=$PWD/tests/test-window-snapping.py
with scripts/run-gnome-shell.sh. Set GNOBLIN_QS to a matching Quickshell
wrapper, BINGUX_SOURCE to its checkout, and SNAP_TEST_BACKEND=x11 to test X11.
The harness monitor must be its default 1280x800.
"""

import shutil
import json
import os
import socket
import subprocess
import time
from pathlib import Path

repo = Path(__file__).resolve().parents[1]
config = Path(os.environ["XDG_CONFIG_HOME"])
if not str(config).startswith("/tmp/gnoblin-gs."):
    raise SystemExit("Run only as GNOBLIN_TEST_DBUS_CLIENT inside scripts/run-gnome-shell.sh")

scripts = config / "gnoblin/scripts"
(scripts / "lib").mkdir(parents=True, exist_ok=True)
shutil.copy(repo / "src/scripts/compositor-bridge.js", scripts)
for source in (repo / "src/scripts/lib").glob("*"):
    if source.is_file():
        shutil.copy(source, scripts / "lib")
shutil.copy(repo / "tests/window-snapping-input.js", scripts / "snap-input.js")
subprocess.run([str(repo / "src/tools/gnoblinctl"), "script", "reload"], check=True)
root = Path(os.environ.get("BINGUX_SOURCE", str(repo.parent / "bingux"))) / "shell/bingux"
qs_command = os.environ.get("GNOBLIN_QS", "qs")
fixture = config / "snap-fixture"
fixture.mkdir(exist_ok=True)
for name in [
    "SnapAssist.qml",
    "SnapLayouts.js",
    "import-snap-layouts.py",
    "ShortcutSession.qml",
    "ShellPopup.qml",
    "PanelOutline.qml",
    "PopupShadow.qml",
    "PopupTransitions.qml",
    "Theme.qml",
    "CompositorEnvironment.qml",
]:
    (fixture / name).write_bytes((root / name).read_bytes())
# Test-only access to the rendered preview, without production debug handlers.
snap_source = fixture / "SnapAssist.qml"
snap_source.write_text(
    snap_source.read_text().replace(
        "    id: root",
        """    id: root
    IpcHandler {
        target: "snap-motion-test"
        function pose(x: int, y: int, active: bool): void {
            root.update({active: active, serial: 99999, x: x, y: y, modifiers: 0,
                monitor: {id: 0, x: 0, y: 0, width: 1280, height: 800},
                area: {x: 0, y: 32, width: 1280, height: 768}});
        }
        function sample(): string {
            return JSON.stringify({x: highlight.x, y: highlight.y,
                width: highlight.width, opacity: highlight.opacity, mapped: preview.visible});
        }
    }""",
        1,
    )
)
(fixture / "qmldir").write_text(
    "singleton Theme 1.0 Theme.qml\nsingleton PopupTransitions 1.0 PopupTransitions.qml\nsingleton CompositorEnvironment 1.0 CompositorEnvironment.qml\nSnapAssist 1.0 SnapAssist.qml\nShellPopup 1.0 ShellPopup.qml\nPanelOutline 1.0 PanelOutline.qml\nPopupShadow 1.0 PopupShadow.qml\nShortcutSession 1.0 ShortcutSession.qml\n"
)
app_source = fixture / "app.py"
app_source.write_text(
    "import gi\ngi.require_version('Gtk', '3.0')\nfrom gi.repository import Gtk\nw=Gtk.Window(title='Snap test app');w.set_default_size(600,400)\nh=Gtk.HeaderBar(title='Snap test app');h.set_show_close_button(True);w.set_titlebar(h)\nw.add(Gtk.Label(label='Window snapping integration test'))\nw.connect('destroy',Gtk.main_quit);w.show_all();Gtk.main()\n"
)
(fixture / "shell.qml").write_text("""import Quickshell
import Quickshell.Wayland
ShellRoot {
 PanelWindow {
  anchors { top: true; left: true; right: true }
  implicitHeight: 32
  exclusiveZone: 32
  color: "#102030"
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "snap-test-topbar"
 }
 SnapAssist {}
}
""")
log = open("/tmp/snap-integration-qml.log", "w")
qs = subprocess.Popen([qs_command, "-p", str(fixture)], stdout=log, stderr=log)
app_env = dict(os.environ, GDK_BACKEND=os.environ.get("SNAP_TEST_BACKEND", "wayland"))
if app_env["GDK_BACKEND"] == "x11":
    if os.environ.get("GNOBLIN_TEST_XWAYLAND") != "1":
        raise SystemExit("X11 checks require GNOBLIN_TEST_XWAYLAND=1")
    app_env.update(
        {key: value for key, value in json.loads((config / "snap-display.json").read_text()).items() if value}
    )
app = subprocess.Popen(["python3", str(app_source)], env=app_env)
sock = socket.socket(socket.AF_UNIX)
sock.settimeout(4)
sock.connect(os.environ.get("GNOBLIN_COMPOSITOR_SOCKET", os.environ["XDG_RUNTIME_DIR"] + "/gnoblin/compositor-v1.sock"))
reader = sock.makefile()


def command(cmd):
    sock.sendall((json.dumps(dict(op="command", id="test", **cmd)) + "\n").encode())
    while True:
        r = json.loads(reader.readline())
        if r.get("event") == "error":
            raise AssertionError(r)
        if r.get("event") == "reply":
            return r["result"]


def window():
    return next((w for w in command(dict(command="windows"))["windows"] if w["title"] == "Snap test app"))


def pointer(op, **kw):
    p = config / "snap-input.json"
    tmp = p.with_suffix(".tmp")
    tmp.write_text(json.dumps(dict(op=op, **kw)))
    tmp.replace(p)
    time.sleep(0.16)


try:
    time.sleep(1.3)
    w = window()
    wid = w["id"]
    before = w["geometry"]
    print("BEFORE", before, flush=True)
    command(dict(command="window", action="focus", window=wid))
    time.sleep(0.2)
    # Exercise the generic compositor boundary before any snap state exists.
    pointer("move", x=before["x"] + 200, y=before["y"] + 25)
    pointer("button", down=True)
    pointer("move", x=500, y=100)
    pointer("move", x=400, y=40)
    pointer("button", down=False)
    time.sleep(0.4)
    clamped = window()["geometry"]
    assert clamped["y"] >= 32 and clamped["width"] == before["width"], clamped
    # Super+drag must obey the same exclusive-zone boundary while held.
    pointer("move", x=clamped["x"] + 250, y=clamped["y"] + 150)
    pointer("key", key="Super_L", down=True)
    pointer("button", down=True)
    pointer("move", x=500, y=5)
    time.sleep(0.3)
    assert window()["geometry"]["y"] >= 32, window()
    pointer("key", key="Escape", down=True)
    pointer("key", key="Escape", down=False)
    pointer("button", down=False)
    pointer("key", key="Super_L", down=False)
    time.sleep(0.3)
    binding_config = config / "gnoblin" / "init.lua"
    binding_config.write_text("""local g = require("gnoblin")
g.set({keybindings = {wm = {
    maximize = {"<Super>Up"},
    minimize = {},
    unmaximize = {},
}}})
""")
    subprocess.run([str(repo / "src/tools/gnoblinctl"), "config", "reload"], check=True)
    command = ", ".join(
        json.dumps(part) for part in [str(repo / "src/tools/gnoblinctl"), "window", "restore-or-minimize", "active"]
    )
    with binding_config.open("a") as output:
        output.write(f"""g.set({{shortcuts = {{{{
    name = "restore-or-minimize",
    binding = "<Super>Down",
    command = {{{command}}},
}}}}}})\n""")
    subprocess.run([str(repo / "src/tools/gnoblinctl"), "config", "reload"], check=True)

    def chord(key):
        pointer("key", key="Super_L", down=True)
        pointer("key", key=key, down=True)
        pointer("key", key=key, down=False)
        pointer("key", key="Super_L", down=False)
        time.sleep(0.4)

    chord("Up")
    assert window()["maximized"]
    chord("Down")
    assert not window()["maximized"] and not window()["minimized"]
    chord("Down")
    assert window()["minimized"]
    command(dict(command="window", action="focus", window=wid))
    time.sleep(0.3)
    print("PASS: Super+drag clamp and config-driven Up/Down restore/minimise", flush=True)
    before = window()["geometry"]
    assert before["y"] >= 32, before
    pointer("move", x=before["x"] + before["width"] // 2, y=before["y"] + 15)
    pointer("move", x=before["x"] + before["width"] // 2, y=before["y"] + 15)
    pointer("button", down=True)
    pointer("move", x=600, y=200)
    pointer("move", x=600, y=50)
    time.sleep(0.5)
    subprocess.run(["grim", "/tmp/bingux-snap-picker.png"], check=True)
    pointer("move", x=375, y=80)
    time.sleep(0.3)
    subprocess.run(["grim", "/tmp/bingux-snap-region.png"], check=True)
    pointer("button", down=False)
    time.sleep(0.6)
    after = window()["geometry"]
    print("AFTER", after, flush=True)
    assert after["x"] == 8 and after["y"] == 40 and after["width"] == 628, after
    assert window()["focused"], "snap must preserve application focus"
    subprocess.run(["grim", "/tmp/bingux-snap-after.png"], check=True)
    pointer("move", x=after["x"] + after["width"] // 2, y=after["y"] + 35)
    pointer("button", down=True)
    pointer("move", x=after["x"] + after["width"] // 2 + 24, y=after["y"] + 35)
    pointer("move", x=800, y=300)
    pointer("button", down=False)
    time.sleep(0.4)
    restored = window()["geometry"]
    print("RESTORED", restored, flush=True)
    assert restored["width"] == before["width"] and restored["height"] == before["height"], restored
    pointer("key", key="Super_L", down=True)
    pointer("key", key="z", down=True)
    pointer("key", key="z", down=False)
    pointer("key", key="Super_L", down=False)
    time.sleep(0.4)
    subprocess.run(["grim", "/tmp/bingux-snap-keyboard.png"], check=True)
    pointer("key", key="Right", down=True)
    pointer("key", key="Right", down=False)
    pointer("key", key="Return", down=True)
    pointer("key", key="Return", down=False)
    time.sleep(0.5)
    keyed = window()["geometry"]
    print("KEYBOARD", keyed, flush=True)
    assert keyed["x"] == 644 and keyed["width"] == 628, keyed
    pointer("move", x=keyed["x"] + 200, y=keyed["y"] + 25)
    pointer("button", down=True)
    pointer("move", x=keyed["x"] + 220, y=keyed["y"] + 25)
    pointer("move", x=600, y=60)
    pointer("key", key="Escape", down=True)
    pointer("key", key="Escape", down=False)
    pointer("button", down=False)
    time.sleep(0.4)
    cancelled = window()["geometry"]
    print("CANCELLED", cancelled, flush=True)
    assert cancelled == keyed, cancelled
    current = window()["geometry"]
    pointer("move", x=current["x"] + 200, y=current["y"] + 35)
    pointer("button", down=True)
    pointer("move", x=current["x"] + 224, y=current["y"] + 35)
    pointer("key", key="Control_L", down=True)
    pointer("move", x=200, y=400)
    time.sleep(0.3)
    pointer("button", down=False)
    pointer("key", key="Control_L", down=False)
    time.sleep(0.5)
    region = window()["geometry"]
    print("CTRL REGION", region, flush=True)
    assert region["x"] == 8 and region["y"] == 40 and region["width"] == 628, region
    for px, py, expected in [
        (640, 0, (0, 32, 1280, 768)),
        (0, 400, (8, 40, 628, 752)),
        (1279, 400, (644, 40, 628, 752)),
        (0, 0, (8, 40, 628, 372)),
        (1279, 0, (644, 40, 628, 372)),
        (0, 799, (8, 420, 628, 372)),
        (1279, 799, (644, 420, 628, 372)),
    ]:
        current = window()["geometry"]
        pointer("move", x=current["x"] + 200, y=current["y"] + 25)
        pointer("button", down=True)
        pointer("move", x=640, y=300)
        pointer("move", x=640, y=340)
        time.sleep(0.4)
        restored_frame = window()["geometry"]
        if px == 640 and py == 0:
            pre_maximize_size = (restored_frame["width"], restored_frame["height"])
        elif px == 0 and py == 400:
            assert not window()["maximized"]
            assert (restored_frame["width"], restored_frame["height"]) == pre_maximize_size, (
                "drag restore size",
                restored_frame,
                pre_maximize_size,
            )
        pointer("move", x=px, y=py)
        subprocess.run(["grim", "/tmp/bingux-snap-edge-preview.png"], check=True)
        pointer("button", down=False)
        time.sleep(0.4)
        if px == 640 and py == 0:
            assert window()["maximized"], "Top-edge snap must enter native maximisation"
        actual = window()["geometry"]
        assert tuple(actual[k] for k in ("x", "y", "width", "height")) == expected, (px, py, actual)
    current = window()["geometry"]
    pointer("move", x=current["x"] + 200, y=current["y"] + 25)
    pointer("move", x=current["x"] + 200, y=current["y"] + 25)
    pointer("button", down=True)
    pointer("move", x=current["x"] + 224, y=current["y"] + 25)
    pointer("move", x=640, y=300)
    # Motion and release in one input batch: no UI offer round trip at the edge.
    pointer("drop", x=0, y=799)
    time.sleep(0.4)
    flung = window()["geometry"]
    assert tuple(flung[k] for k in ("x", "y", "width", "height")) == (8, 420, 628, 372), flung

    def motion(method, *args):
        result = subprocess.check_output(
            [qs_command, "-p", str(fixture), "ipc", "call", "snap-motion-test", method, *map(str, args)], text=True
        )
        return json.loads(result) if method == "sample" else None

    motion("pose", 0, 400, "true")
    time.sleep(0.3)
    assert abs(motion("sample")["x"] - 8) < 1
    motion("pose", 1279, 400, "true")
    samples = [motion("sample") for _ in range(5)]
    if os.environ.get("BINGUX_REDUCED_MOTION") != "1":
        assert any(8 < s["x"] < 644 for s in samples), ("preview jumped", samples)
        assert all(a["x"] <= b["x"] for a, b in zip(samples, samples[1:])), samples
    time.sleep(0.3)
    assert abs(motion("sample")["x"] - 644) < 1
    motion("pose", 1279, 400, "false")
    fading = motion("sample")
    if os.environ.get("BINGUX_REDUCED_MOTION") != "1":
        assert 0 < fading["opacity"] < 1 and fading["mapped"], ("preview exit cut off", fading)
    time.sleep(0.3)
    assert motion("sample")["opacity"] == 0
    print("PASS: preview interpolates between regions and remains mapped through its exit fade", flush=True)
    print(
        "PASS: picker, restore, keyboard, Escape, Ctrl, seven edge/corner targets, immediate fling release and topbar clamp",
        flush=True,
    )
finally:
    app.terminate()
    qs.terminate()
    app.wait()
    qs.wait()
    log.close()
