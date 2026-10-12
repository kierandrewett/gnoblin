#!/usr/bin/env python3
"""Measure real framebuffer-blur allocations and pixels in a private devkit.

Requires Quickshell, grim, Pillow, and ptrace access to the owned compositor.
Use --reference before changing Mutter, then --compare against that directory.
GDB distorts timing; this fixture measures allocation counts, not performance.
"""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import time

from PIL import Image, ImageChops, ImageStat

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("artifacts", type=Path)
parser.add_argument("--prefix", type=Path, default=Path("/usr/local/lib/gnoblin"))
parser.add_argument("--quickshell", default="qs")
parser.add_argument("--reference", action="store_true")
parser.add_argument("--compare", type=Path)
parser.add_argument("--capture-failure-marker", type=Path, help="Gate the test-only capture failure preload shim")
args = parser.parse_args()
assert platform.machine() == "x86_64", "GDB allocation arguments use the x86-64 calling convention"
art = args.artifacts.resolve()
art.mkdir(parents=True, exist_ok=False)
root = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin"
assert os.environ["WAYLAND_DISPLAY"].startswith("gnoblin-devkit-"), "private devkit required"
assert not (root / "init.lua").exists(), "use an empty disposable config"
root.mkdir(parents=True, exist_ok=True)
seat = (Path(os.environ["XDG_RUNTIME_DIR"]) / "gnoblin-blur-proof.lock").open("w")
fcntl.flock(seat, fcntl.LOCK_EX | fcntl.LOCK_NB)
ctl = str(args.prefix / "bin/gnoblinctl")


def command(*argv):
    return subprocess.check_output(argv, text=True, timeout=20)


def identity():
    pending = [int(os.environ["GNOBLIN_DEVKIT_HOST_PID"])]
    seen = set()
    found = []
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        proc = Path("/proc") / str(pid)
        try:
            for task in (proc / "task").iterdir():
                pending.extend(int(p) for p in (task / "children").read_text().split())
            libraries = {
                line.split()[-1]
                for line in (proc / "maps").read_text().splitlines()
                if "libmutter-51.so" in line or "libmutter-clutter-51.so" in line
            }
            argv = (proc / "cmdline").read_bytes().split(b"\0")
            if not libraries or not any(a.startswith(b"--gnoblin-runtime-fd=") for a in argv):
                continue
            executable = str((proc / "exe").resolve())
            assert executable == str(args.prefix / "bin/gnoblin"), executable
            paths = [proc / "exe", *map(Path, libraries)]
            hashes = {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
            assert hashes[str(proc / "exe")] == hashlib.sha256((args.prefix / "bin/gnoblin").read_bytes()).hexdigest()
            assert all(p.startswith(str(args.prefix / "lib64")) for p in libraries)
            found.append(dict(pid=pid, executable=executable, hashes=hashes))
        except (FileNotFoundError, ProcessLookupError):
            continue
    assert len(found) == 1, found
    (art / "identity.json").write_text(json.dumps(found, indent=2))
    return found[0]["pid"]


def configure(radius):
    (root / "init.lua").write_text(
        'gnoblin.window_rule { match = {layer = "^blurcachecard$"}, '
        f'blur = {radius}, animation = "none", '
        'corners = {mode = "off", shadow = false} }\n'
    )
    command(ctl, "config", "reload")
    time.sleep(0.35)


def screen(name):
    path = art / (name + ".png")
    subprocess.run(["grim", str(path)], check=True, timeout=20)
    return Image.open(path).convert("RGB")


def counts():
    return json.loads((art / "counts-live.json").read_text())


qml = art / "shell.qml"
qml.write_text("""import QtQuick
import Quickshell
import Quickshell.Wayland
import Quickshell.Io
ShellRoot {
 id: root
 property int phase: 0
 property int cardWidth: 400
 property real alpha: 1
 property bool shown: true
 IpcHandler {
  target: "fixture"
  function step(value: int): void { root.phase = value }
  function resize(value: int): void { root.cardWidth = value }
  function fade(value: real): void { root.alpha = value }
  function shown(value: int): void { root.shown = value !== 0 }
 }
 PanelWindow {
  anchors { top: true; bottom: true; left: true; right: true }
  WlrLayershell.layer: WlrLayer.Background
  WlrLayershell.namespace: "blurcachebackground"
  color: root.phase % 2 === 0 ? "#204060" : "#603020"
  Repeater { model: 60
   Rectangle { required property int index; x: index * 20; width: 10; height: 600
    color: root.phase % 2 === 0 ? "white" : "#b0c0d0" }
  }
 }
 PanelWindow {
  visible: root.shown
  anchors { top: true; left: true }
  implicitWidth: root.cardWidth; implicitHeight: 300
  WlrLayershell.layer: WlrLayer.Top
  WlrLayershell.namespace: "blurcachecard"
  color: "transparent"
  Rectangle { anchors.fill: parent; radius: 20; color: "#80303030"; opacity: root.alpha }
 }
}
""")
# Register before creating surfaces so all cache generations are tracked.
gdb_script = art / "count.gdb"
gdb_script.write_text(
    """set pagination off
set confirm off
set debuginfod enabled off
set print thread-events off
python
import gdb, json
from collections import Counter
from pathlib import Path
art = Path("""
    + repr(str(art))
    + """)
totals = Counter()
textures = {}
sizes = Counter()
def save():
    result = dict(counts=dict(totals), live_textures={str(k):v for k,v in textures.items()},
                  sizes=[dict(size=list(k),count=v) for k,v in sizes.items()])
    temporary = art / 'counts-next.json'
    temporary.write_text(json.dumps(result))
    temporary.replace(art / 'counts-live.json')
class Count(gdb.Breakpoint):
    def __init__(self, name):
        self.key = name
        super().__init__('*' + name, internal=True)
    def stop(self):
        totals[self.key] += 1
        save()
        return False
class Returned(gdb.FinishBreakpoint):
    def __init__(self, width, height):
        self.size = [width, height]
        super().__init__(gdb.newest_frame(), internal=True)
    def stop(self):
        address = int(gdb.parse_and_eval('$rax'))
        if address:
            assert address not in textures
            textures[address] = self.size
            sizes[tuple(self.size)] += 1
            totals['texture_new'] += 1
            save()
        return False
class TextureNew(gdb.Breakpoint):
    def stop(self):
        frame = gdb.newest_frame().older()
        while frame:
            if 'blur' in (frame.name() or ''):
                Returned(int(gdb.parse_and_eval('$rsi')), int(gdb.parse_and_eval('$rdx')))
                break
            frame = frame.older()
        return False
class TextureDispose(gdb.Breakpoint):
    def stop(self):
        address = int(gdb.parse_and_eval('$rdi'))
        if address in textures:
            textures.pop(address)
            totals['texture_dispose'] += 1
            save()
        return False
for symbol in ['clutter_blur_node_new_from_framebuffer', 'clutter_blur_node_finalize',
               'clutter_blur_cache_new', 'clutter_blur_cache_finalize', 'clutter_blur_free']:
    Count(symbol)
try:
    gdb.parse_and_eval('clutter_blur_node_new_from_framebuffer_cached')
except gdb.error:
    pass
else:
    Count('clutter_blur_node_new_from_framebuffer_cached')
TextureNew('*cogl_texture_2d_new_with_size', internal=True)
TextureDispose('*cogl_texture_2d_gl_dispose', internal=True)
save()
(art / 'ready').write_text('ready')
end
continue
python
save()
end
detach
quit
"""
)
client = debugger = None
report = {"timing_is_debugger_distorted": True, "phases": {}}


def ipc(action, value):
    command(args.quickshell, "-p", str(qml), "ipc", "call", "fixture", action, str(value))
    time.sleep(0.25)


try:
    pid = identity()
    configure(0)
    with (art / "gdb.log").open("w") as debug_log, (art / "quickshell.log").open("w") as client_log:
        debugger = subprocess.Popen(
            ["gdb", "-q", "-nx", "-batch", "-p", str(pid), "-x", str(gdb_script)],
            stdout=debug_log,
            stderr=subprocess.STDOUT,
        )
        for _ in range(150):
            if (art / "ready").exists():
                break
            assert debugger.poll() is None, (art / "gdb.log").read_text()
            time.sleep(0.1)
        assert (art / "ready").exists(), "debugger setup timed out"
        client = subprocess.Popen([args.quickshell, "-p", str(qml)], stdout=client_log, stderr=client_log)
        time.sleep(1)
        assert client.poll() is None, (art / "quickshell.log").read_text()
        unblurred = screen("unblurred")
        configure(24)
        blurred = screen("blurred")
        roi = (30, 40, 370, 260)
        contrast = [ImageStat.Stat(image.crop(roi)).stddev[0] for image in [unblurred, blurred]]
        assert contrast[1] < contrast[0] * 0.95, contrast
        if args.capture_failure_marker:
            marker = args.capture_failure_marker
            assert not marker.exists(), "failure marker must be absent before the test"
            try:
                marker.touch(exist_ok=False)
                ipc("step", 2)
                failed = screen("capture-failed")
            finally:
                marker.unlink(missing_ok=True)
            difference = ImageChops.difference(failed, unblurred)
            max_error = max(v[1] for v in difference.getextrema())
            report["capture_failure"] = {"max_pixel_error": max_error}
            (art / "failure-result.json").write_text(json.dumps(report["capture_failure"]) + "\n")
            assert max_error <= 1, f"failed capture painted stale cached blur: pixel difference {max_error}"
            ipc("step", 0)
            recovered = screen("capture-recovered")
            recovery_error = max(v[1] for v in ImageChops.difference(recovered, blurred).getextrema())
            assert recovery_error <= 1, "blur did not recover after capture failure"
            report["capture_failure"]["recovery_pixel_error"] = recovery_error
        for cycle in range(12):
            ipc("step", cycle + 1)
        even = screen("even")
        ipc("step", 13)
        odd = screen("odd")
        assert ImageStat.Stat(ImageChops.difference(even.crop(roi), odd.crop(roi))).mean[0] > 5, "stale backdrop"
        report["phases"]["fixed"] = counts()
        for alpha in [0.5, 0.0, 1.0]:
            ipc("fade", alpha)
            screen(f"alpha-{alpha}")
        report["phases"]["alpha"] = counts()
        # Exercise a small resize and larger changes in the capture bounds.
        for width in [401, 460, 400]:
            ipc("resize", width)
            screen(f"width-{width}")
        for radius in [12, 24, 0, 24]:
            configure(radius)
            screen(f"radius-{radius}")
            if radius == 0:
                assert not counts()["live_textures"], "disabled blur retained targets"
        ipc("shown", 0)
        time.sleep(0.5)
        report["phases"]["hidden"] = counts()
        ipc("shown", 1)
        screen("shown-again")
        client.terminate()
        client.wait(timeout=5)
        time.sleep(1)
        debugger.send_signal(signal.SIGINT)
        debugger.wait(timeout=15)
        assert debugger.returncode == 0, (art / "gdb.log").read_text()
        report["final"] = counts()
        report["contrast"] = contrast
        if args.compare:
            differences = {}
            for path in sorted(art.glob("*.png")):
                reference = Image.open(args.compare / path.name).convert("RGB")
                actual = Image.open(path).convert("RGB")
                assert actual.size == reference.size, path.name
                diff = ImageChops.difference(reference, actual)
                differences[path.name] = {
                    "max": max(v[1] for v in diff.getextrema()),
                    "mean": ImageStat.Stat(diff).mean,
                }
                assert differences[path.name]["max"] <= 1, (path.name, differences[path.name])
            report["pixel_differences"] = differences
        final = report["final"]
        assert not final["live_textures"], final
        assert final["counts"]["texture_new"] == final["counts"]["texture_dispose"], final
        assert final["counts"]["clutter_blur_cache_new"] == final["counts"]["clutter_blur_cache_finalize"], final
        fixed = report["phases"]["fixed"]["counts"].get("texture_new", 0)
        report["fixed_geometry_texture_allocations"] = fixed
        report["status"] = "reference" if args.reference else "measured"
        (art / "result.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report), flush=True)
        if not args.reference:
            assert report["phases"]["fixed"]["counts"]["clutter_blur_node_finalize"] >= 16, "insufficient redraws"
            assert fixed <= 4, f"fixed geometry allocated {fixed} textures, expected one four-target generation"
            assert report["phases"]["alpha"]["counts"]["texture_new"] == fixed, "alpha changes rebuilt targets"
            assert not report["phases"]["hidden"]["live_textures"], "hidden surface retained targets"
            report["status"] = "passed"
            (art / "result.json").write_text(json.dumps(report, indent=2) + "\n")
finally:
    if args.capture_failure_marker:
        args.capture_failure_marker.unlink(missing_ok=True)
    if debugger and debugger.poll() is None:
        debugger.send_signal(signal.SIGINT)
        try:
            debugger.wait(timeout=10)
        except subprocess.TimeoutExpired:
            debugger.terminate()
            debugger.wait(timeout=5)
    if client and client.poll() is None:
        client.terminate()
        client.wait(timeout=5)
    shutil.copyfile(os.environ["GNOBLIN_DEVKIT_RUNTIME_LOG"], art / "runtime.log")
