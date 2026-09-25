#!/usr/bin/env python3
"""Regression checks for selecting real application toplevels."""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from gnoblin_test_session import application_window_candidates  # noqa: E402


def main() -> int:
    windows = [
        {"sequence": 2, "title": "previous app", "type": 0, "ready": True, "mapped": True},
        {"sequence": 15, "title": "startup splash", "type": 8, "ready": True, "mapped": True},
        {"sequence": 16, "title": "modal dialog", "type": 4, "ready": True, "mapped": True},
        {"sequence": 17, "title": "PSPP Data Editor", "type": 0, "ready": True, "mapped": True},
        {"sequence": 18, "title": "", "type": 0, "ready": True, "mapped": True},
        {"sequence": 19, "title": "not ready", "type": 0, "ready": False, "mapped": True},
        {"sequence": 20, "title": "not mapped", "type": 0, "ready": True, "mapped": False},
    ]
    selected = application_window_candidates(windows, baseline={2}, splashscreen_type=8)
    if [window["sequence"] for window in selected] != [16, 17]:
        raise SystemExit(f"expected only the mapped application toplevel, got {selected!r}")
    print("PASS: modal dialogs are retained while prior, splash, and non-ready windows are excluded")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
