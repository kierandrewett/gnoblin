"""Shared source readers for tests that assert on the native control sources.

The control code lives in src/control/gnoblin-native-control.c plus any gnoblin-control-*.c
files under src/. Tests read the combined text through these helpers.
"""

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SRC_DIR = ROOT / "src"
CONTROL_DIR = SRC_DIR / "control"


# gnoblin-native-control.c was split in file order. Reading these first, in this order, keeps text searches that
# depend on one function appearing before another working as they did on the single file.
NATIVE_CONTROL_SPLIT = [
    "gnoblin-native-control.c",
    "gnoblin-control-focus.c",
    "gnoblin-control-input-config.c",
    "gnoblin-control-wire.c",
    "gnoblin-control-input-routing.c",
    "gnoblin-control-dispatch.c",
    "gnoblin-control-publish.c",
    "gnoblin-control-events.c",
    "gnoblin-control-text-snap.c",
    "gnoblin-control-launches.c",
    "gnoblin-control-monitors.c",
    "gnoblin-control-queue.c",
    "gnoblin-control-requests.c",
    "gnoblin-control-signals.c",
    "gnoblin-control-connection.c",
    "gnoblin-control-runtime-channel.c",
    "gnoblin-control-api.c",
]


def _combined(first: list[Path], pattern: str) -> str:
    others = [path for path in sorted(SRC_DIR.rglob(pattern)) if path not in first]
    return "".join(path.read_text() for path in [*first, *others])


def control_source() -> str:
    return _combined([CONTROL_DIR / name for name in NATIVE_CONTROL_SPLIT], "gnoblin-control-*.c")


def control_header() -> str:
    return _combined([CONTROL_DIR / "gnoblin-native-control.h"], "gnoblin-control-*.h")


def overlay_manifests() -> str:
    """Every overlay manifest under src/, joined. Tests look for one line in the set."""
    return "".join(path.read_text() for path in sorted(SRC_DIR.rglob("manifest")))


class JoinedSources:
    """Several C files that were split from one. read_text() returns them in the old file order."""

    def __init__(self, directory: Path, names: list[str]) -> None:
        self.directory = directory
        self.names = names

    def read_text(self) -> str:
        return "".join((self.directory / name).read_text() for name in self.names)


LUA_SOURCES = JoinedSources(
    SRC_DIR / "config/lua",
    [
        "gnoblin-lua.c",
        "gnoblin-lua-variant.c",
        "gnoblin-lua-merge.c",
        "gnoblin-lua-subscriptions.c",
        "gnoblin-lua-operations.c",
        "gnoblin-lua-contexts.c",
        "gnoblin-lua-animation.c",
        "gnoblin-lua-input.c",
        "gnoblin-lua-named.c",
        "gnoblin-lua-queries.c",
        "gnoblin-lua-state-queries.c",
        "gnoblin-lua-install.c",
        "gnoblin-lua-runtime.c",
        "gnoblin-lua-dispatch.c",
        "gnoblin-lua-console.c",
        "gnoblin-lua-read.c",
        "gnoblin-lua-finish.c",
    ],
)
CONFIG_SOURCES = JoinedSources(
    SRC_DIR / "config",
    [
        "gnoblin-config-input.c",
        "gnoblin-config-windows.c",
        "gnoblin-config-portal.c",
        "gnoblin-config-keybindings.c",
        "gnoblin-config-validate.c",
        "gnoblin-config.c",
        "gnoblin-config-salvage.c",
    ],
)
RUNTIME_SOURCES = JoinedSources(
    SRC_DIR / "session",
    [
        "gnoblin-runtime-diagnostics.c",
        "gnoblin-runtime-environment.c",
        "gnoblin-runtime-packets.c",
        "gnoblin-runtime-events.c",
        "gnoblin-runtime-compositor.c",
        "gnoblin-runtime-worker.c",
        "gnoblin-runtime-autostart.c",
        "gnoblin-runtime-supervisor.c",
        "gnoblin-runtime.c",
    ],
)
