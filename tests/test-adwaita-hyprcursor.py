#!/usr/bin/env python3
"""Validate Adwaita vectors through the installed Hyprcursor library."""

import ctypes as C
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import time
import zipfile

ROOT = Path(__file__).resolve().parent.parent
THEME = "Adwaita-Hyprcursor"
theme = Path(os.environ.get("ADWAITA_HYPRCURSOR_PATH", Path.home() / ".local/share/icons" / THEME))
if not os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"):
    assert theme.resolve() == (Path.home() / ".local/share/icons" / THEME).resolve(), (
        "Install the theme before testing it"
    )
metadata = json.loads((theme / "cursors.json").read_text())


class Style(C.Structure):
    _fields_ = [("size", C.c_uint)]


class Frame(C.Structure):
    _fields_ = [
        ("surface", C.c_void_p),
        ("size", C.c_int),
        ("delay", C.c_int),
        ("hotspotX", C.c_int),
        ("hotspotY", C.c_int),
    ]


hc = C.CDLL("libhyprcursor.so.0")
cairo = C.CDLL("libcairo.so.2")
hc.hyprcursor_manager_create.argtypes = [C.c_char_p]
hc.hyprcursor_manager_create.restype = C.c_void_p
hc.hyprcursor_manager_valid.argtypes = [C.c_void_p]
hc.hyprcursor_manager_free.argtypes = [C.c_void_p]
hc.hyprcursor_load_theme_style.argtypes = [C.c_void_p, Style]
hc.hyprcursor_get_cursor_image_data.argtypes = [C.c_void_p, C.c_char_p, Style, C.POINTER(C.c_int)]
hc.hyprcursor_get_cursor_image_data.restype = C.POINTER(C.POINTER(Frame))
hc.hyprcursor_cursor_image_data_free.argtypes = [C.POINTER(C.POINTER(Frame)), C.c_int]
hc.hyprcursor_style_done.argtypes = [C.c_void_p, Style]
for name in ("width", "height", "stride"):
    getattr(cairo, "cairo_image_surface_get_" + name).argtypes = [C.c_void_p]
cairo.cairo_image_surface_get_data.argtypes = [C.c_void_p]
cairo.cairo_image_surface_get_data.restype = C.c_void_p
cairo.cairo_surface_flush.argtypes = [C.c_void_p]

if os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"):
    icons = Path.home() / ".icons"
    icons.mkdir(exist_ok=True)
    (icons / THEME).symlink_to(theme.resolve())
manager = hc.hyprcursor_manager_create(THEME.encode())
assert manager and hc.hyprcursor_manager_valid(manager), "Theme did not load"
try:
    for size in (24, 30, 37, 48, 96):
        start = time.monotonic()
        style = Style(size)
        assert hc.hyprcursor_load_theme_style(manager, style)
        for name, info in metadata.items():
            with zipfile.ZipFile(theme / "hyprcursors" / f"{name}.hlc") as archive:
                images = [f for f in archive.namelist() if f.endswith((".svg", ".png"))]
                assert len(images) == info["frames"] and all(f.endswith(".svg") for f in images)
            expected = None
            for shape in [name, *info["aliases"]]:
                count = C.c_int()
                frames = hc.hyprcursor_get_cursor_image_data(manager, shape.encode(), style, C.byref(count))
                assert count.value == info["frames"], (size, shape, count.value)
                hashes = []
                try:
                    for i in range(count.value):
                        frame = frames[i].contents
                        assert cairo.cairo_image_surface_get_width(frame.surface) == size
                        assert cairo.cairo_image_surface_get_height(frame.surface) == size
                        assert (frame.hotspotX, frame.hotspotY) == tuple(
                            math.floor(v * size / 24 + 0.5) for v in info["hotspot"]
                        ), (size, shape)
                        assert frame.delay == max(1, info["duration"]), (shape, frame.delay)
                        cairo.cairo_surface_flush(frame.surface)
                        pixels = C.string_at(
                            cairo.cairo_image_surface_get_data(frame.surface),
                            cairo.cairo_image_surface_get_stride(frame.surface) * size,
                        )
                        assert any(pixels[3::4]), (shape, i, "empty frame")
                        hashes.append(hashlib.sha256(pixels).hexdigest())
                    if expected is None:
                        expected = hashes
                    else:
                        assert hashes == expected, (shape, "alias artwork differs")
                    if count.value > 1:
                        assert len(set(hashes)) == count.value, (shape, "animation frames repeat")
                finally:
                    hc.hyprcursor_cursor_image_data_free(frames, count.value)
        print(f"PASS: {size}px: every SVG frame, hotspot, delay and alias ({time.monotonic() - start:.2f}s)")
        hc.hyprcursor_style_done(manager, style)
finally:
    hc.hyprcursor_manager_free(manager)

# The private compositor check selects the real theme without touching live settings.
if os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"):
    scripts = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin/scripts"
    scripts.mkdir(parents=True, exist_ok=True)
    report = scripts.parent / "adwaita-cursor.json"
    (scripts / "adwaita-cursor-probe.js").write_text(
        """
import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
export default function enable(api) {
    const settings = new Gio.Settings({schema_id: 'org.gnome.desktop.interface'});
    settings.set_string('cursor-theme', THEME);
    // 37 is absent from the bitmap theme, so a 37px texture proves SVG rendering.
    settings.set_int('cursor-size', 37);
    const tracker = global.backend.get_cursor_tracker();
    const device = global.stage.context.get_backend().get_default_seat()
        .create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
    device.notify_absolute_motion(GLib.get_monotonic_time(), 200, 200);
    tracker.set_gnoblin_launch_cursor(true);
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, 150, () => {
        GLib.file_set_contents(REPORT, JSON.stringify({size: tracker.get_sprite().get_width(),
            hotspot: tracker.get_hot(), busy: tracker.get_gnoblin_launch_cursor()}));
        tracker.set_gnoblin_launch_cursor(false);
        return GLib.SOURCE_REMOVE;
    });
    api._disposers.push(() => { tracker.set_gnoblin_launch_cursor(false); device.run_dispose(); });
}
""".replace("THEME", json.dumps(THEME)).replace("REPORT", json.dumps(str(report)))
    )
    subprocess.run([str(ROOT / "src/tools/gnoblinctl"), "script", "reload"], check=True)
    deadline = time.monotonic() + 4
    while not report.exists() and time.monotonic() < deadline:
        time.sleep(0.05)
    actual = json.loads(report.read_text())
    assert actual == {"size": 37, "hotspot": [17, 17], "busy": True}, actual
    print("PASS: Adwaita vector wait cursor rendered by Mutter")
