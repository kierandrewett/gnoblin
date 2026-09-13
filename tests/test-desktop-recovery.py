#!/usr/bin/env python3
"""Drive desktop recovery with real pointer events in a private compositor."""

import ast
import json
import os
from pathlib import Path
import subprocess
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")


def evaluate(code):
    output = subprocess.check_output(
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
        timeout=5,
    )
    ok, value = ast.literal_eval(output.replace("(true,", "(True,", 1).replace("(false,", "(False,", 1))
    assert ok, value
    return json.loads(value)


def wait_for(code, seconds=12):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = evaluate(code)
        if result:
            return result
        time.sleep(0.1)
    print(
        "Windows:",
        evaluate("global.get_window_actors().map(a => ({title: a.meta_window?.title, mapped: a.is_mapped()}))"),
    )
    raise AssertionError(f"Timed out: {code}")


def click(x, y, button):
    evaluate(f"""(() => {{
        global.recoveryPointer ??= global.stage.context.get_backend().get_default_seat()
            .create_virtual_device(imports.gi.Clutter.InputDeviceType.POINTER_DEVICE);
        global.recoveryPointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(), {x}, {y});
        return true;
    }})()""")
    time.sleep(0.1)
    evaluate(f"""(() => {{
        global.recoveryPointer.notify_button(imports.gi.GLib.get_monotonic_time(), {button}, 1);
        global.recoveryPointer.notify_button(imports.gi.GLib.get_monotonic_time(), {button}, 0);
        return true;
    }})()""")


def click_actor(expression):
    time.sleep(0.4)
    x, y = evaluate(f"""(() => {{
        const actor = {expression};
        const [x, y] = actor.get_transformed_position();
        const [w, h] = actor.get_transformed_size();
        return [x + w / 2, y + h / 2];
    }})()""")
    click(x, y, 1)


evaluate("import('resource:///org/gnome/shell/ui/main.js').then(m => { global.recoveryMain = m; return true; })")
main = "global.recoveryMain"
background = f"{main}.layoutManager._bgManagers[0].backgroundActor"
menu = f"{background}._backgroundMenu"
# Use an actual terminal with a unique title and private Wayland connection.
evaluate(
    "new imports.gi.Gio.Settings({schema_id: 'org.gnome.desktop.default-applications.terminal'}).set_string('exec', 'foot --title=Gnoblin-Recovery-Test')"
)
assert evaluate(f"Boolean({menu})"), "desktop recovery menu is missing"
wait_for(f"!{main}.layoutManager._startingUp")
click(60, 60, 3)
wait_for(f"{menu}.isOpen")
click_actor(f"{menu}._getMenuItems()[0]")
terminal = "global.get_window_actors().find(a => a.meta_window?.title === 'Gnoblin-Recovery-Test')"
wait_for(f"Boolean({terminal})")
evaluate(f"(() => {{ {terminal}.meta_window.delete(global.get_current_time()); return true; }})()")
wait_for(f"!({terminal})")
print("PASS: desktop right-click launches a real terminal with external shell stopped")

panel = f"{main}.layoutManager.uiGroup.get_children().find(a => a.name === 'gnoblin-recovery')"
wait_for(f"Boolean({panel}?.visible)")


def recovery_button(name):
    return f"(() => {{ const find = actor => actor.name === 'gnoblin-recovery-{name}' ? actor : actor.get_children().map(find).find(Boolean); return find({panel}); }})()"


if screenshot := os.environ.get("GNOBLIN_RECOVERY_SCREENSHOT"):
    subprocess.run(["grim", screenshot], check=True)
click_actor(recovery_button("terminal"))
wait_for(f"Boolean({terminal})")
evaluate(f"(() => {{ {terminal}.meta_window.delete(global.get_current_time()); return true; }})()")
print("PASS: automatic recovery panel launches a real terminal")
click_actor(recovery_button("console"))
wait_for(f"Boolean({main}.devConsole?.isOpen)")
evaluate(f"(() => {{ {main}.devConsole.close(true); return true; }})()")
print("PASS: recovery opens the developer console")
# The private bus deliberately omits desktop application activation services.
# Start Files on that bus so the click can use the real application interface.
with open(Path(os.environ["XDG_CONFIG_HOME"]) / "files.log", "w") as log:
    files_service = subprocess.Popen(["nautilus", "--gapplication-service"], stdout=log, stderr=log)
    try:
        subprocess.run(["gdbus", "wait", "--session", "--timeout", "8", "org.gnome.Nautilus"], check=True)
        click_actor(recovery_button("config"))
        files = "global.get_window_actors().find(a => a.meta_window?.get_gtk_application_id() === 'org.gnome.Nautilus')"
        wait_for(f"Boolean({files})")
        evaluate(f"(() => {{ {files}.meta_window.delete(global.get_current_time()); return true; }})()")
    finally:
        files_service.terminate()
        files_service.wait(timeout=5)
print("PASS: recovery opens the config folder in Files")

# A real layer client must dismiss the panel; its exit must restore it.
qml = Path(os.environ["XDG_CONFIG_HOME"]) / "recovery-layer.qml"
qml.write_text("""import Quickshell
import Quickshell.Wayland
ShellRoot {
    PanelWindow { visible: true; implicitWidth: 200; implicitHeight: 40
        WlrLayershell.namespace: "recovery-test"
    }
}
""")
qs = os.environ["QS_TEST_BIN"]
with open(qml.with_suffix(".log"), "w+") as log:
    process = subprocess.Popen([qs, "-p", str(qml)], stdout=log, stderr=log)
    try:
        wait_for(f"!({panel}?.visible)", 8)
        assert process.poll() is None
    finally:
        process.terminate()
        process.wait(timeout=5)
    wait_for(f"Boolean({panel}?.visible)")
    click_actor(recovery_button("dismiss"))
    wait_for(f"!({panel}?.visible)")
    time.sleep(4)
    assert evaluate(f"!({panel}?.visible)"), "dismissal did not persist"
print("PASS: layer mapping hides recovery, client exit restores it, dismissal persists")
evaluate(f"(() => {{ {main}.sessionMode.pushMode('unlock-dialog'); return true; }})()")
wait_for(f"!({panel})")
evaluate(f"(() => {{ {main}.sessionMode.popMode('unlock-dialog'); return true; }})()")
wait_for(f"Boolean({panel}?.visible)")
assert evaluate(f"{main}.layoutManager.uiGroup.get_children().filter(a => a.name === 'gnoblin-recovery').length") == 1
print("PASS: lock lifecycle removes recovery and unlock creates only one panel")

if shell_path := os.environ.get("BINGUX_SHELL_TEST_PATH"):
    with open(qml.with_name("bingux.log"), "w+") as log:
        process = subprocess.Popen([qs, "-p", shell_path, "--no-color"], stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if process.poll() is not None:
                    log.seek(0)
                    raise AssertionError(log.read())
                status = subprocess.run(
                    [qs, "ipc", "--pid", str(process.pid), "call", "shell", "status"],
                    capture_output=True,
                    text=True,
                    timeout=3,
                )
                if status.returncode == 0:
                    break
                time.sleep(0.2)
            assert status.returncode == 0, status.stderr
            print("Bingux state:", status.stdout.strip())
            layers = "global.get_window_actors().filter(a => a.is_mapped() && a.meta_window && imports.gi.Meta.gnoblin_layer_anchor(a.meta_window) >= 0).length"
            assert wait_for(layers) >= 1
            subprocess.run(
                [qs, "ipc", "--pid", str(process.pid), "call", "shell", "panel", "calendar", "open"], check=True
            )
            time.sleep(0.5)
            output = subprocess.check_output(
                [qs, "ipc", "--pid", str(process.pid), "call", "shell", "status"], text=True
            )
            assert json.loads(output)["calendar"]
            print("PASS: full Bingux starts, maps layer surfaces and opens its calendar")
            subprocess.run([qs, "ipc", "--pid", str(process.pid), "call", "sidebar", "select", "terminal"], check=True)
            subprocess.run([qs, "ipc", "--pid", str(process.pid), "call", "sidebar", "open"], check=True)
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline:
                output = subprocess.check_output(
                    [qs, "ipc", "--pid", str(process.pid), "call", "sidebar", "status"], text=True
                )
                sidebar = json.loads(output)
                if sidebar["ready"] and sidebar["running"]:
                    break
                time.sleep(0.2)
            assert sidebar["ready"] and sidebar["running"] and sidebar["pid"] > 0, sidebar
            print("PASS: native terminal sidebar starts a real shell")
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=5)
            log.seek(0)
            text = log.read()
            print(text[-5000:])
