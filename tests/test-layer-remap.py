#!/usr/bin/env python3
"""Regression: a wl_surface can become a layer surface again after it was hidden."""

import os
from pathlib import Path
import shlex
import subprocess
import tempfile

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"), "Use the private test session"
repo = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="gnoblin-layer-remap-") as directory:
    build = Path(directory)
    layer = repo / "src/protocols/layer-shell/wlr-layer-shell-unstable-v1.xml"
    protocols = subprocess.check_output(["pkg-config", "--variable=pkgdatadir", "wayland-protocols"], text=True).strip()
    for mode, xml, filename in [
        ("client-header", layer, "wlr-layer-shell-unstable-v1-client-protocol.h"),
        ("private-code", layer, "layer.c"),
        ("private-code", Path(protocols) / "stable/xdg-shell/xdg-shell.xml", "xdg.c"),
    ]:
        subprocess.run(["wayland-scanner", mode, str(xml), str(build / filename)], check=True)
    flags = shlex.split(subprocess.check_output(["pkg-config", "--cflags", "--libs", "wayland-client"], text=True))
    subprocess.run(
        [
            "cc",
            "-I" + str(build),
            str(repo / "tests/layer-shell-remap-client.c"),
            str(build / "layer.c"),
            str(build / "xdg.c"),
            *flags,
            "-o",
            str(build / "client"),
        ],
        check=True,
    )
    for settle, label in [(0, "immediately"), (1, "after the unmap is processed")]:
        result = subprocess.run(
            [str(build / "client"), "4", str(settle)], capture_output=True, text=True, timeout=10
        )
        assert result.returncode == 0, (label, result.stdout, result.stderr)
        assert result.stdout.split() == ["MAPPED", "1", "MAPPED", "2", "MAPPED", "3", "MAPPED", "4"], (
            label,
            result.stdout,
        )
        print(f"PASS: a hidden layer surface maps again {label}, four times over")
