#!/usr/bin/env python3
"""Private compositor regression for global shortcuts during pointer window drags."""

import ast
import json
import os
import subprocess
import time

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")


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


def key(symbol, down):
    evaluate(
        f"(() => {{global.dragKeys.notify_keyval(imports.gi.GLib.get_monotonic_time(),{symbol},{int(down)});return true;}})()"
    )
    time.sleep(0.06)


def pointer(x, y, button=None):
    button_code = (
        ""
        if button is None
        else f"global.dragPointer.notify_button(imports.gi.GLib.get_monotonic_time(),1,{int(button)});"
    )
    evaluate(
        f"(() => {{global.dragPointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(),{x},{y});{button_code}return true;}})()"
    )
    time.sleep(0.12)


app = subprocess.Popen(
    [
        "python3",
        "-c",
        """import gi
gi.require_version('Gtk','3.0')
from gi.repository import Gtk
w=Gtk.Window(title='Drag shortcut fixture');w.set_default_size(500,300)
h=Gtk.HeaderBar(title='Drag shortcut fixture');h.set_show_close_button(True);w.set_titlebar(h)
w.show_all();Gtk.main()
""",
    ]
)
try:
    time.sleep(1)
    evaluate("""(() => {
        global.dragKeys = global.stage.context.get_backend().get_default_seat().create_virtual_device(1);
        global.dragPointer = global.stage.context.get_backend().get_default_seat().create_virtual_device(0);
        global.dragShortcutCount = 0; global.dragActive = false;
        const action = global.display.grab_accelerator('<Alt>s', 0);
        imports.gi.Meta.external_binding_name_for_action(action);
        global.dragTestAction = action;
        global.display.connect('accelerator-activated', (_d,a) => {if (a===action)global.dragShortcutCount++;});
        global.display.connect('grab-op-begin', () => global.dragActive=true);
        global.display.connect('grab-op-end', () => global.dragActive=false);
        return true;
    })()""")
    # The shell action-mode filter must permit this ordinary global binding.
    evaluate(
        "import('resource:///org/gnome/shell/ui/main.js').then(Main => Main.wm.allowKeybinding(imports.gi.Meta.external_binding_name_for_action(global.dragTestAction), imports.gi.Shell.ActionMode.NORMAL)); true"
    )
    time.sleep(0.2)
    frame = evaluate(
        "(() => {const w=global.get_window_actors().find(a=>a.meta_window.title==='Drag shortcut fixture').meta_window;w.move_frame(false,200,200);const r=w.get_frame_rect();return {x:r.x,y:r.y,width:r.width,height:r.height};})()"
    )
    time.sleep(0.3)
    pointer(frame["x"] + 100, frame["y"] + 12)
    pointer(frame["x"] + 100, frame["y"] + 12, True)
    evaluate(
        "(() => {const w=global.get_window_actors().find(a=>a.meta_window.title==='Drag shortcut fixture').meta_window;return w.begin_grab_op(imports.gi.Meta.GrabOp.MOVING,null,global.get_current_time(),null);})()"
    )
    pointer(frame["x"] + 180, frame["y"] + 70)
    assert evaluate("global.dragActive"), "pointer did not start a window drag"
    key(65513, True)
    key(115, True)
    key(115, False)
    key(65513, False)
    assert evaluate("global.dragShortcutCount") == 1, "Alt+S was suppressed during the drag"
    assert evaluate("global.dragActive"), "non-modal shortcut ended the drag"
    key(65307, True)
    key(65307, False)
    assert not evaluate("global.dragActive"), "Escape did not cancel the drag"
    pointer(frame["x"] + 180, frame["y"] + 70, False)
    print("PASS: global Alt+S activates during drag; drag remains active and Escape cancels it")
finally:
    app.terminate()
    app.wait(timeout=5)
