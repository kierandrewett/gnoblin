#!/usr/bin/env python3
"""Deterministic unit checks for native lifecycle fuzz plan generation/replay."""

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
assert all(action["op"] in fuzz.VALID_OPERATIONS for action in first["actions"])
assert fuzz.VALID_OPERATIONS.isdisjoint(fuzz.SEPARATE_TEST_OPERATIONS)

live = set()
peak = 0
for action in first["actions"]:
    if action["op"] == "open":
        assert action["window"] not in live
        live.add(action["window"])
        peak = max(peak, len(live))
    elif action["op"] in fuzz.CLOSE_OPERATIONS:
        assert action["window"] in live
        live.remove(action["window"])
    else:
        assert action["window"] in live
        if action["op"] == "resize":
            assert all(isinstance(action[key], int) for key in ("x", "y", "width", "height"))
assert peak <= 5

with tempfile.TemporaryDirectory() as temporary:
    path = Path(temporary) / "plan.json"
    path.write_text(json.dumps(first))
    assert fuzz.read_plan(path) == first

    unsupported = json.loads(json.dumps(first))
    unsupported["actions"].append({"op": "frame_click", "window": 0})
    path.write_text(json.dumps(unsupported))
    try:
        fuzz.read_plan(path)
    except ValueError as error:
        assert "invalid action" in str(error)
    else:
        raise AssertionError("legacy frame_click plans must be rejected explicitly")

print("PASS: deterministic native plans, bounded window lifetimes, and replay loading")
