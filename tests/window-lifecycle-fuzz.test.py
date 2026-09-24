#!/usr/bin/env python3
"""Deterministic unit checks for lifecycle fuzz plan generation and replay."""

import importlib.util
import json
from pathlib import Path
import tempfile

module_path = Path(__file__).with_name("window-lifecycle-fuzz.py")
spec = importlib.util.spec_from_file_location("window_lifecycle_fuzz", module_path)
fuzz = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(fuzz)

first = fuzz.generate_plan(seed=1738, steps=200, max_windows=5)
second = fuzz.generate_plan(seed=1738, steps=200, max_windows=5)
assert first == second, "same seed must generate the same plan"
assert len(first["actions"]) >= 201
assert first["actions"][0]["op"] == "open"
assert first["actions"][-1]["op"] == "shell_shutdown"
assert fuzz.window_operation_expression({"op": "maximize"}, "window").endswith(".maximize()")
assert fuzz.window_operation_expression(
    {"op": "resize", "x": 10, "y": 20, "width": 300, "height": 200}, "window"
).endswith(".move_resize_frame(false,10,20,300,200)")
assert fuzz.frame_button_center(
    {
        "x": 100,
        "y": 50,
        "layout": {"presentation": {"regions": [[2, 300, 10, 20, 16]]}},
    },
    2,
) == (410, 68)

live = set()
peak = 0
for action in first["actions"]:
    if action["op"] == "open":
        assert action["window"] not in live
        live.add(action["window"])
        peak = max(peak, len(live))
    elif action["op"] in {"wm_close", "graceful_close", "abrupt_close", "frame_click"}:
        assert action["window"] in live
        live.remove(action["window"])
    else:
        assert action["window"] in live
assert peak <= 5

with tempfile.TemporaryDirectory() as temporary:
    path = Path(temporary) / "plan.json"
    path.write_text(json.dumps(first))
    assert fuzz.read_plan(path) == first

print("PASS: deterministic seeded plans, bounded window lifetimes, and replay loading")
