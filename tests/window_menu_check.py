"""End-to-end native SSD -> configured command -> Bingux window menu -> action."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


def check(send, inspect, root, config, repo):
    bingux = repo.parent / "bingux"
    fixture = root / "menu-ui"
    fixture.mkdir()
    for source in (bingux / "shell/bingux").iterdir():
        if source.suffix in (".qml", ".js") or source.name == "qmldir":
            shutil.copy2(source, fixture)
    shutil.copy2(bingux / "tests/window-menu.qml", fixture / "shell.qml")
    scripts = root / "scripts"
    shutil.copy2(repo / "src/scripts/compositor-bridge.js", scripts)
    shutil.copytree(repo / "src/scripts/lib", scripts / "lib", dirs_exist_ok=True)
    (scripts / "frames.js").unlink()  # One-shot fixture setup must not run twice.
    subprocess.run(["gdbus", "call", "--session", "--dest", "org.gnoblin.Shell",
        "--object-path", "/org/gnoblin/Shell", "--method", "org.gnoblin.Shell.Reload"], check=True)
    env = os.environ | {"PATH": str(repo / "src/tools") + ":" + os.environ["PATH"]}
    with (root / "menu-ui.log").open("w") as log:
        ui = subprocess.Popen(["qs", "-p", str(fixture), "--no-color"], env=env, stdout=log, stderr=log)
        try:
            command = ["qs", "ipc", "-p", str(fixture), "call", "menu", "open"]
            text = config.read_text().replace("return config", 'config.shell = {["window-menu"] = {' +
                ",".join(json.dumps(arg) for arg in command) + '}}\nreturn config')
            config.write_text(text)
            time.sleep(1)
            assert ui.poll() is None, (root / "menu-ui.log").read_text()
            x, y, w, h = inspect()["frame"]
            send({"op": "move", "x": x + 60, "y": y + 18})
            send({"op": "button", "button": 3, "down": True})
            send({"op": "button", "button": 3, "down": False})
            opened_at = time.monotonic()
            def status():
                result = subprocess.run(["qs", "ipc", "-p", str(fixture), "call", "menu", "state"],
                    capture_output=True, text=True, timeout=3)
                assert result.returncode == 0, result.stderr
                return json.loads(result.stdout)
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                state = status()
                if state["visible"]: break
                time.sleep(.1)
            assert state["visible"], (state, (root / "menu-ui.log").read_text())
            print(f"WINDOW_MENU_OPEN_MS: {(time.monotonic() - opened_at) * 1000:.1f}", flush=True)
            assert "Minimize" in state["actions"] and "Close" in state["actions"], state
            # The popup is opened at the titlebar click point. A primary click
            # on that frame edge must dismiss it instead of being swallowed by
            # the popup's transparent card input area.
            # Do not move between the secondary release and this primary
            # click: the stationary-pointer case is the regression.
            send({"op": "button", "down": True})
            send({"op": "button", "down": False})
            time.sleep(.4)
            closed_state = status()
            assert not closed_state["visible"], "left-clicking the titlebar edge did not dismiss the menu"
            assert closed_state["retained"], "window menu discarded its cached content after closing"
            send({"op": "move", "x": x + 60, "y": y + 18})
            send({"op": "button", "button": 3, "down": True})
            send({"op": "button", "button": 3, "down": False})
            reopened_at = time.monotonic()
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                state = status()
                if state["visible"]:
                    break
                time.sleep(.1)
            assert state["visible"], "menu did not reopen after edge dismissal"
            print(f"WINDOW_MENU_REOPEN_MS: {(time.monotonic() - reopened_at) * 1000:.1f}", flush=True)
            assert state["origin"] == [0, 0], f"window menu reveal origin is not top-left: {state}"
            time.sleep(.4)
            subprocess.run(["grim", "/tmp/bingux-window-menu.png"], check=True)
            send({"op": "move", "x": state["x"] + 80, "y": state["y"] + 22})
            send({"op": "button", "down": True})
            send({"op": "button", "down": False})
            time.sleep(.5)
            assert inspect()["minimized"], (state, (root / "menu-ui.log").read_text())
            assert not status()["visible"], "menu did not dismiss after action"
            send({"op": "unminimize"})
            time.sleep(.4)
            subprocess.run([str(repo / "src/tools/gnoblinctl"), "window", "menu", state["window"]], check=True)
            time.sleep(.4)
            assert status()["visible"], "CLI window-menu trigger did not open popup"
            send({"op": "key", "code": 1, "down": True})
            send({"op": "key", "code": 1, "down": False})
            assert not status()["visible"], "Escape did not dismiss menu"
            print("PASS: real SSD right click opens Bingux window menu and click minimizes original window")
        finally:
            ui.terminate()
            ui.wait(timeout=5)
