#!/usr/bin/env python3
"""Check actual GTK Files corner fill and border continuity on a disposable VM.

Requires an installed Bingux session with one scale-1 1280x800 output, Nautilus,
and Pillow on the host. Saves screenshots, magnified corners and window state.
The temporary config rule and uniquely owned Files service are removed on exit.
"""

import argparse
import json
from pathlib import Path
import socket
import subprocess
import time

from PIL import Image, ImageChops

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("run", type=Path)
args = parser.parse_args()
run = args.run.resolve()
runner = str(Path(__file__).resolve().parents[2] / "scripts/qemu-e2e")
env = [
    "env",
    "XDG_RUNTIME_DIR=/run/user/1000",
    "WAYLAND_DISPLAY=wayland-0",
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus",
]
ctl = "/usr/local/lib/gnoblin/bin/gnoblinctl"
unit = "files-csd-proof-" + str(time.time_ns())
config = "/home/luna/.config/gnoblin/config/99-" + unit + ".lua"
art = run / unit
art.mkdir(exist_ok=False)
records = []
results = []
owned = False
config_created = False
sock = None
stream = None


def guest(*argv):
    return subprocess.check_output([runner, "guest", str(run), "--", *env, *argv], timeout=20)


def control(*argv):
    return json.loads(guest(ctl, *argv))


def files():
    return [w for w in control("window", "list")["windows"] if w["app_id"] == "org.gnome.Nautilus"]


def window():
    matches = files()
    assert len(matches) == 1, matches
    return matches[0]


def wait(predicate, message):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.1)
    raise AssertionError(message)


def qmp(command, **arguments):
    stream.write(json.dumps({"execute": command, "arguments": arguments}) + "\n")
    stream.flush()
    while True:
        reply = json.loads(stream.readline())
        if "error" in reply:
            raise RuntimeError(reply)
        if "return" in reply:
            return reply["return"]


def capture(label):
    time.sleep(0.8)
    current = window()
    frame = current["frame"]
    raw = art / (label + ".ppm")
    qmp("screendump", filename=str(raw))
    image = Image.open(raw).convert("RGB")
    image.save(art / (label + ".png"))
    raw.unlink()
    x, y, width, height = (frame[k] for k in ("x", "y", "width", "height"))
    pixels = image.crop((x, y, x + width, y + height))
    pixels.save(art / (label + "-window.png"))
    for name, bx, by in [
        ("tl", x, y),
        ("tr", x + width - 40, y),
        ("bl", x, y + height - 40),
        ("br", x + width - 40, y + height - 40),
    ]:
        image.crop((bx - 10, by - 10, bx + 50, by + 50)).resize((600, 600), Image.Resampling.NEAREST).save(
            art / (label + "-" + name + ".png")
        )
    records.append({"label": label, "window": current})
    (art / "records.json").write_text(json.dumps(records, indent=2))
    return pixels


def configure(remove, radius=4, border=0):
    text = (
        'gnoblin.window_rule {match={app_id="^org%.gnome%.Nautilus$"},animation="none",'
        'corners={mode="force",radius='
        + str(radius)
        + ",smoothing=0,border_width="
        + str(border)
        + ',border_color="#ff00ffff",shadow=false,remove_csd='
        + ("true" if remove else "false")
        + "}}\n"
    )
    guest("python3", "-c", "from pathlib import Path; Path(" + repr(config) + ").write_text(" + repr(text) + ")")
    control("config", "reload")


def check_corners(off, on, label):
    width, height = on.size
    samples = []
    # Equal-depth straight-edge samples avoid GTK gradients and reveal the
    # correct local colour: sidebar at left, content background at right.
    for point, adjacent in [
        ((3, 3), (25, 3)),
        ((width - 4, 3), (width - 25, 3)),
        ((3, height - 4), (25, height - 4)),
        ((width - 4, height - 4), (width - 25, height - 4)),
    ]:
        before, filled, background = off.getpixel(point), on.getpixel(point), on.getpixel(adjacent)
        assert max(abs(filled[i] - background[i]) for i in range(3)) <= 2, (label, point, filled, background)
        assert max(abs(before[i] - filled[i]) for i in range(3)) > 30, (label, point, before, filled)
        samples.append({"point": point, "disabled": before, "filled": filled, "app": background})
    body = (40, 40, width - 40, height - 40)
    assert ImageChops.difference(off.crop(body), on.crop(body)).getbbox() is None, "Fill changed opaque app body"
    results.append({"case": label, "corners": samples, "opaque_body_unchanged": True})
    (art / "assertions.json").write_text(json.dumps(results, indent=2))


try:
    assert not files(), "Close Files in the disposable VM before running this fixture"
    monitors = control("monitor", "list")["monitors"]
    assert len(monitors) == 1 and (monitors[0]["width"], monitors[0]["height"]) == (1280, 800), monitors
    guest("python3", "-c", "from pathlib import Path; assert not Path(" + repr(config) + ").exists()")
    (art / "versions.txt").write_bytes(guest(ctl, "version") + guest("nautilus", "--version"))
    sock = socket.socket(socket.AF_UNIX)
    sock.settimeout(10)
    sock.connect(str(run / "qmp.sock"))
    stream = sock.makefile("rw")
    stream.readline()
    qmp("qmp_capabilities")
    guest("systemd-run", "--user", "--collect", "--unit=" + unit, "/usr/bin/nautilus", "--new-window", "/home/luna")
    owned = True
    wait(files, "Files did not open")
    target = window()["id"]
    # Files remembers maximization from the previous fixture's shutdown.
    if window()["maximized"]:
        control("window", "unmaximize", target)
        wait(lambda: not window()["maximized"], "Files did not unmaximize")
    control("window", "move", target, "300", "100")
    control("window", "resize", target, "700", "480")
    wait(lambda: window()["frame"]["width"] == 700, "Files did not resize")
    capture("default")
    config_created = True
    for index in range(4):
        configure(False)
        off = capture(f"{index}-fill-off")
        configure(True)
        on = capture(f"{index}-fill-on")
        check_corners(off, on, str(index))
    configure(True, 12, 1)
    border = capture("magenta-outline")
    width, height = border.size
    edges = {
        "top": [(x, 0) for x in range(20, width - 20)],
        "bottom": [(x, height - 1) for x in range(20, width - 20)],
        "left": [(0, y) for y in range(20, height - 20)],
        "right": [(width - 1, y) for y in range(20, height - 20)],
    }
    for name, points in edges.items():
        assert all(border.getpixel(point) == (255, 0, 255) for point in points), "Discontinuous border: " + name
        results.append({"edge": name, "continuous_pixels": len(points)})
    # Adjacent inner pixels must be app pixels, not wallpaper or another outline.
    assert border.getpixel((1, height // 2)) == on.getpixel((1, height // 2))
    assert border.getpixel((width - 2, height // 2)) == on.getpixel((width - 2, height // 2))
    control("window", "maximize", target)
    wait(lambda: window()["maximized"], "Files did not maximize")
    maximized = capture("maximized")
    # The shell's bar stroke can overlap the first row of the work area.
    # Compare each corner with its same-row straight edge, not a fixed colour.
    assert maximized.getpixel((0, 0)) == maximized.getpixel((40, 0)), "Maximized Files retained a rounded cutout"
    assert maximized.getpixel((1, 1)) == maximized.getpixel((40, 1)), (
        "Maximized Files retained a rounded cutout inside the shell stroke"
    )
    control("window", "unmaximize", target)
    wait(lambda: not window()["maximized"], "Files did not restore")
    restored = capture("restored")
    assert restored.size == border.size
    for points in edges.values():
        assert all(restored.getpixel(point) == border.getpixel(point) for point in points), (
            "Restoring changed outline pixels"
        )
    # GTK can fade its content scrollbar after resize; compare the frame's
    # corners rather than treating changing app controls as decoration errors.
    for box in [
        (0, 0, 30, 30),
        (width - 30, 0, width, 30),
        (0, height - 30, 30, height),
        (width - 30, height - 30, width, height),
    ]:
        assert ImageChops.difference(restored.crop(box), border.crop(box)).getbbox() is None, (
            "Restoring changed corner pixels",
            box,
        )
    (art / "assertions.json").write_text(json.dumps(results, indent=2))
    print(
        "PASS actual GTK Files: 16 reconstructed corners, unchanged opaque body, continuous four-edge outline and restore"
    )
    print("Evidence:", art)
finally:
    if config_created:
        guest("python3", "-c", "from pathlib import Path; Path(" + repr(config) + ").unlink(missing_ok=True)")
        control("config", "reload")
    if owned:
        guest("systemctl", "--user", "stop", unit + ".service")
    if stream is not None:
        stream.close()
    if sock is not None:
        sock.close()
