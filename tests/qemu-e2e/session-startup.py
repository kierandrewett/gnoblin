#!/usr/bin/env python3
"""Record bounded GDM-to-session evidence from an existing disposable QEMU run.

This is a host-operated evidence harness.  It restarts GDM only inside the VM,
then writes the QMP frames and guest-side observations needed to distinguish
the greeter, wallpaper, and shell in later analysis.  It does not click GDM or
claim credential-entry timing.
"""

import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import socket
import subprocess
import time

from PIL import Image


ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts/qemu-e2e"


def utc_now():
    return datetime.now(timezone.utc)


def utc_text(value):
    return value.isoformat(timespec="microseconds").replace("+00:00", "Z")


def parse_sample(value):
    try:
        name, x, y = value.split(":", 2)
        return name, int(x), int(y)
    except ValueError as error:
        raise argparse.ArgumentTypeError("samples use NAME:X:Y") from error


def existing_run_metadata(run):
    """Keep the small run descriptors that identify the VM under test."""
    records = {}
    for name in ("renderer", "metadata.json", "build.json", "qemu-command.txt"):
        path = run / name
        if path.is_file() and path.stat().st_size <= 1_000_000:
            records[name] = path.read_text(errors="replace")
    return records


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("run", type=Path, help="existing QEMU run directory")
    parser.add_argument("--cycles", type=int, default=5, help="fresh GDM starts to record (default: 5)")
    parser.add_argument("--seconds", type=float, default=10, help="capture duration per cycle, at most 10")
    parser.add_argument("--hz", type=float, default=20, help="QMP capture rate per cycle, at most 20")
    parser.add_argument(
        "--sample",
        action="append",
        type=parse_sample,
        default=[],
        metavar="NAME:X:Y",
        help="record a named RGB sample in every frame; no state is inferred",
    )
    parser.add_argument("--output", type=Path, help="artifact directory (default: a dated directory in RUN)")
    args = parser.parse_args()
    if not 1 <= args.cycles:
        parser.error("--cycles must be positive")
    if not 0 < args.seconds <= 10:
        parser.error("--seconds must be in (0, 10]")
    if not 0 < args.hz <= 20:
        parser.error("--hz must be in (0, 20]")

    run = args.run.resolve()
    qmp_path = run / "qmp.sock"
    if not qmp_path.is_socket():
        parser.error(f"missing QMP socket: {qmp_path}")
    output = (args.output or run / ("session-startup-" + utc_now().strftime("%Y%m%dT%H%M%SZ"))).resolve()
    output.mkdir(parents=True, exist_ok=False)

    def guest_capture(*command, timeout=20):
        result = subprocess.run(
            [str(RUNNER), "guest", str(run), "--", *command], text=True, capture_output=True, timeout=timeout
        )
        return {
            "argv": list(command),
            "returncode": result.returncode,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }

    sock = socket.socket(socket.AF_UNIX)
    sock.settimeout(10)
    sock.connect(str(qmp_path))
    stream = sock.makefile("rw")
    greeting = json.loads(stream.readline())

    def qmp(command, **arguments):
        stream.write(json.dumps({"execute": command, "arguments": arguments}) + "\n")
        stream.flush()
        while True:
            reply = json.loads(stream.readline())
            if "error" in reply:
                raise RuntimeError(reply)
            if "return" in reply:
                return reply["return"]

    try:
        qmp("qmp_capabilities")
        metadata = {
            "schema": 1,
            "purpose": "bounded GDM restart capture; not credential-click timing",
            "run": str(run),
            "existing_run_metadata": existing_run_metadata(run),
            "qmp_greeting": greeting,
            "cycles": args.cycles,
            "capture_seconds": args.seconds,
            "capture_hz": args.hz,
            "samples": [{"name": name, "x": x, "y": y} for name, x, y in args.sample],
            "wallpaper_hint": "/home/kieran/Documents/wall2.png",
            "started_utc": utc_text(utc_now()),
            "started_host_monotonic_ns": time.monotonic_ns(),
        }
        (output / "metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")

        for cycle in range(1, args.cycles + 1):
            cycle_dir = output / f"cycle-{cycle:02d}"
            frames_dir = cycle_dir / "frames"
            frames_dir.mkdir(parents=True)
            start_utc = utc_now()
            start_mono = time.monotonic_ns()
            restart = subprocess.Popen(
                [str(RUNNER), "guest", str(run), "--", "sudo", "-n", "systemctl", "restart", "gdm"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            timeline = [{"event": "restart-requested", "utc": utc_text(start_utc), "host_monotonic_ns": start_mono}]
            interval = 1 / args.hz
            deadline = time.monotonic() + args.seconds
            frame = 0
            restart_returned = None
            while time.monotonic() < deadline:
                if restart_returned is None and restart.poll() is not None:
                    restart_returned = (utc_now(), time.monotonic_ns())
                captured_utc = utc_now()
                captured_mono = time.monotonic_ns()
                ppm = frames_dir / f"{frame:04d}.ppm"
                png = frames_dir / f"{frame:04d}.png"
                qmp("screendump", filename=str(ppm))
                with Image.open(ppm) as image:
                    rgb = image.convert("RGB")
                    samples = {}
                    for name, x, y in args.sample:
                        if 0 <= x < rgb.width and 0 <= y < rgb.height:
                            samples[name] = list(rgb.getpixel((x, y)))
                        else:
                            samples[name] = None
                    rgb.save(png)
                    size = [rgb.width, rgb.height]
                ppm.unlink()
                timeline.append(
                    {
                        "event": "frame",
                        "index": frame,
                        "utc": utc_text(captured_utc),
                        "host_monotonic_ns": captured_mono,
                        "png": str(png.relative_to(cycle_dir)),
                        "size": size,
                        "samples": samples,
                    }
                )
                frame += 1
                remaining = interval - (time.monotonic_ns() - captured_mono) / 1_000_000_000
                if remaining > 0:
                    time.sleep(remaining)
            stdout, stderr = restart.communicate(timeout=20)
            returned_utc, returned_mono = restart_returned or (utc_now(), time.monotonic_ns())
            timeline.append(
                {
                    "event": "restart-returned",
                    "utc": utc_text(returned_utc),
                    "host_monotonic_ns": returned_mono,
                    "returncode": restart.returncode,
                    "stdout": stdout,
                    "stderr": stderr,
                }
            )
            since = start_utc.strftime("%Y-%m-%d %H:%M:%S UTC")
            evidence = {
                "cycle": cycle,
                "restart_since_utc": since,
                "gdm_journal": guest_capture(
                    "sudo", "-n", "journalctl", "-u", "gdm", "--since", since, "--no-pager", "-o", "short-precise"
                ),
                "user_journal": guest_capture(
                    "journalctl", "--user", "--since", since, "--no-pager", "-o", "short-precise"
                ),
                "native_pids_and_hashes": guest_capture(
                    "sh",
                    "-c",
                    "ps -eo pid,ppid,lstart,args | grep -E '[g]dm|[g]noblin|[b]ingux' || true; "
                    "for pid in $(pgrep -f 'gnoblin|bingux' || true); do "
                    "exe=$(readlink -f /proc/$pid/exe 2>/dev/null) || continue; "
                    'printf \'pid=%s exe=%s\\n\' "$pid" "$exe"; sha256sum "$exe"; done; '
                    "for exe in $(command -v gnoblin bingux bingux-wallpaper 2>/dev/null); do "
                    'readlink -f "$exe"; sha256sum "$exe"; done',
                ),
            }
            (cycle_dir / "timeline.json").write_text(json.dumps(timeline, indent=2) + "\n")
            (cycle_dir / "guest-evidence.json").write_text(json.dumps(evidence, indent=2) + "\n")
            print(
                f"cycle {cycle}: {frame} frames; restart return={restart.returncode}; evidence={cycle_dir}", flush=True
            )
    finally:
        stream.close()
        sock.close()


if __name__ == "__main__":
    main()
