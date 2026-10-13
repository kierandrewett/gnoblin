#!/usr/bin/env python3
"""Capture native shadow invalidation states in a disposable VM.

Compare the resulting PNGs with a pre-cache build; do not run on a user's seat.
The fixture restores its config drop-in and terminates its Wayland client.
"""

import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import time

from PIL import Image

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("artifacts", type=Path)
parser.add_argument("--client", type=Path, required=True)
parser.add_argument("--prefix", type=Path, default=Path("/usr/local/lib/gnoblin"))
args = parser.parse_args()
art = args.artifacts.resolve()
art.mkdir(parents=True, exist_ok=False)
os.environ.setdefault("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-0")
seat_lock = (Path(os.environ["XDG_RUNTIME_DIR"]) / "gnoblin-shadow-proof.lock").open("w")
fcntl.flock(seat_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
config = Path.home() / ".config/gnoblin/conf.d/99-shadow-cache-proof.lua"
assert config.parent.is_dir(), "VM config must load conf.d/**/*.lua"
assert not config.exists(), "refusing to overwrite an existing drop-in"
log = (art / "frames.log").open("w")
client = None
challenger = None
remote_session = None
window = None
states = []


def control(*argv):
    return json.loads(subprocess.check_output([str(args.prefix / "bin/gnoblinctl"), "--json", *argv], text=True))


def configure(radius=24, smoothing=0, shadow=None, duration=0, opacity=1):
    if shadow is None:
        shadow = '{x=0,y=4,blur=20,spread=0,opacity=0.8,color="#000000"}'
    config.write_text(
        'gnoblin.window_rule {match={app_id="^org%.gnoblin%.PressureProbe$"},'
        f'frame={{mode="off"}},blur=0,opacity={opacity},animation="none",'
        f'corners={{mode="force",radius={radius},smoothing={smoothing},'
        f"remove_csd=false,border_width=0,shadow={shadow},"
        f'shadow_animation={{duration={duration},easing="linear"}}}}}}\n'
    )
    control("config", "reload")


def capture(name, wait=0.8, visible=True):
    time.sleep(wait)
    path = art / f"{name}.png"
    subprocess.run(["grim", str(path)], check=True)
    current = next(w for w in control("window", "list")["windows"] if w["id"] == window["id"])
    image = Image.open(path).convert("RGB")
    if visible:
        assert image.getpixel((round(480 * scale), round(340 * scale))) == (64, 176, 128), (
            "probe body missing or unexpectedly modified"
        )
    states.append(dict(name=name, window=current))
    return image


try:
    monitors = [m for m in control("monitor", "list")["monitors"] if m["enabled"]]
    assert len(monitors) == 1 and monitors[0]["x"] == 0 and monitors[0]["y"] == 0, (
        "fixture requires one monitor at the origin"
    )
    scale = monitors[0]["scale"]
    configure()
    client = subprocess.Popen([str(args.client.resolve())], stdout=log)
    for _ in range(50):
        window = next((w for w in control("window", "list")["windows"] if w["title"] == "Frame pressure probe"), None)
        if window:
            break
        time.sleep(0.1)
    assert window, "probe client did not map"
    control("window", "move", window["id"], "320", "220")
    original = capture("default")
    configure(radius=64, smoothing=0.8)
    capture("shape")
    configure(shadow='{x=18,y=-8,blur=12,spread=8,opacity=0.7,color="#b020e0"}')
    capture("geometry-color-padding")
    configure(
        shadow='{{x=0,y=4,blur=20,opacity=0.8,color="#000000"},{x=-8,y=2,blur=6,spread=3,opacity=0.7,color="#20a0ff"}}'
    )
    capture("two-layers")
    configure(shadow="false")
    disabled = capture("disabled")
    configure()
    enabled = capture("reenabled")
    roi = tuple(round(v * scale) for v in (220, 120, 740, 560))
    assert original.crop(roi).tobytes() == enabled.crop(roi).tobytes(), "re-enabled shadow differs from initial state"
    assert disabled.crop(roi).tobytes() != enabled.crop(roi).tobytes(), "shadow did not clear/reappear"
    configure(shadow='{x=18,y=-8,blur=12,spread=8,opacity=0.7,color="#b020e0"}', duration=1600)
    interim = capture("transition-middle", wait=0.2)
    end = capture("transition-end", wait=1.8)
    assert interim.crop(roi).tobytes() != end.crop(roi).tobytes(), "animated shadow did not update"
    control("window", "minimize", window["id"])
    capture("minimized", visible=False)
    control("window", "unminimize", window["id"])
    capture("restored")
    configure(opacity=0.55)
    capture("opacity", visible=False)
    configure()
    capture("opaque-again")
    # Exercise the same focus-driven style change used by shell integrations.
    from gi.repository import Gio, GLib

    os.environ.setdefault("DBUS_SESSION_BUS_ADDRESS", f"unix:path={os.environ['XDG_RUNTIME_DIR']}/bus")
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    remote_session = bus.call_sync(
        "org.gnome.Mutter.RemoteDesktop",
        "/org/gnome/Mutter/RemoteDesktop",
        "org.gnome.Mutter.RemoteDesktop",
        "CreateSession",
        None,
        None,
        Gio.DBusCallFlags.NONE,
        1500,
        None,
    ).unpack()[0]

    def remote(method, params=None):
        return bus.call_sync(
            "org.gnome.Mutter.RemoteDesktop",
            remote_session,
            "org.gnome.Mutter.RemoteDesktop.Session",
            method,
            params,
            None,
            Gio.DBusCallFlags.NONE,
            1500,
            None,
        )

    remote("Start")
    known = {w["id"] for w in control("window", "list")["windows"]}
    challenger = subprocess.Popen([str(args.client.resolve())], stdout=log)
    other = None
    for _ in range(50):
        other = next(
            (
                w
                for w in control("window", "list")["windows"]
                if w["id"] not in known and w["title"] == "Frame pressure probe"
            ),
            None,
        )
        if other:
            break
        time.sleep(0.1)
    assert other, "focus challenger did not map"
    control("window", "move", other["id"], "800", "220")
    config.write_text(
        config.read_text()
        + 'gnoblin.window_rule {match={app_id="^org%.gnoblin%.PressureProbe$",focused=false},corners={shadow={blur=20,opacity=0.8,color="#e03020"},shadow_animation={duration=400,easing="linear"}}}\n'
    )
    control("config", "reload")

    def click(x, y):
        remote("NotifyPointerMotionRelative", GLib.Variant("(dd)", (-8000.0, -8000.0)))
        remote("NotifyPointerMotionRelative", GLib.Variant("(dd)", (float(x), float(y))))
        remote("NotifyPointerButton", GLib.Variant("(ib)", (272, True)))
        remote("NotifyPointerButton", GLib.Variant("(ib)", (272, False)))

    click(480, 340)
    active = capture("focused")
    assert states[-1]["window"]["focused"], "click did not focus probe"
    click(960, 340)
    inactive = capture("unfocused")
    assert not states[-1]["window"]["focused"], "click did not move focus away"
    assert active.crop(roi).tobytes() != inactive.crop(roi).tobytes(), "focus shadow did not update"
    click(480, 340)
    active_again = capture("refocused")
    assert active.crop(roi).tobytes() == active_again.crop(roi).tobytes(), "focus cycle retained stale shadow"
    (art / "result.json").write_text(json.dumps(dict(states=states, roi=roi), indent=2) + "\n")
    print("PASS: shape, layers, color, padding, disable/re-enable, transition, minimize/restore, opacity, focus")
finally:
    if remote_session:
        remote("Stop")
    if challenger:
        challenger.terminate()
        challenger.wait(timeout=5)
    if client:
        client.terminate()
        client.wait(timeout=5)
    config.unlink(missing_ok=True)
    control("config", "reload")
    log.close()
