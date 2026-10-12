#!/usr/bin/env python3
"""Count real compositor shadow draws while a paced Wayland client redraws.

Run in a disposable native VM with sudo perf access. The supplied client is
tests/window-shadow-client.c compiled with xdg-shell client protocol bindings.
Use --reference before changing the compositor to retain the failing result.
"""

import argparse
import fcntl
import json
import os
from pathlib import Path
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("artifacts", type=Path)
parser.add_argument("--client", type=Path, required=True)
parser.add_argument("--prefix", type=Path, default=Path("/usr/local/lib/gnoblin"))
parser.add_argument("--reference", action="store_true")
args = parser.parse_args()
art = args.artifacts.resolve()
art.mkdir(parents=True, exist_ok=False)
os.environ.setdefault("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}")
os.environ.setdefault("WAYLAND_DISPLAY", "wayland-0")
seat_lock = (Path(os.environ["XDG_RUNTIME_DIR"]) / "gnoblin-shadow-proof.lock").open("w")
fcntl.flock(seat_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
subprocess.run(["sudo", "-n", "chrt", "-a", "-o", "-p", "0", str(os.getpid())], check=True)
pid = int(subprocess.check_output(["pgrep", "-f", f"^{args.prefix}/bin/gnoblin --wayland"]).strip())
library = args.prefix / "lib64/libmutter-51.so.0.0.0"
event = "probe_libmutter:gnoblin_shadow_cache_proof"
log = (art / "frames.log").open("w")
client = None


def control(*argv):
    return subprocess.check_output([str(args.prefix / "bin/gnoblinctl"), "--json", *argv], text=True)


try:
    subprocess.run(
        [
            "sudo",
            "-n",
            "perf",
            "probe",
            "-x",
            str(library),
            "--add",
            "gnoblin_shadow_cache_proof=window_shadow_paint_target",
        ],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    client = subprocess.Popen([str(args.client.resolve())], stdout=log)
    window = None
    for _ in range(50):
        window = next(
            (w for w in json.loads(control("window", "list"))["windows"] if w["title"] == "Frame pressure probe"), None
        )
        if window:
            break
        time.sleep(0.1)
    assert window, "probe client did not map"
    control("window", "move", window["id"], "320", "220")
    time.sleep(2)
    window = next(w for w in json.loads(control("window", "list"))["windows"] if w["id"] == window["id"])
    assert window["frame"]["x"] == 320 and window["frame"]["y"] == 220, "probe move did not settle"
    subprocess.run(["grim", str(art / "settled.png")], check=True)
    begin = time.monotonic()
    subprocess.run(
        [
            "sudo",
            "-n",
            "perf",
            "stat",
            "--no-big-num",
            "-x",
            ";",
            "-e",
            event,
            "-p",
            str(pid),
            "-o",
            str(art / "draws.csv"),
            "--",
            "sleep",
            "8",
        ],
        check=True,
    )
    duration = time.monotonic() - begin
    log.flush()
    lines = (art / "frames.log").read_text().splitlines()
    callbacks = sum(line.startswith("FRAME ") for line in lines)
    row = next(line for line in (art / "draws.csv").read_text().splitlines() if event in line)
    draws = int(row.split(";")[0].strip())
    report = dict(
        pid=pid,
        library=str(library),
        callbacks=callbacks,
        shadow_draws=draws,
        duration_seconds=duration,
        window=window,
        expected="at most two settled shadow draws",
    )
    (art / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report), flush=True)
    assert callbacks >= 20, "client did not redraw enough to exercise caching"
    if not args.reference:
        assert draws <= 2, f"Unchanged shadow recalculated {draws} times"
finally:
    if client:
        client.terminate()
        client.wait(timeout=5)
    log.close()
    subprocess.run(
        ["sudo", "-n", "perf", "probe", "--del", event], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
