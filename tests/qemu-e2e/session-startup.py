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
import threading
import time

from PIL import Image


ROOT = Path(__file__).resolve().parents[2]
RUNNER = ROOT / "scripts/qemu-e2e"
READINESS_PROBE = r"""python3 - <<'PYPROBE'
import json, os, pwd, subprocess, time

def process(pid):
    try:
        raw = open(f"/proc/{pid}/stat").read()
        fields = raw[raw.rfind(")") + 2:].split()
        return {"pid": int(pid), "start_monotonic_seconds": int(fields[19]) / os.sysconf("SC_CLK_TCK")}
    except (OSError, ValueError, IndexError):
        return None

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

def command(*argv):
    started = time.monotonic()
    try:
        result = subprocess.run(argv, capture_output=True, text=True, timeout=1, env=env)
        return {"ok": result.returncode == 0, "returncode": result.returncode,
                "stdout": result.stdout.strip(), "stderr": result.stderr.strip(),
                "duration_seconds": time.monotonic() - started}
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"ok": False, "error": str(error), "duration_seconds": time.monotonic() - started}

compositors = []
shell_candidates = []
for name in os.listdir("/proc"):
    if name.isdigit():
        try:
            if os.stat(f"/proc/{name}").st_uid != uid:
                continue
            entry = process(name)
            if not entry:
                continue
            cmdline = open(f"/proc/{name}/cmdline", "rb").read().decode(errors="replace").split("\0")
            cmdline = [arg for arg in cmdline if arg]
            executable = os.path.basename(os.readlink(f"/proc/{name}/exe"))
            if executable == "gnoblin" and any(arg.startswith("--gnoblin-runtime-fd=") for arg in cmdline):
                entry["cmdline"] = cmdline
                compositors.append(entry)
            if executable in ("qs", "quickshell") and any(arg.endswith("/bingux/shell") for arg in cmdline):
                entry["cmdline"] = cmdline
                shell_candidates.append(entry)
        except OSError:
            pass
shell = shell_candidates[0] if len(shell_candidates) == 1 else None
shell_status = command("binguxctl", "ipc", "shell", "status")
try:
    shell_status["json"] = json.loads(shell_status.get("stdout", ""))
except json.JSONDecodeError:
    shell_status["json"] = None
shell_status["expected_fields_present"] = isinstance(shell_status["json"], dict) and all(
        key in shell_status["json"]
        and isinstance(shell_status["json"][key], bool)
        for key in ("controls", "calendar", "notifications")
)
print(json.dumps({"guest_monotonic_seconds": time.monotonic(), "compositors": compositors,
                  "bingux_main": shell, "bingux_main_candidate_count": len(shell_candidates),
                  "gnoblinctl_ping": command("gnoblinctl", "--timeout", "1", "ping"),
                  "binguxctl_shell_status": shell_status}))
PYPROBE"""


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


def readiness_result(baseline, polls):
    baseline_compositors = {
        (item["pid"], item["start_monotonic_seconds"])
        for item in baseline.get("compositors", [])
        if isinstance(item, dict)
    }
    baseline_shell = baseline.get("bingux_main")
    baseline_valid = (
        "observation_error" not in baseline
        and baseline_shell is not None
        and baseline.get("bingux_main_candidate_count") == 1
        and len(baseline_compositors) == 1
    )
    new_compositor_seen = baseline_valid and any(
        len(
            {
                (item.get("pid"), item.get("start_monotonic_seconds"))
                for item in sample.get("guest", {}).get("compositors", [])
            }
        )
        == 1
        and not {
            (item.get("pid"), item.get("start_monotonic_seconds"))
            for item in sample.get("guest", {}).get("compositors", [])
        }.intersection(baseline_compositors)
        for sample in polls
    )
    new_shell_seen = baseline_valid and any(
        (item := sample.get("guest", {}).get("bingux_main")) is not None
        and sample.get("guest", {}).get("bingux_main_candidate_count") == 1
        and (item.get("pid"), item.get("start_monotonic_seconds"))
        != (baseline_shell.get("pid"), baseline_shell.get("start_monotonic_seconds"))
        for sample in polls
    )
    ipc_seen = any(
        (guest := sample.get("guest", {})).get("gnoblinctl_ping", {}).get("ok")
        and guest["gnoblinctl_ping"].get("stdout") == "pong"
        and (status := guest.get("binguxctl_shell_status", {})).get("ok")
        and status.get("expected_fields_present")
        and isinstance(status.get("json"), dict)
        for sample in polls
    )
    observed_poll = None
    if baseline_valid:
        for sample in polls:
            guest = sample.get("guest", {})
            current_compositors = {
                (item.get("pid"), item.get("start_monotonic_seconds")) for item in guest.get("compositors", [])
            }
            current_shell = guest.get("bingux_main")
            shell_status = guest.get("binguxctl_shell_status", {})
            shell_json = shell_status.get("json")
            if (
                len(current_compositors) == 1
                and not current_compositors.intersection(baseline_compositors)
                and current_shell is not None
                and (current_shell.get("pid"), current_shell.get("start_monotonic_seconds"))
                != (baseline_shell.get("pid"), baseline_shell.get("start_monotonic_seconds"))
                and guest.get("bingux_main_candidate_count") == 1
                and guest.get("gnoblinctl_ping", {}).get("ok")
                and guest["gnoblinctl_ping"].get("stdout") == "pong"
                and shell_status.get("ok")
                and shell_status.get("expected_fields_present")
                and isinstance(shell_json, dict)
            ):
                observed_poll = sample
                break
    complete = observed_poll is not None
    return {
        "outcome": "observed" if complete else "incomplete",
        "new_compositor_observed": new_compositor_seen,
        "new_bingux_main_observed": new_shell_seen,
        "public_ipc_responses_observed": ipc_seen,
        "qualifying_poll_request_utc": observed_poll.get("request_utc") if complete else None,
        "incomplete_reason": None
        if complete
        else (
            baseline.get("observation_error")
            or "capture ended before every new process generation and IPC response was observed"
        ),
    }


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
    parser.add_argument(
        "--observe-readiness",
        action="store_true",
        help="also poll for new compositor and Bingux processes plus successful public IPC responses",
    )
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
            baseline = None
            observer = None
            observer_results = []
            observer_stop = threading.Event()
            if args.observe_readiness:
                result = guest_capture("sh", "-c", READINESS_PROBE)
                try:
                    baseline = (
                        json.loads(result["stdout"])
                        if result["returncode"] == 0
                        else {
                            "observation_error": "pre-restart identity probe failed",
                            "capture": result,
                        }
                    )
                except (KeyError, json.JSONDecodeError):
                    baseline = {"observation_error": "could not read pre-restart process identities", "capture": result}

                def observe_readiness():
                    while not observer_stop.is_set():
                        requested_utc = utc_now()
                        requested_mono = time.monotonic_ns()
                        try:
                            result = guest_capture("sh", "-c", READINESS_PROBE, timeout=4)
                        except (OSError, subprocess.SubprocessError) as error:
                            result = {"returncode": None, "error": str(error)}
                        response_mono = time.monotonic_ns()
                        record = {
                            "request_utc": utc_text(requested_utc),
                            "request_host_monotonic_ns": requested_mono,
                            "response_utc": utc_text(utc_now()),
                            "response_host_monotonic_ns": response_mono,
                            "host_round_trip_seconds": (response_mono - requested_mono) / 1_000_000_000,
                        }
                        try:
                            record["guest"] = json.loads(result["stdout"])
                            record["capture_returncode"] = result["returncode"]
                        except (KeyError, json.JSONDecodeError):
                            record["capture_error"] = result
                        observer_results.append(record)
                        observer_stop.wait(0.1)

                observer = threading.Thread(target=observe_readiness, name="startup-readiness", daemon=True)
                observer.start()
            start_utc = utc_now()
            start_mono = time.monotonic_ns()
            try:
                restart = subprocess.Popen(
                    [str(RUNNER), "guest", str(run), "--", "sudo", "-n", "systemctl", "restart", "gdm"],
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                )
            except BaseException:
                if observer is not None:
                    observer_stop.set()
                    observer.join()
                raise
            timeline = [{"event": "restart-requested", "utc": utc_text(start_utc), "host_monotonic_ns": start_mono}]
            interval = 1 / args.hz
            deadline = time.monotonic() + args.seconds
            frame = 0
            restart_returned = None
            try:
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
            finally:
                if observer is not None:
                    observer_stop.set()
                    observer.join()
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
            if args.observe_readiness:
                evidence["readiness_observation"] = {
                    "baseline": baseline,
                    "polls": observer_results,
                    **readiness_result(baseline, observer_results),
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
