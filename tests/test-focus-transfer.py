#!/usr/bin/env python3
"""Check app activation against real windows in a private Gnoblin session."""

import ast
import json
import os
from pathlib import Path
import subprocess
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
root = Path(__file__).resolve().parent.parent
config = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin/scripts"
config.mkdir(parents=True, exist_ok=True)
(config / "focus-transfer-test.js").write_text("""
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Clutter from 'gi://Clutter';
export default function(api) {
 if (GLib.getenv("EXPECT_FOCUS_TRANSFER") === "0") GLib.setenv("GNOME_SHELL_SESSION_MODE", "gnome", true);
 const keyboard = global.stage.context.get_backend().get_default_seat().create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
 const window = title => global.display.list_all_windows().find(w => w.title === title);
 const impl = Gio.DBusExportedObject.wrapJSObject(`<node><interface name="org.gnoblin.FocusTransferTest">
 <method name="Activate"><arg type="s" direction="in"/><arg type="u" direction="in"/></method>
 <method name="Prepare"><arg type="s" direction="in"/></method>
 <method name="Type"/>
 <method name="State"><arg type="s" direction="out"/></method>
 </interface></node>`, {
 Activate(title, timestamp) { window(title).activate(timestamp); },
 Prepare(title) {
  const target = window(title);
  const manager = global.workspace_manager;
  if (manager.n_workspaces < 2) manager.append_new_workspace(false, global.get_current_time());
  target.change_workspace_by_index(1, false); target.minimize();
 },
 Type() {
  for (const code of [Clutter.KEY_f, Clutter.KEY_o, Clutter.KEY_c, Clutter.KEY_u, Clutter.KEY_s, Clutter.KEY_Return]) {
   keyboard.notify_keyval(GLib.get_monotonic_time(), code, Clutter.KeyState.PRESSED);
   keyboard.notify_keyval(GLib.get_monotonic_time(), code, Clutter.KeyState.RELEASED);
  }
 },
 State() { return JSON.stringify({focus: global.display.focus_window?.title ?? null,
  windows: global.display.list_all_windows().map(w => ({title:w.title, minimized:w.minimized, workspace:w.get_workspace()?.index()}))}); },
 });
 impl.export(Gio.DBus.session, '/org/gnoblin/FocusTransferTest');
 const name = Gio.bus_own_name(Gio.BusType.SESSION,'org.gnoblin.FocusTransferTest',Gio.BusNameOwnerFlags.NONE,null,null,null);
 api._disposers.push(() => { impl.unexport(); Gio.bus_unown_name(name); keyboard.run_dispose(); });
}
""")
subprocess.run([str(root / "src/tools/gnoblinctl"), "script", "reload"], check=True)


def call(method, *args):
    result = subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnoblin.FocusTransferTest",
            "--object-path",
            "/org/gnoblin/FocusTransferTest",
            "--method",
            "org.gnoblin.FocusTransferTest." + method,
            *map(str, args),
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    return ast.literal_eval(result.stdout)


def state():
    return json.loads(call("State")[0])


def wait(predicate):
    end = time.monotonic() + 4
    while time.monotonic() < end:
        current = state()
        if predicate(current):
            return current
        time.sleep(0.05)
    raise AssertionError(current)


apps = []
try:
    for title in ["Focus Target", "Focus Other"]:
        destination = config / (title.replace(" ", "-") + ".txt")
        apps.append(
            subprocess.Popen(
                [
                    "foot",
                    "--title=" + title,
                    "sh",
                    "-c",
                    'read line; printf "%s" "$line" > "$1"; sleep 30',
                    "sh",
                    str(destination),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        )
        wait(lambda s: s["focus"] == title)
    call("Prepare", "Focus Target")
    call("Activate", "Focus Other", 0)
    wait(lambda s: s["focus"] == "Focus Other")
    call("Activate", "Focus Target", 1)
    expected = os.environ.get("EXPECT_FOCUS_TRANSFER", "1") == "1"
    if expected:
        wait(lambda s: s["focus"] == "Focus Target")
        call("Type")
        end = time.monotonic() + 3
        result = config / "Focus-Target.txt"
        while time.monotonic() < end and not result.exists():
            time.sleep(0.05)
        assert result.read_text() == "focus", "Target must receive keyboard input"
        assert not next(w for w in state()["windows"] if w["title"] == "Focus Target")["minimized"]
    else:
        time.sleep(0.3)
        assert state()["focus"] == "Focus Other", "GNOME focus prevention must remain intact"
    # Exercise xdg_activation_v1 itself, including a token without an input serial.
    protocol_dir = subprocess.check_output(
        ["pkg-config", "--variable=pkgdatadir", "wayland-protocols"], text=True
    ).strip()
    protocol = str(Path(protocol_dir) / "staging/xdg-activation/xdg-activation-v1.xml")
    subprocess.run(
        ["wayland-scanner", "client-header", protocol, str(config / "xdg-activation-v1-client-protocol.h")], check=True
    )
    subprocess.run(["wayland-scanner", "private-code", protocol, str(config / "activation-protocol.c")], check=True)
    flags = subprocess.check_output(
        ["pkg-config", "--cflags", "--libs", "gtk+-3.0", "wayland-client"], text=True
    ).split()
    binary = config / "activation-client"
    subprocess.run(
        [
            "cc",
            str(root / "tests/focus-transfer-client.c"),
            str(config / "activation-protocol.c"),
            "-I" + str(config),
            "-o",
            str(binary),
            *flags,
        ],
        check=True,
    )
    request = config / "activation-request"
    received = config / "activation-input"
    apps.append(
        subprocess.Popen([str(binary), str(request), str(received)], env=os.environ | {"GDK_BACKEND": "wayland"})
    )
    wait(lambda s: any(w["title"] == "Wayland Activation Target" for w in s["windows"]))
    call("Activate", "Focus Other", 0)
    wait(lambda s: s["focus"] == "Focus Other")
    request.touch()
    if expected:
        wait(lambda s: s["focus"] == "Wayland Activation Target")
        call("Type")
        end = time.monotonic() + 3
        while time.monotonic() < end and not received.exists():
            time.sleep(0.05)
        assert received.read_text() == "focused", "Wayland activation must deliver keyboard input"
    else:
        time.sleep(0.4)
        assert state()["focus"] == "Focus Other", "GNOME must reject a token without user input"
    print("PASS: native Wayland activation token policy and keyboard delivery")
    print(
        "PASS: "
        + (
            "stale app activation restores, changes workspace, and receives typing"
            if expected
            else "GNOME retains its focus prevention"
        )
    )
finally:
    for app in apps:
        app.terminate()
        app.wait()
