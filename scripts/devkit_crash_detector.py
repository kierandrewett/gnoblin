#!/usr/bin/env python3
"""Crash-log matching shared by the devkit harness and static tests."""

from __future__ import annotations

import re


CRASH_PATTERNS = tuple(
    re.compile(pattern, re.IGNORECASE)
    for pattern in (
        r"assertion failed",
        r"\bBail out\b",
        r"\bg_assert\b",
        r"Aborted \(core dumped\)",
        r"\bSegmentation fault\b",
        r"\bstatus 139\b",
        r"\bruntime check failed\b",
        r"\b[A-Za-z0-9_.-]+-CRITICAL\b",
        r"\bpanicked at\b",
        r"frame_callback_list",
        r"wl_abort",
    )
)


def match_crash_log(text: str) -> str | None:
    """Return the first matching log line for a compositor-fatal signature."""
    for pattern in CRASH_PATTERNS:
        if pattern.search(text):
            return next(
                (line.strip() for line in text.splitlines() if pattern.search(line)),
                pattern.pattern,
            )
    return None


def _self_test() -> int:
    fatal = [
        "(gnoblin-shell:1): mdk-CRITICAL **: mdk_keyboard_notify_key failed",
        "meta_context_terminate: runtime check failed: (g_main_loop_is_running (priv->main_loop))",
        "Segmentation fault (core dumped)",
        "autostart client exited (status 139)",
        "thread 'main' panicked at src/lib.rs:1",
        "Bail out! assertion failed: frame_callback_list",
    ]
    benign = [
        "WARNING: radv is not a conformant Vulkan implementation, testing use only.",
        "(gnoblin-shell:1): libmutter-WARNING **: D-Bus client with active sessions vanished",
        "zwlr_layer_surface_v1#7: error 0: expected client protocol error",
    ]

    for line in fatal:
        if not match_crash_log(line):
            print(f"FAIL: did not match fatal line: {line}")
            return 1
    for line in benign:
        if match_crash_log(line):
            print(f"FAIL: matched benign line: {line}")
            return 1
    print("PASS: devkit crash detector signatures")
    return 0


if __name__ == "__main__":
    raise SystemExit(_self_test())
