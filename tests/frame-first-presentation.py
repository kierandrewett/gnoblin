#!/usr/bin/env python3
"""Pixel regression for first-map readiness; run inside a disposable devkit.

GNOBLIN_FRAME_RENDERER names a frame renderer (the Cairo sample or bingux-frame). No installed session is changed.
Artifacts include every sampled screenshot, window state and renderer wire log.
"""

import argparse
import json
import os
from pathlib import Path
import shlex
import signal
import subprocess
import time

from PIL import Image

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("artifacts", type=Path)
parser.add_argument(
    "--config-drop-in", type=Path, help="relative path loaded by a copied config, e.g. conf.d/99-frame-proof.lua"
)
args = parser.parse_args()
art = args.artifacts.resolve()
art.mkdir(parents=True, exist_ok=True)
assert not any(art.iterdir()), "use a fresh artifact directory"
renderer = Path(os.environ["GNOBLIN_FRAME_RENDERER"]).resolve()
assert renderer.is_file()
bind_no_buffer = Path(os.environ["GNOBLIN_FRAME_BIND_NO_BUFFER"]).resolve()
assert bind_no_buffer.is_file()
assert os.environ["WAYLAND_DISPLAY"].startswith("gnoblin-devkit-"), "use a disposable devkit"
ctl = str(Path(os.environ["GNOBLIN_PREFIX"]) / "bin/gnoblinctl")
config_root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
config_relative = args.config_drop_in or Path("init.lua")
assert not config_relative.is_absolute() and ".." not in config_relative.parts
config = config_root / config_relative
assert not config.exists(), "fixture config path must be unused"
config.parent.mkdir(parents=True, exist_ok=True)
app = art / "app.py"
app.write_text("""import gi,sys,pathlib
gi.require_version("Gtk", "4.0")
from gi.repository import Gtk,GLib
Gtk.init()
w=Gtk.Window(title="Frame first proof"); w.set_default_size(600,360)
w.set_child(Gtk.Label(label="FRAME BODY READY"))
css=Gtk.CssProvider(); css.load_from_string("window {background-color:#f6f5f4;}")
Gtk.StyleContext.add_provider_for_display(w.get_display(),css,800)
loop=GLib.MainLoop()
w.connect("close-request",lambda *_:(loop.quit(),False)[1])
w.present()
command=pathlib.Path(sys.argv[1])
def poll():
 if command.exists():
  command.unlink(); w.set_default_size(800,360)
 return True
GLib.timeout_add(20,poll)
loop.run()
""")
identity = []
for process in Path("/proc").iterdir():
    if not process.name.isdigit():
        continue
    try:
        argv = (process / "cmdline").read_bytes().split(b"\0")
        if (
            any(a.startswith(b"--gnoblin-runtime-fd=") for a in argv)
            and argv[0] == str(Path(os.environ["GNOBLIN_PREFIX"]) / "bin/gnoblin").encode()
        ):
            identity.append(
                dict(
                    pid=int(process.name),
                    executable=str((process / "exe").resolve()),
                    libraries=sorted(
                        {
                            line.split()[-1]
                            for line in (process / "maps").read_text().splitlines()
                            if "libmutter-51.so" in line
                        }
                    ),
                )
            )
    except (OSError, PermissionError):
        pass
(art / "runtime-identity.json").write_text(json.dumps(identity, indent=2))
assert identity, "private compositor executable was not used; check compiled compositor path"
assert all(
    str(Path(os.environ["GNOBLIN_PREFIX"]) / "lib64") in lib for item in identity for lib in item["libraries"]
), "private libmutter was not used"
rows = []
processes = []
results = []


def control(*argv):
    return subprocess.run([ctl, *argv], capture_output=True, text=True, check=True).stdout


def windows():
    return json.loads(control("--json", "window", "list"))["windows"]


def capture(label, duration, start):
    samples = []
    until = time.monotonic() + duration
    while time.monotonic() < until:
        i = len(rows)
        shot = art / f"{i:04d}-{label}.png"
        begin = time.monotonic() - start
        subprocess.run(["grim", str(shot)], check=True, capture_output=True)
        window = next((w for w in windows() if w["title"] == "Frame first proof"), None)
        row = dict(case=label, start=begin, end=time.monotonic() - start, shot=str(shot), window=window)
        if window:
            f = window["frame"]
            im = Image.open(shot).convert("RGB")
            x, y = f["x"] + 280, f["y"]
            if 0 <= x < im.width and 0 <= y + 100 < im.height:
                row["header"] = list(im.getpixel((x, y + 16)))
                row["body"] = list(im.getpixel((x, y + 100)))
        samples.append(row)
        rows.append(row)
    (art / "timeline.json").write_text(json.dumps(rows, indent=2))
    return samples


def visible(samples):
    return [r for r in samples if r.get("body") == [246, 245, 244]]


def check(name, passed, detail):
    results.append(dict(name=name, passed=bool(passed), detail=detail))
    print(("PASS " if passed else "FAIL ") + name + ": " + str(detail), flush=True)
    (art / "results.json").write_text(json.dumps(results, indent=2))


def setup(label, command):
    wrapper = art / (label + "-renderer.sh")
    pidfile = art / (label + "-renderer.pid")
    wrapper.write_text("#!/bin/sh\necho $$ > " + shlex.quote(str(pidfile)) + "\n" + command + "\n")
    wrapper.chmod(0o755)
    # Different service and argv on every run: reload must not reuse the warm child.
    config.write_text(
        "gnoblin.configure {frame_renderers = {[" + json.dumps(label) + "] = {" + json.dumps(str(wrapper)) + "}}}\n"
        'gnoblin.window_rule {match={title="^Frame first proof$"},frame={mode="prefer-server",renderer='
        + json.dumps(label)
        + ',extents={36,0,0,0}},animation="none",corners={mode="off",shadow=false}}\n'
    )
    control("config", "reload")
    commandfile = art / (label + "-resize")
    commandfile.unlink(missing_ok=True)
    log = (art / (label + "-app.log")).open("w")
    client = subprocess.Popen(
        ["python3", str(app), str(commandfile)],
        env={**os.environ, "GTK_CSD": "0", "GDK_BACKEND": "wayland"},
        stdout=log,
        stderr=log,
    )
    log.close()
    processes.append(client)
    # Let app startup finish before setting the observation interval. On a
    # memory-constrained VM, loading GTK itself can exceed several seconds.
    deadline = time.monotonic() + 10
    while not any(w["title"] == "Frame first proof" for w in windows()):
        assert client.poll() is None, "app exited before creating a window"
        assert time.monotonic() < deadline, "app window was not created"
        time.sleep(0.02)
    return client, pidfile, commandfile, time.monotonic()


def stop(client):
    if client.poll() is None:
        client.terminate()
        client.wait(timeout=5)


def wait_for(path, detail, timeout=5):
    deadline = time.monotonic() + timeout
    while not path.exists():
        assert time.monotonic() < deadline, detail
        time.sleep(0.02)


try:
    for label, delay in [("cold", 0), ("delayed", 0.25)]:
        wirelog = art / (label + "-renderer.log")
        command = f"sleep {delay}; exec env WAYLAND_DEBUG=1 {shlex.quote(str(renderer))} --compact 2>{shlex.quote(str(wirelog))}"
        client, pidfile, resize, start = setup(label, command)
        samples = capture(label, 4.5, start)
        app_pixels = visible(samples)
        bad = [r for r in app_pixels if r["header"] == [36, 36, 36] or r["header"] == r["body"]]
        check(
            label + " first presentation",
            app_pixels and not bad,
            dict(visible=len(app_pixels), wrong_header=len(bad), first=app_pixels[0]["start"] if app_pixels else None),
        )
        if label == "delayed" and app_pixels:
            pid = int(pidfile.read_text())
            os.kill(pid, signal.SIGSTOP)
            try:
                resize.touch()
                resized = capture("paused-resize", 1.6, start)
                painted = visible(resized)
                check(
                    "resize retains custom frame beyond repaint timeout",
                    painted
                    and any(r["window"]["frame"]["width"] == 800 for r in painted)
                    and all(r["header"] != [36, 36, 36] for r in painted),
                    dict(visible=len(painted), widths=sorted({r["window"]["frame"]["width"] for r in painted})),
                )
            finally:
                os.kill(pid, signal.SIGCONT)
            capture("resize-recovered", 0.8, start)
            os.kill(pid, signal.SIGKILL)
            crashed = visible(capture("crash-after-presentation", 0.55, start))
            check(
                "renderer crash shows built-in frame", any(r["header"] == [36, 36, 36] for r in crashed), len(crashed)
            )
        stop(client)
        time.sleep(0.15)
    for label, command in [("exit-before-bind", "exit 1"), ("hung", "exec sleep 30")]:
        client, _, _, start = setup(label, command)
        samples = visible(capture(label, 4.0, start))
        check(
            label + " bounded fallback",
            samples and samples[0]["start"] < 3.5 and all(r["header"] == [36, 36, 36] for r in samples),
            dict(first=samples[0]["start"] if samples else None, visible=len(samples)),
        )
        stop(client)
    bind_ready = art / "bind-no-buffer.ready"
    client, pidfile, _, start = setup(
        "bind-no-buffer", f"exec {shlex.quote(str(bind_no_buffer))} --ready {shlex.quote(str(bind_ready))}"
    )
    wait_for(bind_ready, "renderer did not attach and acknowledge a frame configure")
    samples = visible(capture("bind-no-buffer", 4.0, start))
    try:
        os.kill(int(pidfile.read_text()), 0)
        renderer_alive = True
    except OSError:
        renderer_alive = False
    check(
        "bound renderer without a buffer has bounded fallback",
        samples and samples[0]["start"] < 3.5 and all(r["header"] == [36, 36, 36] for r in samples) and renderer_alive,
        dict(first=samples[0]["start"] if samples else None, visible=len(samples), renderer_alive=renderer_alive),
    )
    stop(client)
    client, pidfile, _, start = setup("close-before-buffer", "exec sleep 30")
    # Close once the app has committed and the compositor knows its frame.
    deadline = time.monotonic() + 2
    while not pidfile.exists() and time.monotonic() < deadline:
        time.sleep(0.02)
    assert pidfile.exists(), "renderer was not launched before close"
    samples = capture("before-close", 0.1, start)
    window = next(w for w in windows() if w["title"] == "Frame first proof")
    control("window", "close", window["id"])
    samples += capture("after-close", 2.5, start)
    check(
        "close before first buffer never presents or strands window",
        client.poll() is not None
        and not visible(samples)
        and not any(w["title"] == "Frame first proof" for w in windows()),
        dict(exit=client.returncode, samples=len(samples)),
    )
    client, pidfile, _, start = setup("minimize-before-buffer", "exec sleep 30")
    deadline = time.monotonic() + 5
    while not pidfile.exists() and time.monotonic() < deadline:
        time.sleep(0.02)
    assert pidfile.exists(), "renderer was not launched before minimize"
    window = next(w for w in windows() if w["title"] == "Frame first proof")
    control("window", "minimize", window["id"])
    minimized = capture("minimized-before-buffer", 2.4, start)
    check("minimize cancels pending map without presenting", not visible(minimized), len(minimized))
    control("window", "unminimize", window["id"])
    restored = visible(capture("restored-after-cancel", 1, start))
    check(
        "restore after pending-map cancellation presents fallback",
        restored and all(r["header"] == [36, 36, 36] for r in restored),
        len(restored),
    )
    stop(client)
    control("ping")
finally:
    for client in processes:
        stop(client)
    config.unlink(missing_ok=True)
    control("config", "reload")
    for log in (Path(os.environ["XDG_STATE_HOME"]) / "gnoblin").glob("*.log"):
        (art / log.name).write_text(log.read_text())
    runtime_log = os.environ.get("GNOBLIN_DEVKIT_RUNTIME_LOG")
    if runtime_log:
        (art / "runtime.log").write_text(Path(runtime_log).read_text())

raise SystemExit(0 if all(r["passed"] for r in results) else 1)
