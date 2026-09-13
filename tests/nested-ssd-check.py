#!/usr/bin/env python3
"""Real input and presentation checks in the normal-config nested desktop."""

import json
from pathlib import Path
import runpy
import time
from PIL import Image

check = runpy.run_path(str(Path(__file__).with_name("nested-desktop-check.py")), run_name="nested_checks")
evaluate, run, root = check["evaluate"], check["run"], check["root"]
evaluate(
    "global.nestedPointer ??= global.stage.context.get_backend().get_default_seat().create_virtual_device(imports.gi.Clutter.InputDeviceType.POINTER_DEVICE);global.nestedKeys ??= global.stage.context.get_backend().get_default_seat().create_virtual_device(imports.gi.Clutter.InputDeviceType.KEYBOARD_DEVICE);true"
)


def input_event(kind, **data):
    if kind == "move":
        evaluate(
            f"global.nestedPointer.notify_absolute_motion(imports.gi.GLib.get_monotonic_time(),{data['x']},{data['y']});true"
        )
    elif kind == "button":
        evaluate(
            f"global.nestedPointer.notify_button(imports.gi.GLib.get_monotonic_time(),{data.get('button', 1)},{1 if data['down'] else 0});true"
        )
    elif kind == "key":
        evaluate(
            f"global.nestedKeys.notify_key(imports.gi.GLib.get_monotonic_time(),{data['code']},{1 if data['down'] else 0});true"
        )
    time.sleep(0.06)


def inspect():
    return evaluate(
        '(()=>{let w=global.get_window_actors().find(a=>a.meta_window.title==="SSD fixture").meta_window,f=w.get_frame_rect();return {frame:[f.x,f.y,f.width,f.height],minimized:w.minimized,maximized:w.maximized_horizontally&&w.maximized_vertically,layout:imports.gi.Meta.gnoblin_window_frame_get(w).recursiveUnpack()};})()'
    )


if __name__ == "__main__":
    evaluate(
        'global.get_window_actors().find(a=>a.meta_window.title==="SSD fixture").meta_window.activate(global.get_current_time());true'
    )
    time.sleep(0.5)
    state = inspect()
    assert state["layout"]["mode"] == 2, state
    renderer = "bingux" if state["layout"]["presentation"]["external"] else "native"
    assert state["layout"]["border"][0] in (36, 46) and state["layout"]["border"][1:] == [0, 0, 0], state
    x, y, w, h = state["frame"]
    run(["grim", str(root / f"{renderer}-ssd.png")])
    # Observe every native-fallback visibility transition, not a screenshot
    # taken after the transient flash has already disappeared.
    if renderer == "bingux":
        evaluate(
            'global.ssdFallbackFlashes=0;global.ssdObservedActor=global.get_window_actors().find(a=>a.meta_window.title==="SSD fixture");global.ssdObservedRoot=global.ssdObservedActor.get_children().find(c=>c.get_name()==="gnoblin-native-frame");global.ssdObservedFallback=global.ssdObservedRoot.get_children()[0];global.ssdVisibilitySignal=global.ssdObservedFallback.connect("notify::visible",()=>{if(global.ssdObservedFallback.visible)global.ssdFallbackFlashes++;});true'
        )
        for i in range(30):
            title = "Nested Ghostty" if i % 2 == 0 else "SSD fixture"
            evaluate(
                f"global.get_window_actors().find(a=>a.meta_window.title==={json.dumps(title)}).meta_window.activate(global.get_current_time());true"
            )
            time.sleep(0.025)
        flashes = evaluate(
            "global.ssdObservedFallback.disconnect(global.ssdVisibilitySignal);global.ssdFallbackFlashes"
        )
        assert flashes == 0, ("old native SSD flashed during focus changes", flashes)
        assert inspect()["layout"]["presentation"]["external"], "Bingux lost presentation on focus"
        print("PASS: 30 focus changes, zero native-fallback visibility flashes", flush=True)
        # The external surface must not be the picked actor: the compositor's
        # transparent hit strips drive Bingux's hover repaint.
        regions = state["layout"]["presentation"]["regions"]
        control = next(r for r in regions if r[0] in (2, 3, 4))
        input_event("move", x=x + w // 2, y=y + 18 + 40)
        time.sleep(0.3)
        run(["grim", str(root / "bingux-hover-before.png")])
        input_event("move", x=x + control[1] + control[3] // 2, y=y + control[2] + control[4] // 2)
        time.sleep(0.04)
        run(["grim", str(root / "bingux-hover-mid.png")])
        time.sleep(0.16)
        run(["grim", str(root / "bingux-hover-after.png")])
        before = Image.open(root / "bingux-hover-before.png").convert("RGB")
        mid = Image.open(root / "bingux-hover-mid.png").convert("RGB")
        after = Image.open(root / "bingux-hover-after.png").convert("RGB")
        crop = (x + control[1], y + control[2], x + control[1] + control[3], y + control[2] + control[4])
        changed = sum(a != b for a, b in zip(before.crop(crop).getdata(), after.crop(crop).getdata()))
        mid_changed = sum(a != b for a, b in zip(before.crop(crop).getdata(), mid.crop(crop).getdata()))
        settled_changed = sum(a != b for a, b in zip(mid.crop(crop).getdata(), after.crop(crop).getdata()))
        hovered = evaluate(
            'imports.gi.Meta.gnoblin_window_frame_get(global.get_window_actors().find(a=>a.meta_window.title==="SSD fixture").meta_window).recursiveUnpack().presentation.hover'
        )
        assert hovered == control[0], ("compositor did not record Bingux control hover", hovered, control)
        assert changed, "Bingux control did not repaint on compositor hover"
        assert mid_changed and settled_changed, ("Bingux control hover did not animate", mid_changed, settled_changed)
        print("PASS: Bingux control hover changed rendered pixels with a fade", flush=True)
        input_event("move", x=x + w // 2, y=y + 70)
        run(["grim", str(root / "bingux-leave-mid.png")])
        time.sleep(0.3)
        run(["grim", str(root / "bingux-leave-after.png")])
        leave_mid = Image.open(root / "bingux-leave-mid.png").convert("RGB").crop(crop).tobytes()
        leave_after = Image.open(root / "bingux-leave-after.png").convert("RGB").crop(crop).tobytes()
        assert leave_mid != after.crop(crop).tobytes() and leave_mid != leave_after, "Bingux hover-out snapped"
        assert leave_after == before.crop(crop).tobytes(), "Bingux hover-out did not return to GTK idle colors"
        print("PASS: Bingux control fades back to its original GTK colors on leave", flush=True)
        evaluate(
            'global.get_window_actors().find(a=>a.meta_window.title==="Nested Ghostty").meta_window.focus(global.get_current_time());true'
        )
        time.sleep(0.07)
        run(["grim", str(root / "bingux-focus-mid.png")])
        time.sleep(0.3)
        run(["grim", str(root / "bingux-focus-after.png")])
        sample = (x + 20, y + 12)
        focused_color = before.getpixel(sample)
        middle_color = Image.open(root / "bingux-focus-mid.png").convert("RGB").getpixel(sample)
        unfocused_color = Image.open(root / "bingux-focus-after.png").convert("RGB").getpixel(sample)
        assert focused_color != middle_color != unfocused_color, (
            "Bingux focus color snapped",
            focused_color,
            middle_color,
            unfocused_color,
        )
        evaluate(
            'global.get_window_actors().find(a=>a.meta_window.title==="SSD fixture").meta_window.activate(global.get_current_time());true'
        )
        time.sleep(0.3)
        print("PASS: Bingux header background fades on focus loss", flush=True)

        def double_click_title():
            current = inspect()["frame"]
            input_event("move", x=current[0] + current[2] // 2, y=current[1] + 18)
            input_event("button", down=True)
            input_event("button", down=False)
            input_event("button", down=True)
            input_event("button", down=False)
            time.sleep(0.25)

        double_click_title()
        assert inspect()["maximized"], "Bingux titlebar double-click did not maximize"
        double_click_title()
        assert not inspect()["maximized"], "Bingux titlebar double-click did not restore"
        print("PASS: Bingux titlebar double-click toggled maximize/restore", flush=True)
    input_event("move", x=x + 90, y=y + 18)
    input_event("button", button=3, down=True)
    input_event("button", button=3, down=False)
    time.sleep(1)
    run(["grim", str(root / f"{renderer}-menu.png")])
    (root / f"{renderer}-menu-position.json").write_text(json.dumps({"x": x + 90, "y": y + 18}))
    print("MENU SCREENSHOT:", root / f"{renderer}-menu.png", flush=True)
    # The first menu row is Minimize. Opening the menu changed focus; the
    # action must still target the original SSD window.
    input_event("move", x=x + 130, y=y + 46)
    input_event("button", down=True)
    input_event("button", down=False)
    time.sleep(0.5)
    assert inspect()["minimized"], "menu did not minimize its original SSD target"
    evaluate('global.get_window_actors().find(a=>a.meta_window.title==="SSD fixture").meta_window.unminimize();true')
    print("PASS:", renderer, "SSD right-click menu minimized the original window", flush=True)
