#!/usr/bin/env python3
"""Correlate the compositor's dock layer state with the displayed pixels.

Runs against an existing prepared QEMU run whose prefix is synced. For each
fresh GDM login it records, every tick, one guest timestamp per sample: the
layer surfaces from `gnoblinctl layer list`, the native compositor and Bingux
shell process identities, and the dock layout from `binguxctl ipc shell
layoutSnapshot` (cached and refreshed off the sampling loop). The displayed
output is grabbed over VNC at the same tick. QMP screendump fails under
virtio-vga-gl ("no surface"), so VNC is the frame source.

After the observation window the check forces a stage repaint with
`gnoblinctl config reload` and grabs a reference frame. The dock band is
compared against that reference.

Outcomes per login, judged from the window end, before the repaint. The window
counts only from the new session's epoch, the later of the new compositor and
new shell start. Before that the old dock is still on screen and proves nothing.

- invisible-mapped: at window end the new dock layer is still mapped and its band
  does not match the repainted reference. This is the reported symptom and fails
  the run.
- not-mapped or incomplete: the new dock never mapped, or never matched the
  reference. These also fail the run.
- drawn-late: the dock matched more than --grace seconds after it mapped. This is
  startup latency, reported with draw_latency, and does not fail.
- drawn: the dock matched within --grace seconds of mapping.

Per-tick invisible_mapped flags in ticks.json are diagnostic only: a mapped dock
that stays unmatched for --grace seconds. A fade-in counts as unmatched until the
band reaches --threshold.
"""

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import socket
import struct
import subprocess
import time

from PIL import Image, ImageChops


ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts/qemu-e2e"
DOCK_NAMESPACE = "bingux-dock"
TOP_BAR_NAMESPACE = "bingux-top-bar"

# Runs inside the guest as one long-lived process. Each stdin line is a
# request: "restart" restarts GDM first, "sample" only samples. Every request
# gets one JSON line back. Process identification and PATH mirror
# session-startup.py so both checks observe the same processes.
GUEST_PROBE = r"""
import json, os, pwd, subprocess, sys, threading, time

uid = os.getuid()
home = pwd.getpwuid(uid).pw_dir
runtime = f"/run/user/{uid}"
env = os.environ.copy()
env.update({
    "HOME": home,
    "XDG_RUNTIME_DIR": runtime,
    "DBUS_SESSION_BUS_ADDRESS": f"unix:path={runtime}/bus",
    "PATH": os.pathsep.join([
        os.path.join(home, ".local/share/gnoblin/quickshell-fedora/bin"),
        os.path.join(home, ".local/share/bingux/bin"),
        "/usr/local/lib/gnoblin/bin",
        os.path.join(home, ".local/bin"),
        "/usr/local/bin", "/usr/bin", "/bin",
    ]),
})
ticks_per_second = os.sysconf("SC_CLK_TCK")
layout = {"cached": None, "busy": False}

def identity(pid):
    raw = open(f"/proc/{pid}/stat").read()
    fields = raw[raw.rfind(")") + 2:].split()
    return {"pid": pid, "start": int(fields[19]) / ticks_per_second}

def processes():
    compositors, shells = [], []
    for name in os.listdir("/proc"):
        if not name.isdigit():
            continue
        try:
            if os.stat(f"/proc/{name}").st_uid != uid:
                continue
            entry = identity(int(name))
            argv = [a for a in open(f"/proc/{name}/cmdline", "rb").read().decode(errors="replace").split("\0") if a]
            executable = os.path.basename(os.readlink(f"/proc/{name}/exe"))
        except (OSError, ValueError, IndexError):
            continue
        if executable == "gnoblin" and any(a.startswith("--gnoblin-runtime-fd=") for a in argv):
            compositors.append(entry)
        if executable in ("qs", "quickshell") and any(a.endswith("/bingux/shell") for a in argv):
            shells.append(entry)
    return compositors, shells

def run_json(*argv):
    try:
        out = subprocess.run(list(argv), capture_output=True, text=True, timeout=2, env=env)
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"ok": False, "error": str(error)}
    if out.returncode != 0:
        return {"ok": False, "stderr": out.stderr.strip()[-300:]}
    try:
        return {"ok": True, "json": json.loads(out.stdout)}
    except ValueError as error:
        return {"ok": False, "error": str(error)}

def refresh_layout(shell_pid):
    requested = time.monotonic()
    result = run_json("binguxctl", "ipc", "shell", "layoutSnapshot")
    entry = {"pid": shell_pid, "requested": requested, "answered": time.monotonic(), "ok": result["ok"]}
    if result["ok"]:
        dock = result["json"].get("dock") or {}
        entry.update(applications=len(dock.get("applications") or []),
                     pinned=len(dock.get("pinnedApps") or []),
                     order=len(dock.get("order") or []))
    else:
        entry.update({k: v for k, v in result.items() if k != "ok"})
    layout["cached"] = entry
    layout["busy"] = False

def sample():
    compositors, shells = processes()
    layers = run_json("gnoblinctl", "--timeout", "1", "--format", "json", "layer", "list")
    reply = {
        "guest": time.monotonic(),
        "compositors": compositors,
        "shells": shells,
        "layers": layers["json"]["layers"] if layers["ok"] else None,
        "layer_error": None if layers["ok"] else layers,
    }
    if len(shells) == 1 and not layout["busy"]:
        cached = layout["cached"]
        if not (cached and cached["pid"] == shells[0]["pid"] and cached["ok"]):
            layout["busy"] = True
            threading.Thread(target=refresh_layout, args=(shells[0]["pid"],), daemon=True).start()
    reply["layout"] = layout["cached"]
    return reply

for line in sys.stdin:
    restart = None
    if line.strip() == "restart":
        restart = time.monotonic()
        subprocess.Popen(["sudo", "-n", "systemctl", "restart", "gdm"],
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    reply = sample()
    reply["restart_requested"] = restart
    print(json.dumps(reply), flush=True)
"""


class Vnc:
    """Minimal RFB 3.8 client: raw 32 bpp frames from QEMU's VNC server, no auth."""

    def __init__(self, port):
        self.sock = socket.create_connection(("127.0.0.1", port), timeout=10)
        if not self.read(12).startswith(b"RFB 003."):
            raise RuntimeError("unexpected VNC greeting")
        self.sock.sendall(b"RFB 003.008\n")
        methods = self.read(self.read(1)[0])
        if 1 not in methods:
            raise RuntimeError(f"VNC server requires authentication: {list(methods)}")
        self.sock.sendall(b"\x01")
        if struct.unpack(">I", self.read(4))[0] != 0:
            raise RuntimeError("VNC security handshake failed")
        self.sock.sendall(b"\x01")  # shared desktop
        self.width, self.height = struct.unpack(">HH", self.read(4))
        self.read(16)
        self.read(struct.unpack(">I", self.read(4))[0])
        # 32 bpp, 24-bit colour, little-endian: a pixel arrives as B, G, R, pad.
        self.sock.sendall(struct.pack(">B3xBBBBHHHBBB3x", 0, 32, 24, 0, 1, 255, 255, 255, 16, 8, 0))
        self.sock.sendall(struct.pack(">BxHii", 2, 2, 0, -223))  # raw + DesktopSize
        self.pixels = bytearray(self.width * self.height * 4)

    def read(self, size):
        data = bytearray()
        while len(data) < size:
            chunk = self.sock.recv(size - len(data))
            if not chunk:
                raise RuntimeError("VNC connection closed")
            data += chunk
        return bytes(data)

    def frame(self):
        self.sock.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, self.width, self.height))
        if self.read(1)[0] != 0:
            raise RuntimeError("expected a FramebufferUpdate")
        self.read(1)
        for _ in range(struct.unpack(">H", self.read(2))[0]):
            x, y, w, h, encoding = struct.unpack(">HHHHi", self.read(12))
            if encoding == -223:
                self.width, self.height = w, h
                self.pixels = bytearray(w * h * 4)
                continue
            if encoding != 0:
                raise RuntimeError(f"unexpected VNC encoding {encoding}")
            data = self.read(w * h * 4)
            stride = self.width * 4
            for row in range(h):
                start = (y + row) * stride + x * 4
                self.pixels[start : start + w * 4] = data[row * w * 4 : (row + 1) * w * 4]
        return Image.frombuffer("RGB", (self.width, self.height), bytes(self.pixels), "raw", "BGRX", 0, 1).copy()

    def close(self):
        self.sock.close()


class Probe:
    def __init__(self, run):
        self.process = subprocess.Popen(
            [str(RUNNER), "guest", str(run), "--", "python3", "-u", "-c", GUEST_PROBE],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
        )

    def sample(self, request="sample"):
        sent = time.monotonic()
        self.process.stdin.write(request + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        received = time.monotonic()
        if not line:
            raise RuntimeError("guest probe exited")
        reply = json.loads(line)
        reply["host_sent"], reply["host_received"] = sent, received
        return reply

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=10)


def layer(sample, namespace):
    for item in sample.get("layers") or []:
        if item.get("namespace") == namespace:
            return item
    return None


def rect(item):
    geometry = item["geometry"]
    return (geometry["x"], geometry["y"], geometry["x"] + geometry["width"], geometry["y"] + geometry["height"])


def match_fraction(frame, reference, box, tolerance):
    """Fraction of pixels in box whose every channel is within tolerance of the reference."""
    difference = ImageChops.difference(frame.crop(box), reference.crop(box))
    within = None
    for band in difference.split():
        mask = band.point(lambda value: 255 if value <= tolerance else 0)
        within = mask if within is None else ImageChops.multiply(within, mask)
    return within.histogram()[255] / (within.width * within.height)


def utc_text():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")


def identities(items):
    return {(item["pid"], item["start"]) for item in items}


def new_starts(samples, key, old):
    return sorted({item["start"] for s in samples for item in s[key] if (item["pid"], item["start"]) not in old})


def observe_login(args, probe, vnc, login_dir):
    login_dir.mkdir(parents=True)
    baseline = probe.sample()
    old = {"compositors": identities(baseline["compositors"]), "shells": identities(baseline["shells"])}

    ticks = []
    started = time.monotonic()
    while True:
        tick_started = time.monotonic()
        request = "restart" if not ticks else "sample"
        sample = probe.sample(request)
        image = vnc.frame()
        ticks.append({"index": len(ticks), "wall_utc": utc_text(), "probe": sample, "image": image})
        if time.monotonic() - started >= args.seconds:
            break
        time.sleep(max(0.0, args.interval - (time.monotonic() - tick_started)))

    # Force a full stage repaint, then take the reference frame from the repainted output.
    subprocess.run([str(RUNNER), "guest", str(args.run), "--", "gnoblinctl", "--timeout", "5", "config", "reload"],
                   check=True, capture_output=True, text=True)
    time.sleep(args.repaint_wait)
    reference_sample = probe.sample()
    reference = vnc.frame()
    reference.save(login_dir / "reference.png")

    dock_ref = layer(reference_sample, DOCK_NAMESPACE)
    bar_ref = layer(reference_sample, TOP_BAR_NAMESPACE)
    result = {
        "restart_requested_guest": ticks[0]["probe"]["restart_requested"],
        "reference_dock": dock_ref,
        "reference_top_bar_mapped": bool(bar_ref and bar_ref.get("mapped")),
        "new_compositor_start_guest": new_starts([t["probe"] for t in ticks], "compositors", old["compositors"])[:1] or None,
        "new_shell_start_guest": new_starts([t["probe"] for t in ticks], "shells", old["shells"])[:1] or None,
    }
    if not dock_ref or not dock_ref.get("mapped"):
        result.update(outcome="incomplete", reason="dock not mapped in the repaint reference")
        write_login(login_dir, ticks, result, reference)
        return result

    new_compositor = result["new_compositor_start_guest"]
    new_shell = result["new_shell_start_guest"]
    if not new_compositor or not new_shell:
        result.update(outcome="incomplete", reason="no new compositor and shell start observed after restart")
        write_login(login_dir, ticks, result, reference)
        return result
    # The old session's dock stays on screen until its compositor exits. Only ticks after
    # the new session starts can show the new dock, so the window starts there.
    epoch = max(new_compositor[0], new_shell[0])
    dock_box = rect(dock_ref)
    first_mapped = None
    span_start = None
    for item in ticks:
        sample = item["probe"]
        in_session = sample["guest"] >= epoch
        dock = layer(sample, DOCK_NAMESPACE)
        mapped = bool(dock and dock.get("mapped")) and in_session
        if mapped and span_start is None:
            span_start = sample["guest"]
            first_mapped = first_mapped or span_start
        if not mapped:
            span_start = None
        match = match_fraction(item["image"], reference, dock_box, args.tolerance)
        mapped_for = None if span_start is None else round(sample["guest"] - span_start, 3)
        item.update(
            in_session=in_session,
            dock_mapped=mapped,
            dock_geometry=rect(dock) if mapped else None,
            dock_match=round(match, 4),
            dock_shown=mapped and match >= args.threshold,
            mapped_for=mapped_for,
        )
        item["invisible_mapped"] = bool(mapped and mapped_for >= args.grace and not item["dock_shown"])

    invisible = [t["index"] for t in ticks if t["invisible_mapped"]]
    first_shown = next((t for t in ticks if t["dock_shown"]), None)
    last = ticks[-1]
    draw_latency = None if first_shown is None or first_mapped is None else round(first_shown["probe"]["guest"] - first_mapped, 3)
    result.update(
        session_epoch_guest=epoch,
        first_dock_mapped_guest=first_mapped,
        first_dock_shown_guest=None if first_shown is None else first_shown["probe"]["guest"],
        draw_latency=draw_latency,
        invisible_mapped_frames=invisible,
        invisible_mapped_count=len(invisible),
        window_end_mapped=last["dock_mapped"],
        window_end_shown=last["dock_shown"],
    )
    if last["dock_mapped"] and not last["dock_shown"]:
        result.update(outcome="invisible-mapped", reason="dock still mapped and undrawn at window end, before repaint")
    elif first_mapped is None:
        result.update(outcome="not-mapped", reason="the new dock never reported mapped")
    elif first_shown is None:
        result.update(outcome="incomplete", reason="dock mapped but never matched the repaint reference")
    elif draw_latency > args.grace:
        result["outcome"] = "drawn-late"
    else:
        result["outcome"] = "drawn"
    write_login(login_dir, ticks, result, reference)
    return result


def write_login(login_dir, ticks, result, reference):
    frames = login_dir / "frames"
    frames.mkdir(exist_ok=True)
    records = []
    for item in ticks:
        name = f"{item['index']:04d}.png"
        if item.get("invisible_mapped") or item.get("dock_shown") is False or item["index"] % 20 == 0:
            item["image"].save(frames / name)
            item["png"] = f"frames/{name}"
        records.append({k: v for k, v in item.items() if k != "image"})
    (login_dir / "ticks.json").write_text(json.dumps(records, indent=1, default=str) + "\n")
    (login_dir / "result.json").write_text(json.dumps(result, indent=1, default=str) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("run", type=Path, help="prepared QEMU run directory with a synced prefix")
    parser.add_argument("--logins", type=int, default=5, help="fresh GDM logins (default: 5)")
    parser.add_argument("--seconds", type=float, default=60, help="observation per login (default: 60)")
    parser.add_argument("--interval", type=float, default=0.5, help="seconds between ticks (default: 0.5)")
    parser.add_argument("--grace", type=float, default=2.0, help="seconds a mapped dock may take to be drawn (default: 2)")
    parser.add_argument("--threshold", type=float, default=0.9, help="dock band match fraction (default: 0.9)")
    parser.add_argument("--tolerance", type=int, default=24, help="per-channel pixel tolerance (default: 24)")
    parser.add_argument("--repaint-wait", type=float, default=2.0, help="seconds after config reload (default: 2)")
    parser.add_argument("--output", type=Path, help="new artifact directory (default: dated directory in RUN)")
    args = parser.parse_args()
    if args.logins < 1 or args.seconds <= 0 or args.interval <= 0:
        parser.error("--logins, --seconds and --interval must be positive")

    args.run = args.run.resolve()
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    output = (args.output or args.run / f"dock-first-paint-{stamp}").resolve()
    output.mkdir(parents=True, exist_ok=False)
    vnc = Vnc(int((args.run / "vnc-port").read_text()))
    probe = Probe(args.run)
    summary = {
        "schema": 1,
        "run": str(args.run),
        "prefix": (args.run / "synced-prefix").read_text().strip(),
        "renderer": (args.run / "renderer").read_text().strip(),
        "settings": {k: str(v) for k, v in vars(args).items() if k not in ("run", "output")},
        "started_utc": utc_text(),
        "logins": [],
    }
    failed = False
    try:
        for login in range(1, args.logins + 1):
            login_dir = output / f"login-{login:02d}"
            result = observe_login(args, probe, vnc, login_dir)
            summary["logins"].append({"login": login, "dir": str(login_dir), **result})
            failed |= result["outcome"] in ("invisible-mapped", "not-mapped", "incomplete")
            print(f"login {login}: {result['outcome']}; invisible-mapped frames={result.get('invisible_mapped_count', '-')}",
                  flush=True)
    finally:
        summary["finished_utc"] = utc_text()
        (output / "summary.json").write_text(json.dumps(summary, indent=1, default=str) + "\n")
        probe.close()
        vnc.close()
    print(f"summary: {output / 'summary.json'}")
    raise SystemExit(1 if failed else 0)


if __name__ == "__main__":
    main()
