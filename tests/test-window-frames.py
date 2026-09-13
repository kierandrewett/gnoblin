#!/usr/bin/env python3
"""Native/external frame configure, pixels and input; private compositor only."""

import json
import os
from pathlib import Path
import subprocess
import signal
import time
from PIL import Image

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
repo = Path(__file__).resolve().parents[1]
if os.environ.get("GNOBLIN_SSD_RENDERER", "native") != "native":
    unauthorized = subprocess.run(
        [str(repo / "build/frame-renderers/gnoblin-frame-cairo")],
        env={k: v for k, v in os.environ.items() if k != "WAYLAND_SOCKET"},
        capture_output=True,
        text=True,
        timeout=5,
    )
    assert unauthorized.returncode == 2 and "No private frame renderer capability" in unauthorized.stderr, unauthorized
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
root.mkdir(parents=True, exist_ok=True)
qml = root / "frames.qml"
qml.write_text("""import QtQuick
import Quickshell
ShellRoot { FloatingWindow { id: fixture; visible: true; title: "SSD fixture";
 implicitWidth: 320; implicitHeight: 240; color: "#ff8800"
 Rectangle { width: parent.width; height: 20; color: "red" }
 MouseArea { anchors.fill: parent; onClicked: mouse => { fixture.color = "#0088ff"; console.warn("SSD_CLICK", mouse.x, mouse.y) } }
 } }
""")
negotiated = os.environ.get("GNOBLIN_SSD_NEGOTIATED") == "1"
gtk = os.environ.get("GNOBLIN_SSD_RENDERER") == "bingux"
top = 36
content_y = top + 24  # Below the fixture's retained 20px strip in negotiated SSD.
custom = os.environ.get("GNOBLIN_SSD_RENDERER", "native") != "native"
theme = Path(os.environ.get("GNOBLIN_SSD_THEME", str(root / "theme.txt")))
if custom:
    theme.write_text("#204080\n")
client_env = {**os.environ, "WAYLAND_DEBUG": "1"}
if not negotiated:
    client_env["QT_WAYLAND_DISABLE_WINDOWDECORATION"] = "1"
else:
    client_env.pop("QT_WAYLAND_DISABLE_WINDOWDECORATION", None)
with (root / "frames-client.log").open("w") as log:
    client = subprocess.Popen(["qs", "-p", str(qml)], stdout=log, stderr=log, env=client_env)
    try:
        time.sleep(1)
        scripts = root / "scripts"
        scripts.mkdir(exist_ok=True)
        (scripts / "frames.js").write_text((repo / "tests/window-frames-native.js").read_text())
        (scripts / "input.js").write_text((repo / "tests/window-frames-input.js").read_text())
        subprocess.run(
            [
                "gdbus",
                "call",
                "--session",
                "--dest",
                "org.gnoblin.Shell",
                "--object-path",
                "/org/gnoblin/Shell",
                "--method",
                "org.gnoblin.Shell.Reload",
            ],
            check=True,
        )
        result = root / "frames-results.json"
        deadline = time.monotonic() + 12
        while not result.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        assert result.exists(), "no frame results"
        data = json.loads(result.read_text())
        assert "error" not in data, data
        assert len(data["records"]) == 3, data
        first = data["records"][0]
        assert first["layout"]["crop"] == ([0, 0, 0, 0] if negotiated else [20, 0, 0, 0]), data
        assert first["layout"]["mode"] == (2 if negotiated else 1), data
        assert first["header"][1] == (-top if negotiated else 20 - top), data
        assert first["title"] == "SSD fixture", data
        time.sleep(0.3)
        screenshot = root / "ssd-screen.png"
        subprocess.run(["grim", str(screenshot)], check=True)
        image = Image.open(screenshot).convert("RGB")
        image.save("/tmp/gnoblin-native-frame-screen.png")
        x, y, width, height = first["frame"]
        for point in [
            (x + width // 2, y),
            (x, y + height // 2),
            (x + width - 1, y + height // 2),
            (x + width // 2, y + height - 1),
        ]:
            assert image.getpixel(point) == (80, 80, 80), ("SSD outer border missing", point, image.getpixel(point))
        if negotiated:
            assert image.getpixel((x + width // 2, y + top)) == (255, 0, 0), "border crossed client/titlebar seam"
        header_color = image.getpixel((x + 4, y + 4)) if gtk else ((32, 64, 128) if custom else (36, 36, 36))
        assert header_color != (255, 136, 0), "header missing"
        window_image = Image.open(root / "ssd-window.png").convert("RGB")
        assert window_image.size == (width, height), "window capture omitted frame extents"
        assert window_image.getpixel((4, 4)) == header_color, (
            "window capture omitted frame pixels",
            window_image.getpixel((4, 4)),
        )
        assert image.getpixel((x + 4, y + 4)) == header_color, (
            "titlebar not actually painted",
            image.getpixel((x + 4, y + 4)),
        )
        assert image.getpixel((x + 8, y + content_y)) == (255, 136, 0), "client crop content incorrect"

        def send(command):
            path = Path(os.environ["XDG_CONFIG_HOME"]) / "snap-input.json"
            temporary = path.with_suffix(".tmp")
            temporary.write_text(json.dumps(command))
            temporary.replace(path)
            time.sleep(0.12)
            assert not path.exists(), "input command not consumed"

        send({"op": "move", "x": x + 30, "y": y + content_y})
        send({"op": "move", "x": x + 30, "y": y + content_y})
        send({"op": "button", "down": True})
        send({"op": "button", "down": False})
        log.flush()
        text = (root / "frames-client.log").read_text()
        expected_y = content_y - top + (0 if negotiated else 20)
        assert f"SSD_CLICK 28 {expected_y}" in text, text[-3000:]
        subprocess.run(["grim", str(screenshot)], check=True)
        assert Image.open(screenshot).convert("RGB").getpixel((x + 8, y + content_y)) == (0, 136, 255), (
            "client click had no visual effect"
        )

        def click(px, py):
            send({"op": "move", "x": px, "y": py})
            send({"op": "button", "down": True})
            send({"op": "button", "down": False})
            time.sleep(0.3)

        def inspect():
            send({"op": "inspect"})
            return json.loads((Path(os.environ["XDG_CONFIG_HOME"]) / "snap-result.json").read_text())["window"]

        def button(action):
            current = inspect()
            region = next(r for r in current["layout"]["presentation"]["regions"] if r[0] == action)
            return (current["frame"][0] + region[1] + region[3] // 2, current["frame"][1] + region[2] + region[4] // 2)

        if not custom or gtk:
            bx, by = button(3)
            subprocess.run(["grim", str(screenshot)], check=True)
            idle = Image.open(screenshot).convert("RGB").getpixel((bx - 6, by + 8))
            send({"op": "move", "x": bx, "y": by})
            time.sleep(0.25)
            subprocess.run(["grim", str(screenshot)], check=True)
            hovered = Image.open(screenshot).convert("RGB")
            hovered.save("/tmp/bingux-gtk-buttons.png" if gtk else "/tmp/gnoblin-native-buttons.png")
            assert hovered.getpixel((bx - 6, by + 8)) != idle, ("button hover missing", idle)
            if not custom:
                assert hovered.getpixel((bx - 16, by - 16)) == header_color, (
                    "transparent icon pixels punched a hole in the titlebar"
                )
            send({"op": "button", "down": True})
            subprocess.run(["grim", str(screenshot)], check=True)
            assert Image.open(screenshot).convert("RGB").getpixel((bx - 6, by + 8)) != hovered.getpixel(
                (bx - 6, by + 8)
            ), "pressed feedback missing"
            send({"op": "move", "x": x + 30, "y": y + 60})
            send({"op": "button", "down": False})
        if custom and not gtk:
            theme.write_text("#802040\n")
            time.sleep(0.5)
            subprocess.run(["grim", str(screenshot)], check=True)
            assert Image.open(screenshot).convert("RGB").getpixel((x + 4, y + 4)) == (128, 32, 64), (
                "custom renderer did not hot reload"
            )
            theme.write_text("invalid color !!!")
            time.sleep(0.3)
            subprocess.run(["grim", str(screenshot)], check=True)
            assert Image.open(screenshot).convert("RGB").getpixel((x + 4, y + 4)) == (128, 32, 64), (
                "invalid renderer discarded previous frame"
            )
            print("PASS: external renderer pixels, hot reload and invalid-edit retention")
        click(*button(3))
        state = inspect()
        assert state["maximized"], f"maximize button did not work: {state}"
        mx, my, mw, mh = state["frame"]
        click(*button(3))
        assert not inspect()["maximized"], "restore button did not work"
        if custom:
            # This PID belongs to the renderer provisioned by this private compositor.
            pid = inspect()["layout"]["presentation"]["pid"]
            assert Path(f"/proc/{pid}/exe").resolve() == (
                repo.parent / "bingux/build/bingux-frame"
                if gtk
                else repo / "build/frame-renderers" / ("gnoblin-frame-" + os.environ["GNOBLIN_SSD_RENDERER"])
            )
            os.kill(pid, signal.SIGSTOP)
            try:
                click(*button(3))
                stalled = inspect()
                assert stalled["maximized"] and not stalled["layout"]["presentation"]["external"], stalled
                sx, sy, sw, sh = stalled["frame"]
                subprocess.run(["grim", str(screenshot)], check=True)
                assert Image.open(screenshot).convert("RGB").getpixel((sx + 4, sy + 4)) == (36, 36, 36), (
                    "hung renderer lost native fallback"
                )
            finally:
                os.kill(pid, signal.SIGCONT)
            time.sleep(0.4)
            click(*button(3))
            assert not inspect()["maximized"]
            os.kill(pid, signal.SIGKILL)
            deadline = time.monotonic() + 2
            while True:
                fallen = inspect()
                if not fallen["layout"]["presentation"]["external"]:
                    break
                assert time.monotonic() < deadline, fallen
            subprocess.run(["grim", str(screenshot)], check=True)
            fx, fy, fw, fh = fallen["frame"]
            assert Image.open(screenshot).convert("RGB").getpixel((fx + 4, fy + 4)) == (36, 36, 36), (
                "crash lost native frame",
                Image.open(screenshot).convert("RGB").getpixel((fx + 4, fy + 4)),
                fallen,
            )
            deadline = time.monotonic() + 4
            while time.monotonic() < deadline:
                recovered = inspect()["layout"]["presentation"]
                if recovered["external"] and recovered["pid"] != pid:
                    break
            else:
                raise AssertionError("renderer did not restart")
            print("PASS: private capability restriction, hung-renderer native controls, crash fallback and restart")
            before = inspect()
            pid = before["layout"]["presentation"]["pid"]
            # Persist the fixture's previously direct rule in a private config,
            # so a real config reload does not legitimately turn its SSD off.
            reload_config = root / "reload.lua"
            original = Path(os.environ["GNOBLIN_CONFIG"]).read_text()
            reload_config.write_text(
                "local config = (function()\n"
                + original
                + "\nend)()\n"
                + 'config["window-rules"] = {{match={title="SSD fixture"}, frame={'
                + f'mode="{"prefer-server" if negotiated else "replace"}", renderer="{os.environ["GNOBLIN_SSD_RENDERER"]}",'
                + f"crop={{20,0,0,0}}, extents={{{top},2,2,2}}"
                + "}}}\nreturn config\n"
            )
            send({"op": "config-path", "path": str(reload_config)})
            subprocess.run([str(repo / "src/tools/gnoblinctl"), "config", "reload"], check=True)

            def await_renderer(previous_pid):
                deadline = time.monotonic() + 5
                while time.monotonic() < deadline:
                    current = inspect()
                    presentation = current["layout"]["presentation"]
                    if presentation["external"] and presentation["pid"] != previous_pid:
                        assert current["frame"] == before["frame"], "renderer reload changed geometry"
                        return presentation["pid"]
                raise AssertionError("live renderer replacement did not present")

            pid = await_renderer(pid)
            # A watched edit uses the same reload path, including unchanged argv.
            reload_config.write_text(reload_config.read_text() + "\n-- watched SSD reload\n")
            pid = await_renderer(pid)
            if gtk:
                send(
                    {
                        "op": "renderers",
                        "services": {
                            "bingux": [
                                str(repo.parent / "bingux/build/bingux-frame"),
                                "--button-layout=:close,maximize,minimize",
                            ]
                        },
                    }
                )
                pid = await_renderer(pid)
            send({"op": "renderers", "services": {"broken": ["relative/path"]}})
            assert json.loads((Path(os.environ["XDG_CONFIG_HOME"]) / "snap-result.json").read_text()).get("error")
            assert inspect()["layout"]["presentation"]["pid"] == pid, "invalid registry lost running renderer"
            send({"op": "renderers", "services": {}})
            assert not inspect()["layout"]["presentation"]["external"], "removed service remained attached"
            time.sleep(1)
            assert inspect()["layout"]["presentation"]["pid"] == 0, "retired service restarted"
            subprocess.run([str(repo / "src/tools/gnoblinctl"), "config", "reload"], check=True)
            await_renderer(pid)
            print("PASS: config reload and watched edits restart SSD, invalid registry retention, removal and re-add")
        if os.environ.get("GNOBLIN_TEST_WINDOW_MENU") == "1":
            if not custom:
                reload_config = root / "native-menu.lua"
                reload_config.write_text(
                    'local config = {["window-rules"] = {{match={title="SSD fixture"}, '
                    'frame={mode="prefer-server", renderer="native", extents={36,2,2,2}}, '
                    'borders={["inner-width"]=1,["inner-color"]="#505050ff",["outer-width"]=1}}}}\nreturn config\n'
                )
                send({"op": "config-path", "path": str(reload_config)})
                subprocess.run([str(repo / "src/tools/gnoblinctl"), "config", "reload"], check=True)
            from window_menu_check import check

            check(send, inspect, root, reload_config, repo)
        # Move by the titlebar, using the compositor grab rather than client CSD.
        send({"op": "move", "x": x + 50, "y": y + 18})
        send({"op": "button", "down": True})
        send({"op": "move", "x": x + 100, "y": y + 58})
        send({"op": "button", "down": False})
        time.sleep(0.3)
        moved = inspect()["frame"]
        assert moved[:2] == [x + 50, y + 40], f"SSD drag failed: {moved}"
        edge = moved[0] + moved[2] - 1
        send({"op": "move", "x": edge, "y": moved[1] + 100})
        send({"op": "button", "down": True})
        send({"op": "move", "x": edge + 64, "y": moved[1] + 100})
        send({"op": "button", "down": False})
        time.sleep(0.3)
        resized = inspect()["frame"]
        assert resized[2] == moved[2] + 64 and resized[3] == moved[3], ("SSD resize failed", moved, resized)
        moved = resized
        click(*button(4))
        assert inspect()["minimized"], "SSD minimize button failed"
        send({"op": "unminimize"})
        time.sleep(0.4)
        click(*button(2))
        assert inspect() is None, "SSD close button did not close window"
        if negotiated:
            assert "zxdg_toplevel_decoration_v1" in text, "no protocol negotiation"
        print("PASS: committed crop, visible SSD titlebar, fullscreen removal and restoration", data)
    finally:
        client.terminate()
        client.wait(timeout=5)
