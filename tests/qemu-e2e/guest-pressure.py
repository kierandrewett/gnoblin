#!/usr/bin/env python3
"""Bounded guest-only CPU/memory pressure check for a live Gnoblin session.

The paced client writes START (absolute monotonic microseconds), FRAME
(microseconds since START), and KEY (absolute monotonic microseconds) records.
Phase boundaries are recorded in both clocks so frame and input observations
cannot silently be assigned to the pre-focus startup interval.
The result evaluates lifecycle and observation integrity only. Performance is
reported as not evaluated: this probe has no rendering acceptance budget.
"""

import argparse
import ctypes
import fcntl
import json
import math
import os
import signal
import subprocess
import threading
import time
from pathlib import Path

from gi.repository import Gio, GLib

REMOTE_DESKTOP = "org.gnome.Mutter.RemoteDesktop"
REMOTE_OBJECT = "/org/gnome/Mutter/RemoteDesktop"
REMOTE_SESSION = "org.gnome.Mutter.RemoteDesktop.Session"
KEY_A = 30
BTN_LEFT = 0x110
UNIT_PROPERTIES = ("ActiveState", "CPUUsageNSec", "MemoryCurrent")


def monotonic_us():
    return time.monotonic_ns() // 1000


def interrupted(signum, frame):
    del signum, frame
    raise KeyboardInterrupt


def percentile(values, fraction=0.95):
    if not values:
        return None
    return sorted(values)[math.ceil(len(values) * fraction) - 1]


def check_positive(parser, value, name, allow_zero=False):
    if value < 0 or (value == 0 and not allow_zero):
        parser.error(f"{name} must be {'non-negative' if allow_zero else 'positive'}")
    return value


def run(command, **kwargs):
    return subprocess.run(command, check=True, text=True, **kwargs)


def set_observer_policy():
    """Use the prior observer policy and report its actual result."""
    libc = ctypes.CDLL(None, use_errno=True)
    libc.prctl(15, ctypes.c_char_p(b"pressure-watch"), 0, 0, 0)
    run(["sudo", "-n", "chrt", "-a", "-o", "-p", "0", str(os.getpid())])
    run(["sudo", "-n", "renice", "-n", "-10", "-p", str(os.getpid())], stdout=subprocess.DEVNULL)
    return {
        "pid": os.getpid(),
        "scheduler": os.sched_getscheduler(0),
        "nice": os.getpriority(os.PRIO_PROCESS, 0),
    }


def process_policy(pid):
    """Return scheduling facts for one process identity, rejecting PID reuse."""
    stat = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    # comm is parenthesized and may itself contain spaces or parentheses.
    fields = stat[stat.rfind(")") + 2 :].split()
    identity = fields[19]  # proc stat field 22: starttime (fields begin at 3).
    return {
        "pid": pid,
        "starttime_ticks": int(identity),
        "nice": os.getpriority(os.PRIO_PROCESS, pid),
        "scheduler": os.sched_getscheduler(pid),
    }


def require_normal_policy(policy, where):
    if policy["nice"] != 0 or policy["scheduler"] != os.SCHED_OTHER:
        raise RuntimeError(
            f"paced client policy mismatch {where}: pid {policy['pid']} "
            f"nice={policy['nice']} scheduler={policy['scheduler']} "
            f"(expected nice=0 scheduler={os.SCHED_OTHER})"
        )


class ClientLog:
    def __init__(self, client, frames_path):
        self.records = []
        self.policy_error = None
        self.frames_path = frames_path
        self.log = frames_path.open("w", encoding="utf-8")
        self.process = subprocess.Popen(
            [
                "/usr/bin/nice",
                "-n",
                str(max(0, -os.getpriority(os.PRIO_PROCESS, 0))),
                "/usr/bin/chrt",
                "--other",
                "0",
                str(client),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            bufsize=1,
        )
        self.pid = self.process.pid
        self.identity = None
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()

    def _read(self):
        assert self.process.stdout
        for line in self.process.stdout:
            self.log.write(line)
            self.log.flush()
            fields = line.split()
            record = None
            try:
                if len(fields) == 2 and fields[0] == "START":
                    try:
                        policy = process_policy(self.pid)
                        self.identity = policy
                        record = {"kind": "START", "absolute_us": int(fields[1]), "process": policy}
                        try:
                            require_normal_policy(policy, "after START")
                        except RuntimeError as exc:
                            self.policy_error = str(exc)
                    except Exception as exc:
                        self.policy_error = str(exc)
                elif len(fields) == 4 and fields[0] == "FRAME":
                    record = {
                        "kind": "FRAME",
                        "client_us": int(fields[1]),
                        "delta_us": int(fields[2]),
                        "wayland_stamp": int(fields[3]),
                    }
                elif len(fields) == 4 and fields[0] == "KEY":
                    record = {
                        "kind": "KEY",
                        "absolute_us": int(fields[1]),
                        "keycode": int(fields[2]),
                        "state": int(fields[3]),
                    }
                elif fields and fields[0] in ("START", "FRAME", "KEY"):
                    record = {"kind": "MALFORMED", "line": line.rstrip()}
            except ValueError:
                record = {"kind": "MALFORMED", "line": line.rstrip()}
            if record:
                self.records.append(record)
        self.log.flush()

    def wait_start(self, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for item in list(self.records):
                if item["kind"] == "START":
                    return item["absolute_us"]
            if self.policy_error:
                raise RuntimeError(self.policy_error)
            if self.process.poll() is not None:
                raise RuntimeError(f"pressure client exited before START ({self.process.returncode})")
            time.sleep(0.01)
        raise RuntimeError("pressure client did not emit START within 5 seconds")

    def wait_key_press_after(self, sent_us, timeout=2):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            for item in list(self.records):
                if (
                    item["kind"] == "KEY"
                    and item["keycode"] == KEY_A
                    and item["state"] == 1
                    and item["absolute_us"] >= sent_us
                ):
                    return item
            if self.process.poll() is not None:
                raise RuntimeError(f"pressure client exited while awaiting focus receipt ({self.process.returncode})")
            time.sleep(0.01)
        raise RuntimeError("focus click did not deliver KEY 30 press to Frame pressure probe")

    def alive(self):
        return self.process.poll() is None

    def sample_policy(self):
        return process_policy(self.pid)

    def require_sample_policy(self, policy):
        if self.identity is None or policy["starttime_ticks"] != self.identity["starttime_ticks"]:
            raise RuntimeError(f"paced client process identity changed for pid {self.pid}")
        require_normal_policy(policy, "during sample")

    def stop(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=5)
        self.thread.join(timeout=1)
        self.log.close()


class RemoteInput:
    def __init__(self):
        self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self.session = None

    def call(self, path, interface, method, args=None, timeout=1500):
        return self.bus.call_sync(
            REMOTE_DESKTOP, path, interface, method, args, None, Gio.DBusCallFlags.NONE, timeout, None
        )

    def start(self):
        self.session = self.call(REMOTE_OBJECT, REMOTE_DESKTOP, "CreateSession").unpack()[0]
        self.call(self.session, REMOTE_SESSION, "Start")

    def stop(self):
        if self.session:
            try:
                self.call(self.session, REMOTE_SESSION, "Stop")
            except GLib.Error:
                pass
            self.session = None

    def focus(self, x, y):
        state = self.bus.call_sync(
            "org.gnome.Mutter.DisplayConfig",
            "/org/gnome/Mutter/DisplayConfig",
            "org.gnome.Mutter.DisplayConfig",
            "GetCurrentState",
            None,
            None,
            Gio.DBusCallFlags.NONE,
            1500,
            None,
        ).unpack()
        _, monitors, logical, properties = state
        if len(monitors) != 1 or len(logical) != 1 or tuple(logical[0][:2]) != (0, 0):
            raise RuntimeError("pressure focus requires one monitor at origin 0,0")
        mode = next(mode for mode in monitors[0][1] if mode[6].get("is-current"))
        width, height = mode[1:3]
        if logical[0][3] in (1, 3, 5, 7):
            width, height = height, width
        if properties.get("layout-mode", 1) == 1:
            width = round(width / logical[0][2])
            height = round(height / logical[0][2])
        if not (0 <= x < width and 0 <= y < height):
            raise RuntimeError(f"focus target {x},{y} outside {width}x{height} display")
        self.focus_display = {"width": width, "height": height}
        # GNOME's upper-left hot corner opens Overview even for injected motion.
        # Reset through the lower-right corner before moving to the client.
        self.call(
            self.session,
            REMOTE_SESSION,
            "NotifyPointerMotionRelative",
            GLib.Variant("(dd)", (float(width * 2), float(height * 2))),
        )
        time.sleep(0.25)
        self.call(
            self.session,
            REMOTE_SESSION,
            "NotifyPointerMotionRelative",
            GLib.Variant("(dd)", (x - width + 1, y - height + 1)),
        )
        time.sleep(0.25)
        self.call(self.session, REMOTE_SESSION, "NotifyPointerButton", GLib.Variant("(ib)", (BTN_LEFT, True)))
        self.call(self.session, REMOTE_SESSION, "NotifyPointerButton", GLib.Variant("(ib)", (BTN_LEFT, False)))

    def key_a(self):
        sent = monotonic_us()
        self.call(self.session, REMOTE_SESSION, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (KEY_A, True)))
        self.call(self.session, REMOTE_SESSION, "NotifyKeyboardKeycode", GLib.Variant("(ub)", (KEY_A, False)))
        return sent

    def probe(self):
        start = monotonic_us()
        self.call(REMOTE_OBJECT, "org.freedesktop.DBus.Properties", "GetAll", GLib.Variant("(s)", (REMOTE_DESKTOP,)))
        return (monotonic_us() - start) / 1000


def unit_state(name):
    result = run(
        ["sudo", "-n", "systemctl", "show", name, *[f"--property={prop}" for prop in UNIT_PROPERTIES]],
        capture_output=True,
    ).stdout.splitlines()
    return dict(line.split("=", 1) for line in result if "=" in line)


def read_pressure(name):
    return Path(f"/proc/pressure/{name}").read_text(encoding="utf-8").strip()


def read_meminfo():
    wanted = {"MemAvailable", "SwapFree"}
    return {
        key.rstrip(":"): value.strip()
        for key, value in (line.split(":", 1) for line in Path("/proc/meminfo").read_text().splitlines())
        if key.rstrip(":") in wanted
    }


def summarize_observations(results, rows, phases, sent, records):
    """Aggregate the captured stream; never infer a rendering performance pass."""
    results["performance"] = {
        "status": "not_evaluated",
        "reason": "No performance acceptance budget; frame callbacks do not measure presentation.",
    }
    malformed = [record for record in records if record["kind"] == "MALFORMED"]
    if malformed:
        results["errors"].append(f"malformed client records: {len(malformed)}")

    # The first callback has no predecessor. Subsequent deltas must describe
    # adjacent CLOCK_MONOTONIC callback dispatches, not Wayland's opaque stamp.
    frame_records = [record for record in records if record["kind"] == "FRAME"]
    frame_errors = []
    previous = None
    for frame in frame_records:
        timestamp, delta = frame["client_us"], frame["delta_us"]
        if (
            timestamp < 0
            or (previous is None and delta != 0)
            or (previous is not None and (delta <= 0 or delta != timestamp - previous))
        ):
            frame_errors.append(f"inconsistent FRAME timestamp/delta at client_us={timestamp}")
        previous = timestamp
    results["errors"].extend(frame_errors)
    intervals = frame_records[1:] if not frame_errors and not malformed else []

    key_receipts = [
        record for record in records if record["kind"] == "KEY" and record["keycode"] == KEY_A and record["state"] == 1
    ]
    paired = []
    # There is no sequence ID in wl_keyboard.key. Pair in stream order only
    # when the complete counts and clocks agree; a missing key shifts all
    # later pairs. Preserve raw evidence, but publish no guessed latencies.
    if len(key_receipts) != len(sent):
        results["errors"].append(f"input receipt mismatch: {len(key_receipts)} received for {len(sent)} sent")
    elif not malformed:
        pairs = list(zip(sent, key_receipts))
        if any(receipt["absolute_us"] < item["sent_absolute_us"] for item, receipt in pairs):
            results["errors"].append("received KEY timestamp precedes its injected key")
        elif any(
            later["absolute_us"] < earlier["absolute_us"] for earlier, later in zip(key_receipts, key_receipts[1:])
        ) or any(later["sent_absolute_us"] < earlier["sent_absolute_us"] for earlier, later in zip(sent, sent[1:])):
            results["errors"].append("input timestamps are not monotonic")
        else:
            paired = [
                item
                | {
                    "received_absolute_us": receipt["absolute_us"],
                    "latency_ms": (receipt["absolute_us"] - item["sent_absolute_us"]) / 1000,
                }
                for item, receipt in pairs
            ]
    for phase in phases:
        phase_rows = [row for row in rows if row["phase"] == phase["name"]]
        phase_inputs = [item for item in paired if item["phase"] == phase["name"]]
        frames = [
            item for item in frame_records if phase["start_client_us"] <= item["client_us"] < phase["end_client_us"]
        ]
        phase_intervals = [
            item for item in intervals if phase["start_client_us"] <= item["client_us"] < phase["end_client_us"]
        ]
        phase.update(
            {
                "samples": len(phase_rows),
                "input_samples": len(phase_inputs),
                "frame_callbacks": len(frames),
                "frame_intervals": len(phase_intervals),
                "frame_cross_boundary_intervals": sum(
                    frame["client_us"] - frame["delta_us"] < phase["start_client_us"] for frame in phase_intervals
                ),
                "frame_p95_ms": percentile([frame["delta_us"] / 1000 for frame in phase_intervals]),
                "frame_max_ms": max((frame["delta_us"] / 1000 for frame in phase_intervals), default=None),
                "frame_over_100ms": sum(frame["delta_us"] > 100000 for frame in phase_intervals),
                "input_p95_ms": percentile([item["latency_ms"] for item in phase_inputs]),
                "input_max_ms": max((item["latency_ms"] for item in phase_inputs), default=None),
                "dbus_p95_ms": percentile([row["dbus_rtt_ms"] for row in phase_rows if row["dbus_rtt_ms"] is not None]),
            }
        )
        if phase["samples"] == 0 or phase["input_samples"] == 0 or phase["frame_intervals"] == 0:
            results["errors"].append(f"insufficient observations in {phase['name']}")
    results["lifecycle_passed"] = results["lifecycle_passed"] and not results["errors"]
    return {"sent": sent, "received": key_receipts, "paired": paired}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact_directory", type=Path)
    parser.add_argument("--client", required=True, type=Path)
    parser.add_argument("--load-probe", required=True, type=Path)
    parser.add_argument("--baseline-seconds", type=float, default=8)
    parser.add_argument("--combined-seconds", type=float, default=40)
    parser.add_argument("--recovery-seconds", type=float, default=5)
    parser.add_argument("--memory-mib", type=int, default=1900)
    parser.add_argument("--swap-mib", type=int, default=512)
    parser.add_argument("--runtime-max-seconds", type=int, default=75)
    parser.add_argument("--sample-interval", type=float, default=0.2)
    parser.add_argument("--focus-x", type=float, default=220)
    parser.add_argument("--focus-y", type=float, default=220)
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, interrupted)
    for name in ("baseline_seconds", "combined_seconds", "recovery_seconds", "sample_interval"):
        check_positive(parser, getattr(args, name), "--" + name.replace("_", "-"))
    for name in ("memory_mib", "runtime_max_seconds"):
        check_positive(parser, getattr(args, name), "--" + name.replace("_", "-"))
    check_positive(parser, args.swap_mib, "--swap-mib", allow_zero=True)
    total = args.baseline_seconds + args.combined_seconds + args.recovery_seconds
    if total > args.runtime_max_seconds:
        parser.error("phase durations must fit within --runtime-max-seconds")
    if args.artifact_directory.exists():
        parser.error("artifact_directory must not already exist")
    if not args.client.is_file() or not os.access(args.client, os.X_OK):
        parser.error("--client must be an executable paced frame-pressure client")
    if not args.load_probe.is_file() or not os.access(args.load_probe, os.X_OK):
        parser.error("--load-probe must be an executable")

    runtime = Path(os.environ.get("XDG_RUNTIME_DIR", f"/run/user/{os.getuid()}"))
    os.environ.setdefault("XDG_RUNTIME_DIR", str(runtime))
    os.environ.setdefault("WAYLAND_DISPLAY", "wayland-0")
    os.environ.setdefault("DBUS_SESSION_BUS_ADDRESS", f"unix:path={runtime / 'bus'}")
    args.artifact_directory.mkdir(parents=True)
    seat_lock = (runtime / "gnoblin-shadow-proof.lock").open("w")
    fcntl.flock(seat_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    results = {
        "lifecycle_passed": False,
        "errors": [],
        "configuration": vars(args)
        | {
            "artifact_directory": str(args.artifact_directory),
            "client": str(args.client),
            "load_probe": str(args.load_probe),
        },
        "artifacts": {},
    }
    # Paths are made JSON-safe before the final write.
    results["configuration"] = {k: str(v) if isinstance(v, Path) else v for k, v in results["configuration"].items()}
    rows, phases, sent, units = [], [], [], []
    client = remote = None
    started_us = None
    failure = None

    def write(name, value):
        path = args.artifact_directory / name
        path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        results["artifacts"][name] = str(path)

    def require_alive(phase):
        if not client.alive():
            raise RuntimeError(f"pressure client exited during {phase} ({client.process.returncode})")

    def run_phase(name, seconds, expect_units=()):
        phase = {"name": name, "start_absolute_us": monotonic_us()}
        phase["start_client_us"] = phase["start_absolute_us"] - started_us
        phases.append(phase)
        deadline = time.monotonic() + seconds
        try:
            while time.monotonic() < deadline:
                require_alive(name)
                key_sent = remote.key_a()
                sent.append({"phase": name, "sent_absolute_us": key_sent, "sent_client_us": key_sent - started_us})
                try:
                    rtt = remote.probe()
                    dbus_error = None
                except GLib.Error as exc:
                    rtt, dbus_error = None, str(exc)
                states = {unit: unit_state(unit) for unit in units}
                inactive = [unit for unit in expect_units if states[unit].get("ActiveState") != "active"]
                if inactive:
                    raise RuntimeError(f"load unit inactive during {name}: {', '.join(inactive)}")
                sampled_us = monotonic_us()
                client_policy = client.sample_policy()
                rows.append(
                    {
                        "phase": name,
                        "absolute_us": sampled_us,
                        "client_us": sampled_us - started_us,
                        "dbus_rtt_ms": rtt,
                        "dbus_error": dbus_error,
                        "units": states,
                        "memory_psi": read_pressure("memory"),
                        "cpu_psi": read_pressure("cpu"),
                        "meminfo": read_meminfo(),
                        "client_alive": True,
                        "client_policy": client_policy,
                    }
                )
                client.require_sample_policy(client_policy)
                if dbus_error:
                    raise RuntimeError(f"RemoteDesktop D-Bus probe failed during {name}: {dbus_error}")
                time.sleep(args.sample_interval)
        finally:
            phase["end_absolute_us"] = monotonic_us()
            phase["end_client_us"] = phase["end_absolute_us"] - started_us

    try:
        results["observer_policy"] = set_observer_policy()
        write("observer-policy.json", results["observer_policy"])
        client = ClientLog(args.client, args.artifact_directory / "frames.log")
        started_us = client.wait_start()
        results["client_start_absolute_us"] = started_us
        results["client_policy_at_start"] = client.identity
        if client.policy_error:
            raise RuntimeError(client.policy_error)
        remote = RemoteInput()
        remote.start()
        remote.focus(args.focus_x, args.focus_y)
        focus_sent = remote.key_a()
        focus_key = client.wait_key_press_after(focus_sent)
        sent.append(
            {
                "phase": "focus",
                "sent_absolute_us": focus_sent,
                "sent_client_us": focus_sent - started_us,
                "focus_receipt_absolute_us": focus_key["absolute_us"],
            }
        )
        results["focus"] = {
            "x": args.focus_x,
            "y": args.focus_y,
            "receipt_absolute_us": focus_key["absolute_us"],
            "display": remote.focus_display,
        }
        run_phase("baseline", args.baseline_seconds)
        stamp = f"{os.getpid()}-{int(time.time())}"
        for kind, extra in (("memory", [str(args.memory_mib)]), ("cpu", [])):
            name = f"gnoblin-pressure-{kind}-{stamp}"
            command = [
                "sudo",
                "-n",
                "systemd-run",
                "--quiet",
                f"--unit={name}",
                f"--property=RuntimeMaxSec={args.runtime_max_seconds}",
                f"--property=MemoryMax={args.memory_mib}M",
                f"--property=MemorySwapMax={args.swap_mib}M",
                "--property=OOMPolicy=stop",
                "--property=Nice=0",
                "--property=CPUWeight=100",
                str(args.load_probe),
                kind,
                *extra,
            ]
            units.append(name)
            run(command)
            if unit_state(name).get("ActiveState") != "active":
                raise RuntimeError(f"load unit failed to become active: {name}")
        run_phase("combined", args.combined_seconds, units)
        for unit in reversed(units):
            run(["sudo", "-n", "systemctl", "stop", unit])
        run_phase("recovery", args.recovery_seconds)
        results["lifecycle_passed"] = True
    except Exception as exc:
        failure = exc
        results["errors"].append(str(exc))
    finally:
        if remote:
            remote.stop()
        for unit in reversed(units):
            subprocess.run(
                ["sudo", "-n", "systemctl", "stop", unit],
                text=True,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
        if client:
            client.stop()
            records = client.records
        else:
            records = []
        inputs = summarize_observations(results, rows, phases, sent, records)
        write("samples.json", rows)
        write("phases.json", phases)
        write("inputs.json", inputs)
        write("summary.json", phases)
        write("result.json", results)
    if failure:
        raise failure
    if not results["lifecycle_passed"]:
        raise SystemExit("pressure lifecycle check failed; see result.json")


if __name__ == "__main__":
    main()
