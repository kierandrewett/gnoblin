#!/usr/bin/env python3
"""Regression checks for selecting real application toplevels."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gnoblin_test_session import (  # noqa: E402
    application_window_candidates,
    close_target_state_ready,
    constrain_move_to_monitor,
    frame_button_center,
    gnoblin_frame_visible,
    wait_for_settled_close_target,
)


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
    visible_frame_without_border = {
        "x": 0,
        "y": 28,
        "layout": {
            "native": True,
            "border": [0, 0, 0, 0],
            "presentation": {
                "visible": True,
                "regions": [[2, 606, 0, 36, 36]],
            },
        },
    }
    if not gnoblin_frame_visible(visible_frame_without_border):
        raise SystemExit("an advertised native frame remains usable when its border width is zero")
    if frame_button_center(visible_frame_without_border, 2) != (624, 46):
        raise SystemExit("a visible native frame close probe must use its advertised button region")
    restored_at_work_area_origin = {
        **visible_frame_without_border,
        "y": 0,
        "actor_position": [0, 36],
        "layout": {
            **visible_frame_without_border["layout"],
            "presentation": {"visible": True, "regions": [[2, 320, 0, 36, 36]]},
        },
    }
    if frame_button_center(restored_at_work_area_origin, 2) != (338, 54):
        raise SystemExit("frame close probes must follow the transformed actor after unfullscreening")
    restoring_after_fullscreen = {
        "x": 0,
        "y": 0,
        "width": 1280,
        "height": 800,
        "fullscreen": False,
        "layout": {
            "native": True,
            "supported": True,
            "border": [0, 0, 0, 0],
            "presentation": {"visible": False, "regions": [[2, 1280, 0, 0, 0]]},
        },
    }
    restored_after_fullscreen = {
        **restoring_after_fullscreen,
        "x": 0,
        "y": 28,
        "height": 772,
        "layout": {
            **restoring_after_fullscreen["layout"],
            "border": [36, 2, 2, 2],
            "presentation": {"visible": True, "regions": [[2, 1242, 0, 36, 36]]},
        },
    }
    if close_target_state_ready(restoring_after_fullscreen):
        raise SystemExit("a nonfullscreen flag must not make stale fullscreen geometry a valid close target")
    if not close_target_state_ready(restored_after_fullscreen):
        raise SystemExit("a restored visible native frame must be a valid close target")
    samples = iter((restoring_after_fullscreen, restored_after_fullscreen, restored_after_fullscreen))
    settled = wait_for_settled_close_target(lambda: next(samples), stable_seconds=0, timeout=1)
    if settled != restored_after_fullscreen:
        raise SystemExit("close-target settling must wait for restored geometry and visible frame regions")
    moving_actor_states = iter(
        {
            **restored_at_work_area_origin,
            "actor_position": [0, y],
        }
        for y in (0, 36, 72, 72)
    )
    settled_actor = wait_for_settled_close_target(lambda: next(moving_actor_states), stable_seconds=0.01, timeout=1)
    if settled_actor["actor_position"] != [0, 72]:
        raise SystemExit("close-target settling must wait for the frame actor's stage position")
    hidden_frame = {
        "layout": {
            **visible_frame_without_border["layout"],
            "presentation": {"visible": False, "regions": [[2, 606, 0, 36, 36]]},
        }
    }
    if gnoblin_frame_visible(hidden_frame):
        raise SystemExit("a hidden native frame must not be treated as a visible close target")
    print("PASS: window selection, monitor-bounded moves, and visible-frame detection handle their edge cases")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
