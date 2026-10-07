#!/usr/bin/env python3
"""Seeded Wayland window lifecycle fuzzer for an isolated Gnoblin devkit.

Each run saves its seed, generated action plan, executed prefix, and compositor
log. Replay a run with `python3 tests/window-lifecycle-fuzz.py --replay repro.json`.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import random
import re
import secrets
import shlex
import shutil
import signal
import subprocess
import sys
import threading
import time
import traceback

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = Path(__file__).resolve()

VALID_OPERATIONS = {
    "open",
    "minimize",
    "unminimize",
    "maximize",
    "unmaximize",
    "fullscreen",
    "unfullscreen",
    "resize",
    "wm_close",
    "graceful_close",
    "abrupt_close",
}
CLOSE_OPERATIONS = {"wm_close", "graceful_close", "abrupt_close"}
# These require user input or supervisor lifecycle interfaces that the native
# window API intentionally does not provide. Keep them in a separate test path.
SEPARATE_TEST_OPERATIONS = {"activate", "hover", "frame_click", "drag_resize", "frame_policy", "shell_shutdown"}
FATAL_LOG = re.compile(
    r"(?:Mutter|Meta|Gnoblin)-CRITICAL|Traceback \(most recent call last\)|"
    r"assertion .* failed|segmentation fault|runtime check failed|core dumped|"
    r"GNOBLIN_GDB_(?:FATAL|ABORT): SIG(?:SEGV|ABRT|BUS|ILL)|GNOBLIN_GDB_CRITICAL",
    re.IGNORECASE,
)


def validate_plan(value: object) -> dict:
    if not isinstance(value, dict) or value.get("schema") != 1:
        raise ValueError("fuzz plan must be an object with schema=1")
    if not isinstance(value.get("seed"), int) or not isinstance(value.get("actions"), list):
        raise ValueError("fuzz plan requires an integer seed and actions array")
    for index, action in enumerate(value["actions"]):
        if not isinstance(action, dict) or action.get("op") not in VALID_OPERATIONS:
            raise ValueError(f"invalid action at index {index}")
        if not isinstance(action.get("delay_ms", 0), int) or action.get("delay_ms", 0) < 0:
            raise ValueError(f"invalid delay at index {index}")
        if action["op"] != "open" and not isinstance(action.get("window"), int):
            raise ValueError(f"action {index} requires an integer window id")
        if action["op"] == "resize" and any(
            not isinstance(action.get(key), int) for key in ("x", "y", "width", "height")
        ):
            raise ValueError(f"resize action {index} requires integer x, y, width, and height")
    return value


def generate_plan(seed: int, steps: int, max_windows: int) -> dict:
    if steps < 0 or max_windows < 1:
        raise ValueError("steps must be non-negative and max_windows must be positive")
    rng = random.Random(seed)
    actions: list[dict] = []
    active: list[int] = []
    next_window = 0

    def open_window() -> None:
        nonlocal next_window
        active.append(next_window)
        actions.append({"op": "open", "window": next_window, "delay_ms": rng.randrange(10, 80)})
        next_window += 1

    open_window()
    weights = [
        ("open", 12),
        ("minimize", 4),
        ("unminimize", 4),
        ("maximize", 4),
        ("unmaximize", 3),
        ("fullscreen", 2),
        ("unfullscreen", 2),
        ("resize", 8),
        ("wm_close", 4),
        ("graceful_close", 7),
        ("abrupt_close", 5),
    ]

    for _ in range(steps):
        choices = [item for item in weights if item[0] != "open" or len(active) < max_windows]
        op = (
            "open"
            if not active
            else rng.choices([item[0] for item in choices], weights=[item[1] for item in choices], k=1)[0]
        )
        if op == "open":
            open_window()
            continue
        window = rng.choice(active)
        action: dict = {"op": op, "window": window, "delay_ms": rng.randrange(5, 100)}
        if op == "resize":
            action.update(
                x=rng.randrange(20, 800),
                y=rng.randrange(20, 400),
                width=rng.randrange(220, 760),
                height=rng.randrange(180, 520),
            )
        actions.append(action)
        if op in CLOSE_OPERATIONS:
            active.remove(window)

    return {"schema": 1, "seed": seed, "steps": steps, "max_windows": max_windows, "actions": actions}


def read_plan(path: Path) -> dict:
    return validate_plan(json.loads(path.read_text()))


def save_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def default_artifact_dir(seed: int) -> Path:
    state_home = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state"))
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return state_home / "gnoblin" / "lifecycle-fuzz" / f"{stamp}-seed-{seed}"


def run_parent(args: argparse.Namespace) -> int:
    if args.replay:
        plan = read_plan(args.replay)
        seed = plan["seed"]
    else:
        seed = secrets.randbits(32) if args.seed is None else args.seed
        plan = generate_plan(seed, args.steps, args.max_windows)

    artifact_dir = args.artifact_dir or default_artifact_dir(seed)
    artifact_dir.mkdir(parents=True, exist_ok=True, mode=0o700)
    plan_path = artifact_dir / "plan.json"
    if plan_path.exists():
        raise FileExistsError(f"refusing to overwrite existing run artifact: {plan_path}")
    save_json(plan_path, plan)
    head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True)
    monitor = "1280x800"
    extra_monitor = os.environ.get("EXTRA_MONITOR")
    replay_env = f"MONITOR={shlex.quote(monitor)} "
    if extra_monitor:
        replay_env += f"EXTRA_MONITOR={shlex.quote(extra_monitor)} "
    replay_command = f"{replay_env}python3 {shlex.quote(str(SCRIPT))} --replay {shlex.quote(str(plan_path))}"
    save_json(
        artifact_dir / "run.json",
        {
            "seed": seed,
            "repo": str(ROOT),
            "git_head": head.stdout.strip(),
            "started_utc": datetime.now(timezone.utc).isoformat(),
            "prefix": os.environ.get("GNOBLIN_PREFIX", str(ROOT / "install")),
            "monitor": monitor,
            "extra_monitor": extra_monitor,
            "plan": str(plan_path),
            "replay_command": replay_command,
        },
    )

    env = os.environ.copy()
    env.update(
        {
            "GNOBLIN_LIFECYCLE_FUZZ_PLAN": str(plan_path),
            "GNOBLIN_LIFECYCLE_FUZZ_EVENTS": str(artifact_dir / "events.jsonl"),
            "GNOBLIN_LIFECYCLE_FUZZ_ARTIFACTS": str(artifact_dir),
            "GNOBLIN_DEVKIT_EXEC": f"python3 {shlex.quote(str(SCRIPT))} --inner",
            # Headless hosts may lack PipeWire, which makes Mutter's optional
            # viewer exit. Keep the compositor alive for the fuzzer's run.
            "GNOBLIN_DEVKIT_KEEP_SESSION": "1",
            "MONITOR": monitor,
            "PYTHONUNBUFFERED": "1",
        }
    )
    if extra_monitor:
        env["EXTRA_MONITOR"] = extra_monitor
    else:
        env.pop("EXTRA_MONITOR", None)

    print(f"Gnoblin lifecycle fuzzer: seed={seed}, actions={len(plan['actions'])}", flush=True)
    print(f"Artifacts: {artifact_dir}", flush=True)
    print(f"Replay: {replay_command}", flush=True)

    log_path = artifact_dir / "runner.log"
    with log_path.open("w") as log:
        process = subprocess.Popen(
            [str(ROOT / "scripts/run-gnoblin-devkit.sh")],
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        )

        def stream_output() -> None:
            assert process.stdout is not None
            for line in process.stdout:
                print(line, end="", flush=True)
                log.write(line)
                log.flush()

        reader = threading.Thread(target=stream_output, daemon=True)
        reader.start()
        timed_out = False
        try:
            return_code = process.wait(timeout=args.timeout)
        except subprocess.TimeoutExpired:
            timed_out = True
            process.send_signal(signal.SIGTERM)
            try:
                return_code = process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                return_code = process.wait()
        reader.join()

    runtime_log = artifact_dir / "compositor.log"
    if not runtime_log.exists():
        state_dir = Path(os.environ.get("GNOBLIN_STATE_DIR", Path.home() / ".local/state/gnoblin"))
        published_log = state_dir / "devkit-last.log"
        if published_log.exists():
            shutil.copy2(published_log, runtime_log)
    compositor_log = runtime_log.read_text(errors="replace") if runtime_log.exists() else ""
    diagnostics = [line for line in compositor_log.splitlines() if FATAL_LOG.search(line)]
    failed = return_code != 0 or timed_out or bool(diagnostics)
    if failed:
        executed = []
        client_failure = None
        client_failure_path = artifact_dir / "client-failure.json"
        if client_failure_path.exists():
            try:
                client_failure = json.loads(client_failure_path.read_text())
            except json.JSONDecodeError:
                client_failure = {"error": "client-failure.json was not valid JSON"}
        events_path = artifact_dir / "events.jsonl"
        if events_path.exists():
            for line in events_path.read_text().splitlines():
                try:
                    event = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if event.get("phase") == "start":
                    executed.append(event["action"])
        save_json(
            artifact_dir / "repro.json", {**plan, "actions": executed or plan["actions"], "replay_of": str(plan_path)}
        )
        save_json(
            artifact_dir / "failure.json",
            {
                "seed": seed,
                "return_code": return_code,
                "timed_out": timed_out,
                "diagnostics": diagnostics[-40:],
                "client_failure": client_failure,
                "repro": str(artifact_dir / "repro.json"),
                "runner_log": str(log_path),
                "compositor_log": str(runtime_log),
            },
        )
        (artifact_dir / "repair-request.md").write_text(
            "# Gnoblin lifecycle failure\n\n"
            f"Seed: `{seed}`\n\n"
            f"Replay: `python3 {SCRIPT} --replay {artifact_dir / 'repro.json'}`\n\n"
            "Inspect `compositor.log`, `runner.log`, and `events.jsonl`; reproduce the failure, "
            "make the smallest source fix, then rerun this exact replay and the relevant "
            "headless integration checks. Keep the patch isolated and reviewable.\n"
        )
        print(f"FAIL: devkit lifecycle run did not survive; repro: {artifact_dir / 'repro.json'}", file=sys.stderr)
        return 1

    print(
        f"PASS: native window lifecycle survived seed {seed}; frame interaction and supervisor shutdown remain separate tests; replay: {plan_path}",
        flush=True,
    )
    return 0


def run_inside() -> int:
    if not os.environ.get("GNOBLIN_COMPOSITOR_SOCKET") or not os.environ.get("WAYLAND_DISPLAY", "").startswith(
        "gnoblin-devkit-"
    ):
        raise RuntimeError("the lifecycle driver must run inside run-gnoblin-devkit.sh")
    plan_path = Path(os.environ["GNOBLIN_LIFECYCLE_FUZZ_PLAN"])
    event_path = Path(os.environ["GNOBLIN_LIFECYCLE_FUZZ_EVENTS"])
    artifact_dir = Path(os.environ["GNOBLIN_LIFECYCLE_FUZZ_ARTIFACTS"])
    plan = read_plan(plan_path)
    fixture_dir = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin-lifecycle-fuzz"
    fixture_dir.mkdir(parents=True, exist_ok=True)
    fixture = fixture_dir / "window-client"
    gnoblinctl = os.environ.get("GNOBLINCTL") or shutil.which("gnoblinctl") or "gnoblinctl"
    flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "gtk4"], text=True).split()
    subprocess.run(["cc", str(ROOT / "tests/window-lifecycle-client.c"), "-o", str(fixture), *flags], check=True)
    processes: dict[int, subprocess.Popen] = {}

    def run_ctl(*arguments: str) -> subprocess.CompletedProcess[str]:
        command = [gnoblinctl, *arguments]
        result = subprocess.run(command, capture_output=True, text=True, timeout=10)
        if result.returncode:
            detail = result.stderr.strip() or result.stdout.strip() or "no diagnostic output"
            raise RuntimeError(f"gnoblinctl {shlex.join(arguments)} failed: {detail}")
        return result

    def title_for(window_id: int) -> str:
        return f"Gnoblin Fuzz {window_id:04d}"

    def list_windows(title: str) -> list[dict]:
        result = run_ctl("--json", "window", "list", "--title", title)
        return json.loads(result.stdout)["windows"]

    def window_state(window_id: int) -> dict | None:
        title = title_for(window_id)
        return next((window for window in list_windows(title) if window.get("title") == title), None)

    def wait_for(predicate, description: str, timeout: float = 8.0):
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            last = predicate()
            if last:
                return last
            time.sleep(0.05)
        raise TimeoutError(f"timed out waiting for {description}; last state={last!r}")

    def wait_window(window_id: int, present: bool) -> dict | None:
        return wait_for(
            lambda: (state if (state := window_state(window_id)) else None) if present else not window_state(window_id),
            f"window {window_id} {'to map' if present else 'to close'}",
        )

    def call_window(action: str, window_id: int) -> None:
        state = window_state(window_id)
        if state is None:
            raise RuntimeError(f"target window {window_id} is not mapped")
        run_ctl("window", action, state["id"], "--json")

    def operate(action: dict) -> None:
        op = action["op"]
        window_id = action.get("window")
        if op == "open":
            process = subprocess.Popen(
                [str(fixture), title_for(window_id)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
            )
            processes[window_id] = process
            wait_for(
                lambda: (
                    current
                    if (current := window_state(window_id)) and current.get("frame") and current.get("monitor_id")
                    else None
                ),
                f"window {window_id} to map with native geometry",
            )
            # The snapshot can become visible before the first Wayland configure
            # has reached GTK. Give the client a short turn before mutating it.
            time.sleep(0.25)
            return

        state = window_state(window_id)
        if state is None:
            raise RuntimeError(f"target window {window_id} is not mapped")
        if op in CLOSE_OPERATIONS:
            process = processes[window_id]
            if op == "wm_close":
                call_window("close", window_id)
            elif process.poll() is None:
                process.send_signal(signal.SIGUSR1) if op == "graceful_close" else process.kill()
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
            wait_window(window_id, False)
            if op == "wm_close":
                processes[window_id].wait(timeout=3)
            return
        if op == "resize":
            # Geometry changes need a normal, visible window. Random prior
            # actions can leave it minimized, maximized, or fullscreen.
            if state.get("minimized") or state.get("maximized"):
                call_window("restore", window_id)
            if state.get("fullscreen"):
                call_window("unfullscreen", window_id)
            wait_for(
                lambda: (
                    current
                    if (current := window_state(window_id))
                    and not current.get("minimized")
                    and not current.get("fullscreen")
                    and not current.get("maximized")
                    else None
                ),
                f"window {window_id} to become movable before resize",
            )
            run_ctl("window", "move", state["id"], str(action["x"]), str(action["y"]), "--json")
            run_ctl("window", "resize", state["id"], str(action["width"]), str(action["height"]), "--json")
            return
        if op == "maximize" and state.get("fullscreen"):
            # Mutter does not allow maximizing a fullscreen window, so a random plan can
            # reach this state. Leave fullscreen first, as the resize action does.
            call_window("unfullscreen", window_id)
            wait_for(
                lambda: (current if (current := window_state(window_id)) and not current.get("fullscreen") else None),
                f"window {window_id} to leave fullscreen before maximize",
            )
        call_window(op, window_id)
        property_name, expected = {
            "minimize": ("minimized", True),
            "unminimize": ("minimized", False),
            "maximize": ("maximized", True),
            "unmaximize": ("maximized", False),
            "fullscreen": ("fullscreen", True),
            "unfullscreen": ("fullscreen", False),
        }[op]
        wait_for(
            lambda: (
                current if (current := window_state(window_id)) and current.get(property_name) is expected else None
            ),
            f"window {window_id} {property_name}={expected}",
        )

    failure = None
    event_path.write_text("")
    try:
        with event_path.open("a") as events:
            events.write(json.dumps({"phase": "devkit-ready", "pid": os.getpid()}) + "\n")
        for index, action in enumerate(plan["actions"]):
            with event_path.open("a") as events:
                events.write(json.dumps({"phase": "start", "index": index, "action": action}) + "\n")
            operate(action)
            time.sleep(action.get("delay_ms", 0) / 1000)
            with event_path.open("a") as events:
                events.write(json.dumps({"phase": "done", "index": index, "action": action}) + "\n")
            if index % 25 == 0:
                print(f"lifecycle fuzz: action {index + 1}/{len(plan['actions'])}", flush=True)

        for window_id, process in processes.items():
            if process.poll() is None:
                process.send_signal(signal.SIGUSR1)
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)
                wait_window(window_id, False)
    except Exception as error:
        failure = {"error": str(error), "traceback": traceback.format_exc(), "seed": plan["seed"]}
        try:
            failure["windows_at_failure"] = json.loads(
                subprocess.check_output([gnoblinctl, "--json", "window", "list"], text=True, timeout=10)
            )["windows"]
        except Exception as diagnostic_error:
            failure["windows_diagnostic_error"] = str(diagnostic_error)
        failure["fixture_processes"] = {
            str(window_id): {"pid": process.pid, "returncode": process.poll()}
            for window_id, process in processes.items()
        }
        save_json(artifact_dir / "client-failure.json", failure)
        print(f"lifecycle fuzz failure: {error}\n{failure['traceback']}", file=sys.stderr, flush=True)
    finally:
        for process in processes.values():
            if process.poll() is None:
                process.kill()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
        runtime_log = os.environ.get("GNOBLIN_DEVKIT_RUNTIME_LOG")
        if runtime_log and Path(runtime_log).exists():
            shutil.copy2(runtime_log, artifact_dir / "compositor.log")

    if failure:
        return 1
    print(f"PASS: survived {len(plan['actions'])} native window lifecycle operations", flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, help="reproducible seed (default: random 32-bit seed)")
    parser.add_argument("--steps", type=int, default=300, help="random operations after the first window")
    parser.add_argument("--max-windows", type=int, default=6, help="maximum live fixture windows")
    parser.add_argument("--replay", type=Path, help="replay a saved plan.json or repro.json")
    parser.add_argument("--artifact-dir", type=Path, help="where to keep logs and reproduction data")
    parser.add_argument("--timeout", type=int, default=300, help="whole-devkit timeout in seconds")
    parser.add_argument("--inner", action="store_true", help=argparse.SUPPRESS)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    return run_inside() if args.inner else run_parent(args)


if __name__ == "__main__":
    sys.exit(main())
