#!/usr/bin/env python3
"""Regression checks for selecting real application toplevels."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gnoblin_test_session import application_window_candidates, constrain_move_to_monitor  # noqa: E402


def main() -> int:
    windows = [
        {"sequence": 2, "title": "previous app", "type": 0, "ready": True, "mapped": True, "focused": True},
        {"sequence": 15, "title": "startup splash", "type": 8, "ready": True, "mapped": True, "focused": True},
        {"sequence": 17, "title": "PSPP Data Editor", "type": 0, "ready": True, "mapped": True},
        {"sequence": 18, "title": "", "type": 0, "ready": True, "mapped": True},
        {"sequence": 19, "title": "not ready", "type": 0, "ready": False, "mapped": True},
        {"sequence": 20, "title": "not mapped", "type": 0, "ready": True, "mapped": False},
        {"sequence": 16, "title": "modal dialog", "type": 4, "ready": True, "mapped": True},
        {"sequence": 21, "title": "focused app dialog", "type": 0, "ready": True, "mapped": True, "focused": True},
    ]
    selected = application_window_candidates(windows, baseline={2}, splashscreen_type=8, modal_dialog_type=4)
    if [window["sequence"] for window in selected] != [16, 21, 17]:
        raise SystemExit(f"expected only the mapped application toplevel, got {selected!r}")
    maximized = {
        "x": 0,
        "y": 28,
        "width": 1280,
        "height": 772,
        "monitor_rect": {"x": 0, "y": 0, "width": 1280, "height": 800},
    }
    if constrain_move_to_monitor(maximized, 48, 76) != (0, 28):
        raise SystemExit("an already monitor-sized window must not be moved outside its monitor")
    if constrain_move_to_monitor(maximized, 300, 140) != (0, 28):
        raise SystemExit("a monitor-sized window must keep every requested move within monitor bounds")
    normal = {**maximized, "x": 100, "y": 100, "width": 700, "height": 440}
    if constrain_move_to_monitor(normal, 48, 76) != (48, 76):
        raise SystemExit("normal windows must retain the requested in-bounds position")
    print("PASS: modal and focused app windows are prioritized while prior, splash, and non-ready windows are excluded")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
