#!/usr/bin/env python3
"""Compare real drag damage to a full repaint using the normal config."""

from pathlib import Path
import json
import runpy
import time
from PIL import Image, ImageChops

check = runpy.run_path(str(Path(__file__).with_name("nested-ssd-check.py")), run_name="nested_input")
evaluate, run, root, input_event = [check[k] for k in ("evaluate", "run", "root", "input_event")]
probe = str(root / "frame-probe-v2")
evaluate(
    f"imports.gi.GIRepository.Repository.dup_default().prepend_search_path({json.dumps(probe)});imports.gi.GIRepository.Repository.dup_default().prepend_library_path({json.dumps(probe)});global.dragFrameProbe=imports.gi.FrameProbe2;true"
)
try:
    for title in ["RustDesk", "Nested Ghostty"]:
        evaluate(
            f"global.dragWindow=global.get_window_actors().find(a=>a.meta_window.title==={json.dumps(title)}).meta_window;global.dragWindow.unminimize();global.dragWindow.activate(global.get_current_time());global.dragWindow.move_frame(false,200,100);true"
        )
        time.sleep(0.7)
        frame = evaluate("(()=>{let f=global.dragWindow.get_frame_rect();return [f.x,f.y,f.width,f.height];})()")
        x, y, w, h = frame
        input_event("move", x=x + 80, y=y + 18)
        input_event("button", down=True)
        for i in range(20):
            input_event("move", x=x + 80 + (i + 1) * 15, y=y + 18 + (i + 1) * 5)
        input_event("button", down=False)
        time.sleep(0.5)
        incremental = root / f"{title}-incremental.png"
        full = root / f"{title}-full.png"
        evaluate(
            f"global.dragFrameProbe.arm(global.stage,{json.dumps(str(incremental))});global.stage.queue_redraw_with_clip(new imports.gi.Mtk.Rectangle({{x:200,y:100,width:1,height:1}}));true"
        )
        time.sleep(0.3)
        assert incremental.exists(), "No incremental paint was captured"
        evaluate(f"global.dragFrameProbe.arm(global.stage,{json.dumps(str(full))});global.stage.queue_redraw();true")
        time.sleep(0.3)
        a, b = Image.open(incremental).convert("RGB"), Image.open(full).convert("RGB")
        # Panel clocks and hover-animated dock icons are unrelated to window damage.
        box = (0, 40, a.width, a.height - 110)
        diff = ImageChops.difference(a.crop(box), b.crop(box))
        changed = sum(max(pixel) > 5 for pixel in diff.getdata())
        diff.save(root / f"{title}-damage-diff.png")
        final = evaluate("(()=>{let f=global.dragWindow.get_frame_rect();return [f.x,f.y];})()")
        assert final != frame[:2], ("input did not drag the window", title, frame, final)
        assert max(high for low, high in a.getextrema()) > 100, "Framebuffer reader returned a blank image"
        print(title, "drag", frame[:2], "->", final, "changed pixels vs full repaint:", changed, flush=True)
        assert changed < 50, (title, "stale pixels after drag", changed, diff.getbbox())
    print("PASS: real RustDesk and Ghostty drags leave no stale window pixels")
finally:
    evaluate("global.dragFrameProbe.stop();true")
