"""Shared source readers for tests that assert on the native control sources.

The control code lives in gnoblin-native-control.c plus any gnoblin-control-*.c
files next to it. Tests read the combined text through these helpers.
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CONTROL_DIR = ROOT / "src/native-control"


def _combined(main_name: str, pattern: str) -> str:
    paths = [CONTROL_DIR / main_name, *sorted(CONTROL_DIR.glob(pattern))]
    return "".join(path.read_text() for path in paths)


def control_source() -> str:
    return _combined("gnoblin-native-control.c", "gnoblin-control-*.c")


def control_header() -> str:
    return _combined("gnoblin-native-control.h", "gnoblin-control-*.h")
