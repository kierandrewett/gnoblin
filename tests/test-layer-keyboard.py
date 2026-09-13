#!/usr/bin/env python3
"""Real keyboard delivery and application activation in a private compositor.

Run with GNOBLIN_TEST_UNSAFE_MODE=1 through run-gnome-shell.sh. Repeat with
GNOBLIN_CONFIG pointing to a file setting preserve-active-window=false and
EXPECT_PRESERVE_ACTIVE_WINDOW=0 to check the compatibility mode.
"""

import ast
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
qs = os.environ.get("QS_TEST_BIN", "/home/kieran/.local/bin/gnoblin-quickshell")
expected = os.environ.get("EXPECT_PRESERVE_ACTIVE_WINDOW", "1") == "1"


def evaluate(code):
    result = subprocess.check_output(
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
    ok, value = ast.literal_eval(result.replace("(true,", "(True,", 1).replace("(false,", "(False,", 1))
    assert ok, value
    return json.loads(value)


def wait(predicate):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            if predicate():
                return
        except (subprocess.CalledProcessError, ValueError):
            pass
        time.sleep(0.05)
    raise AssertionError("Timed out waiting for focus/input state")


with tempfile.TemporaryDirectory(prefix="layer-keyboard-") as directory:
    evaluate("""(() => {
        global.layerTestKeyboard = global.stage.context.get_backend()
            .get_default_seat().create_virtual_device(1);
        return true;
    })()""")
    path = Path(directory) / "shell.qml"
    path.write_text("""
import QtQuick
import QtQuick.Window
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
ShellRoot {
    FloatingWindow {
        visible: true; title: "Layer Keyboard App"; implicitWidth: 300; implicitHeight: 200
        TextInput { id: app; focus: true }
    }
    PanelWindow {
        id: panel; visible: false; implicitWidth: 250; implicitHeight: 100
        WlrLayershell.namespace: "arbitrary-third-party-layer"
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
        TextInput { id: input; focus: true }
    }
    PanelWindow {
        id: menu; visible: false; implicitWidth: 150; implicitHeight: 80
        WlrLayershell.namespace: "gnoblin-shell-popup"
        WlrLayershell.layer: WlrLayer.Overlay
        WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
        TextInput { id: menuInput; focus: true }
    }
    IpcHandler {
        target: "probe"
        function open(): void { input.text = ""; panel.visible = true; Qt.callLater(() => input.forceActiveFocus()); }
        function hide(): void { panel.visible = false; }
        function openMenu(): void { menu.visible = true; Qt.callLater(() => menuInput.forceActiveFocus()); }
        function closeMenu(): void { menu.visible = false; }
        function status(): string { return JSON.stringify({app: app.text, layer: input.text, menu: menuInput.text}); }
    }
}
""")
    log = open(Path(directory) / "qml.log", "w+")
    process = subprocess.Popen([qs, "-p", str(path)], stdout=log, stderr=log)

    def ipc(method):
        return subprocess.check_output(
            [qs, "ipc", "--pid", str(process.pid), "call", "probe", method], text=True, stderr=subprocess.DEVNULL
        )

    def focus():
        return evaluate("global.display.focus_window?.title ?? null")

    def type_a():
        evaluate("""(() => {
            const seat = global.stage.context.get_backend().get_default_seat();
            global.layerTestKeyboard ??= seat.create_virtual_device(1);
            global.layerTestKeyboard.notify_keyval(imports.gi.GLib.get_monotonic_time(), 97, 1);
            global.layerTestKeyboard.notify_keyval(imports.gi.GLib.get_monotonic_time(), 97, 0);
            return true;
        })()""")

    try:
        wait(lambda: focus() == "Layer Keyboard App")
        for _ in range(3):
            ipc("open")
            time.sleep(0.25)
            assert (focus() == "Layer Keyboard App") == expected, focus()
            type_a()
            wait(lambda: json.loads(ipc("status"))["layer"] == "a")
            assert json.loads(ipc("status"))["app"] == ""
            ipc("hide")
            wait(lambda: focus() == "Layer Keyboard App")
        type_a()
        wait(lambda: json.loads(ipc("status"))["app"] == "a")
        ipc("open")
        time.sleep(0.2)
        ipc("openMenu")
        time.sleep(0.2)
        assert (focus() == "Layer Keyboard App") == expected, focus()
        type_a()
        wait(lambda: json.loads(ipc("status"))["menu"] == "a")
        assert json.loads(ipc("status"))["layer"] == ""
        ipc("closeMenu")
        time.sleep(0.2)
        if expected:
            type_a()
            wait(lambda: json.loads(ipc("status"))["layer"] == "a")
        ipc("hide")
        wait(lambda: focus() == "Layer Keyboard App")
        print(
            "PASS: arbitrary layer receives real typing; active application preservation =",
            expected,
            "; repeated close/reopen, context menu input, and input return pass",
        )
        search_path = os.environ.get("BINGUX_SEARCH_TEST_PATH")
        if search_path:
            search = subprocess.Popen([qs, "-p", search_path], stdout=log, stderr=log)

            def search_ipc(method):
                return subprocess.check_output(
                    [qs, "ipc", "--pid", str(search.pid), "call", "search", method],
                    text=True,
                    stderr=subprocess.DEVNULL,
                )

            try:
                wait(lambda: bool(search_ipc("status")))
                search_ipc("open")
                wait(lambda: json.loads(search_ipc("status"))["acceptingKeyboard"])
                assert (focus() == "Layer Keyboard App") == expected, focus()
                type_a()
                wait(lambda: json.loads(search_ipc("status"))["query"] == "a")
                search_ipc("close")
                wait(lambda: not json.loads(search_ipc("status"))["visible"])
                search_ipc("open")
                wait(lambda: json.loads(search_ipc("status"))["acceptingKeyboard"])
                search.kill()
                search.wait(timeout=5)
                wait(lambda: focus() == "Layer Keyboard App")
                type_a()
                wait(lambda: json.loads(ipc("status"))["app"] == "aa")
                print(
                    "PASS: real Bingux Search receives typing, preserves activation policy, and releases input after process death"
                )
            finally:
                if search.poll() is None:
                    if sys.exc_info()[0]:
                        print("Search failure state:", search_ipc("status"), ipc("status"))
                    search.terminate()
                    search.wait(timeout=5)
    finally:
        process.terminate()
        process.wait(timeout=5)
        log.seek(0)
        if sys.exc_info()[0]:
            print(log.read())
        log.close()
