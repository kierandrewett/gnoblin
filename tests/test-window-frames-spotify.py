#!/usr/bin/env python3
"""Optional installed Spotify smoke test: private bus, display and empty profile."""

import json
import os
from pathlib import Path
import signal
import subprocess
import time
from PIL import Image

assert os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-")
repo = Path(__file__).resolve().parents[1]
custom = os.environ.get("GNOBLIN_SSD_RENDERER", "native") != "native"
gtk = os.environ.get("GNOBLIN_SSD_RENDERER") == "bingux"
if custom:
    Path(os.environ["GNOBLIN_SSD_THEME"]).write_text("#204080\n")
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
scripts = root / "scripts"
scripts.mkdir(parents=True, exist_ok=True)
# The private document-portal stub advertises this empty mount point.
(Path(os.environ["XDG_CONFIG_HOME"]).parent / "doc/by-app/com.spotify.Client").mkdir(parents=True, exist_ok=True)
(scripts / "spotify.js").write_text((repo / "tests/window-frames-spotify.js").read_text())
subprocess.run(
    [
        "gdbus",
        "call",
        "--session",
        "--dest",
        "org.gnoblin.Shell",
        "--object-path",
        "/org/gnoblin/Shell",
        "--method",
        "org.gnoblin.Shell.Reload",
    ],
    check=True,
)
with (root / "spotify-client.log").open("w") as log:
    client = subprocess.Popen(
        [
            "flatpak",
            "run",
            "--unshare=network",
            "--env=XDG_CONFIG_HOME=/tmp/gnoblin-ssd-profile/config",
            "--env=XDG_CACHE_HOME=/tmp/gnoblin-ssd-profile/cache",
            "--env=WAYLAND_DEBUG=1",
            "--env=LD_PRELOAD=/app/lib/spotify-preload.so",
            "--command=/app/extra/bin/spotify",
            "com.spotify.Client//stable",
            "--ozone-platform=wayland",
            "--user-data-dir=/tmp/gnoblin-ssd-profile/cef",
        ],
        stdout=log,
        stderr=log,
        start_new_session=True,
    )
    try:
        deadline = time.monotonic() + 25
        rows = []
        result = root / "spotify-frames.json"
        while time.monotonic() < deadline:
            if result.exists():
                rows = json.loads(result.read_text())
            if rows and rows[0]["visible"] and (not custom or rows[0]["layout"]["presentation"]["external"]):
                break
            if client.poll() is not None:
                break
            time.sleep(0.2)
        print("SPOTIFY", rows, flush=True)
        assert rows and rows[0]["layout"]["mode"] == 2 and rows[0]["visible"], (
            root / "spotify-client.log"
        ).read_text()[-4000:]
        time.sleep(0.8)
        screenshot = Path("/tmp/gnoblin-spotify-ssd.png")
        subprocess.run(["grim", str(screenshot)], check=True)
        x, y, w, h = rows[0]["frame"]
        pixel = Image.open(screenshot).convert("RGB").getpixel((x + 24, y + 4))
        expected = (255, 255, 255) if gtk else ((32, 64, 128) if custom else (36, 36, 36))
        assert pixel == expected, pixel
        # No wallpaper seam between the header and the application's first row.
        image = Image.open(screenshot).convert("RGB")
        edge = y + rows[0]["layout"]["border"][0] - 1
        # GTK includes its own uniform bottom separator; keep those real pixels.
        edge_color = image.getpixel((x + 24, edge)) if gtk else expected
        for px in range(x + 24, x + w - 24):
            assert image.getpixel((px, edge)) == edge_color, ("header seam", px, image.getpixel((px, edge)))
        # At radius 12 and inset 2, these pixels are within the outer frame
        # but outside the rounded client hole. They must be frame, not black app.
        interior = image.getpixel((x + 14, y + h - 6))
        for px in (x + 4, x + w - 5):
            corner = image.getpixel((px, y + h - 5))
            # The explicit inner border and antialiasing shade the white GTK
            # frame; require a clear frame contribution, not pure white.
            assert min(corner) > max(interior) + 80 if gtk else corner == expected, (
                "square inner bottom corner",
                corner,
            )
        assert max(interior) < 80, "bottom rounding covered client interior"
        print("PASS: real Spotify negotiated SSD and painted the Gnoblin titlebar", screenshot)
    finally:
        if client.poll() is None:
            os.killpg(client.pid, signal.SIGTERM)
        client.wait(timeout=5)
