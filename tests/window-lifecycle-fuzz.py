#!/usr/bin/env python3
"""Seeded Wayland window lifecycle fuzzer for an isolated Gnoblin session.

Each run saves its seed, generated action plan, executed prefix, and shell log.
Replay a run with `python3 tests/window-lifecycle-fuzz.py --replay repro.json`.
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
sys.path.insert(0, str(ROOT / "tests"))
from gnoblin_test_session import (  # noqa: E402
    eval_shell,
    send_pointer,
    shell_window,
    wait_for,
)

VALID_OPERATIONS = {
    "open",
    "activate",
    "minimize",
    "unminimize",
    "maximize",
    "unmaximize",
    "fullscreen",
    "unfullscreen",
    "resize",
    "hover",
    "frame_click",
    "drag_resize",
    "frame_policy",
    "wm_close",
    "graceful_close",
    "abrupt_close",
    "shell_shutdown",
}
FATAL_LOG = re.compile(
    r"GNOME Shell-CRITICAL|(?:Clutter|Mutter|Meta)-CRITICAL|JS ERROR|"
    r"Traceback \(most recent call last\)|assertion .* failed|SIG(SEGV|ABRT)|"
    r"segmentation fault|runtime check failed|core dumped",
    re.IGNORECASE,
)
FRAME_MODES = {0: "off", 1: "auto", 2: "prefer-server", 3: "replace"}


def write_frame_config(path: Path, policies: dict[int, list[int]]) -> None:
    """Set fuzz-window frames through Gnoblin's normal window-rule config path."""
    rules = [
        '        {match = {title = "^Gnoblin Fuzz [0-9][0-9][0-9][0-9]$"}, '
        'frame = {mode = "replace", extents = {36, 2, 2, 2}}},'
    ]
    for window_id, policy in sorted(policies.items()):
        mode = FRAME_MODES[policy[0]]
        crop = ", ".join(str(value) for value in policy[1:5])
        extents = ", ".join(str(value) for value in policy[5:9])
        rules.append(
            f'        {{match = {{title = "^Gnoblin Fuzz {window_id:04d}$"}}, '
            f'frame = {{mode = "{mode}", crop = {{{crop}}}, extents = {{{extents}}}}}}},'
        )
    contents = 'return { ["window-rules"] = {\n' + "\n".join(rules) + "\n    } }\n"
    temporary = path.with_suffix(".tmp")
    temporary.write_text(contents)
    temporary.replace(path)


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
    shutdowns = [i for i, action in enumerate(value["actions"]) if action["op"] == "shell_shutdown"]
    if shutdowns and shutdowns != [len(value["actions"]) - 1]:
        raise ValueError("shell_shutdown must be the final action")
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
        ("activate", 10),
        ("minimize", 4),
        ("unminimize", 4),
        ("maximize", 4),
        ("unmaximize", 3),
        ("fullscreen", 2),
        ("unfullscreen", 2),
        ("resize", 8),
        ("hover", 10),
        ("frame_click", 5),
        ("drag_resize", 4),
        ("frame_policy", 5),
        ("wm_close", 4),
        ("graceful_close", 7),
        ("abrupt_close", 5),
    ]
    closes = {"wm_close", "graceful_close", "abrupt_close", "frame_click"}

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
        elif op == "hover":
            action["point"] = rng.randrange(5)
        elif op == "frame_policy":
            mode = rng.choice([0, 2, 3])
            action["policy"] = [
                mode,
                8 if mode == 2 else 0,
                0,
                0,
                0,
                36 if mode else 0,
                2 if mode else 0,
                2 if mode else 0,
                2 if mode else 0,
            ]
        actions.append(action)
        if op in closes:
            active.remove(window)

    if not active:
        open_window()
    actions.append({"op": "shell_shutdown", "window": rng.choice(active), "delay_ms": rng.randrange(30, 100)})

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
    config_path = artifact_dir / "window-rules.lua"
    write_frame_config(config_path, {})
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
            "gnoblin_config": str(config_path),
            "plan": str(plan_path),
            "replay_command": replay_command,
        },
    )
    state_dir = artifact_dir / "private-state"
    state_dir.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(
        {
            "GNOBLIN_CONFIG": str(config_path),
            "GNOBLIN_TEST_UNSAFE_MODE": "1",
            "GNOBLIN_TEST_CLIENT": str(SCRIPT),
            "GNOBLIN_LIFECYCLE_FUZZ_INNER": "1",
            "GNOBLIN_LIFECYCLE_FUZZ_PLAN": str(plan_path),
            "GNOBLIN_LIFECYCLE_FUZZ_EVENTS": str(artifact_dir / "events.jsonl"),
            "GNOBLIN_STATE_DIR": str(state_dir),
            "GNOBLIN_TEST_CLIENT_EXPECTS_SHELL_EXIT": "1" if plan["actions"][-1]["op"] == "shell_shutdown" else "0",
            "MONITOR": "1280x800",
            "PYTHONUNBUFFERED": "1",
        }
    )
    print(f"Gnoblin lifecycle fuzzer: seed={seed}, actions={len(plan['actions'])}", flush=True)
    print(f"Artifacts: {artifact_dir}", flush=True)
    print(f"Replay: {replay_command}", flush=True)

    log_path = artifact_dir / "runner.log"
    with log_path.open("w") as log:
        process = subprocess.Popen(
            [str(ROOT / "scripts/run-gnome-shell.sh")],
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

    published_log = state_dir / "gnome-shell-last.log"
    if published_log.exists():
        shutil.copy2(published_log, artifact_dir / "shell.log")
    shell_log = (
        (artifact_dir / "shell.log").read_text(errors="replace") if (artifact_dir / "shell.log").exists() else ""
    )
    diagnostics = [line for line in shell_log.splitlines() if FATAL_LOG.search(line)]
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
                "shell_log": str(artifact_dir / "shell.log"),
            },
        )
        (artifact_dir / "repair-request.md").write_text(
            "# Gnoblin lifecycle failure\n\n"
            f"Seed: `{seed}`\n\n"
            f"Replay: `python3 {SCRIPT} --replay {artifact_dir / 'repro.json'}`\n\n"
            "Inspect `shell.log`, `runner.log`, and `events.jsonl`; reproduce the failure, "
            "make the smallest source fix, then rerun this exact replay and the relevant "
            "headless integration checks. Keep the patch isolated and reviewable.\n"
        )
        print(f"FAIL: compositor/session did not survive; repro: {artifact_dir / 'repro.json'}", file=sys.stderr)
        return 1

    print(f"PASS: compositor survived seed {seed}; replayable run: {plan_path}", flush=True)
    return 0


def run_inside() -> int:
    if not os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"):
        raise RuntimeError("the lifecycle driver must run inside run-gnome-shell.sh")
    plan_path = Path(os.environ["GNOBLIN_LIFECYCLE_FUZZ_PLAN"])
    event_path = Path(os.environ["GNOBLIN_LIFECYCLE_FUZZ_EVENTS"])
    config_path = Path(os.environ["GNOBLIN_CONFIG"])
    plan = read_plan(plan_path)
    fixture_dir = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin-lifecycle-fuzz"
    fixture_dir.mkdir(parents=True, exist_ok=True)
    fixture = fixture_dir / "window-client"
    flags = subprocess.check_output(["pkg-config", "--cflags", "--libs", "gtk4"], text=True).split()
    subprocess.run(["cc", str(ROOT / "tests/window-lifecycle-client.c"), "-o", str(fixture), *flags], check=True)
    processes: dict[int, subprocess.Popen] = {}
    frame_policies: dict[int, list[int]] = {}

    def title_for(window_id: int) -> str:
        return f"Gnoblin Fuzz {window_id:04d}"

    def state_for(window_id: int) -> dict | None:
        return shell_window(title_for(window_id))

    def set_frame_policy(window_id: int, policy: list[int]) -> None:
        frame_policies[window_id] = policy
        write_frame_config(config_path, frame_policies)
        result = subprocess.run(["gnoblinctl", "config", "reload"], capture_output=True, text=True, timeout=15)
        if result.returncode:
            raise RuntimeError(f"could not apply window frame policy: {result.stderr.strip() or result.stdout.strip()}")

    def wait_frame(window_id: int, top: int) -> dict:
        deadline = time.monotonic() + 5
        state = None
        while time.monotonic() < deadline:
            state = state_for(window_id)
            if state and state["layout"]["border"][0] == top:
                return state
            time.sleep(0.04)
        raise TimeoutError(f"timed out waiting for native frame on window {window_id}; last state={state!r}")

    def prepare_frame(window_id: int) -> dict:
        title = title_for(window_id)
        expression = (
            "(()=>{const w=global.get_window_actors().find(a=>a.meta_window.title==="
            f"{json.dumps(title)})?.meta_window;if(!w)throw new Error('fuzz target disappeared');"
            "if(w.minimized)w.unminimize();if(w.fullscreen)w.unmake_fullscreen();"
            "w.activate(global.get_current_time());return true;})()"
        )
        eval_shell(expression)
        set_frame_policy(window_id, [3, 0, 0, 0, 0, 36, 2, 2, 2])
        return wait_frame(window_id, 36)

    def wait_window(window_id: int, present: bool) -> None:
        wait_for(
            lambda: state_for(window_id) is not None if present else state_for(window_id) is None,
            f"window {window_id} {'to map' if present else 'to close'}",
        )

    def close_client(window_id: int, op: str) -> None:
        process = processes[window_id]
        if process.poll() is None:
            process.send_signal(signal.SIGUSR1) if op == "graceful_close" else process.kill()
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=3)
        wait_window(window_id, False)

    def operate(action: dict) -> None:
        op = action["op"]
        window_id = action.get("window")
        if op == "open":
            title = title_for(window_id)
            process = subprocess.Popen([str(fixture), title], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            processes[window_id] = process
            wait_window(window_id, True)
            wait_frame(window_id, 36)
            return

        title = title_for(window_id)
        state = state_for(window_id)
        if state is None:
            raise RuntimeError(f"target window {window_id} is not mapped")
        title_js = json.dumps(title)
        window_expr = f"global.get_window_actors().find(a=>a.meta_window.title==={title_js})?.meta_window"
        if op in {"wm_close", "graceful_close", "abrupt_close"}:
            if op == "wm_close":
                eval_shell(f"(()=>{{{window_expr}.delete(global.get_current_time());return true;}})()")
                wait_window(window_id, False)
                processes[window_id].wait(timeout=3)
            else:
                close_client(window_id, op)
            return
        if op == "frame_click":
            state = prepare_frame(window_id)
            send_pointer("click", state["x"] + state["width"] - 20, state["y"] + 18)
            wait_window(window_id, False)
            processes[window_id].wait(timeout=3)
            return
        if op == "shell_shutdown":
            state = prepare_frame(window_id)
            send_pointer("move", state["x"] + state["width"] - 20, state["y"] + 18)
            time.sleep(0.05)
            pid_file = Path(os.environ["GNOBLIN_TEST_SHELL_PID_FILE"])
            shell_pid = int(pid_file.read_text())
            marker = {"requested": True, "shell_pid": shell_pid, "window": window_id}
            save_json(plan_path.parent / "expected-shell-exit.json", marker)
            os.kill(shell_pid, signal.SIGTERM)
            deadline = time.monotonic() + 10
            while Path(f"/proc/{shell_pid}").exists() and time.monotonic() < deadline:
                time.sleep(0.05)
            if Path(f"/proc/{shell_pid}").exists():
                raise TimeoutError(f"shell pid {shell_pid} did not exit after SIGTERM")
            marker["observed_exit"] = True
            save_json(plan_path.parent / "expected-shell-exit.json", marker)
            return
        if op == "frame_policy":
            policy = action["policy"]
            set_frame_policy(window_id, policy)
            if not state["minimized"] and not state["fullscreen"]:
                wait_frame(window_id, policy[5] if policy[0] else 0)
            return
        if op == "hover":
            state = prepare_frame(window_id)
            points = [
                (state["x"] + 1, state["y"] + 1),
                (state["x"] + 12, state["y"] + 18),
                (state["x"] + state["width"] // 2, state["y"] + 18),
                (state["x"] + state["width"] - 2, state["y"] + 18),
                (state["x"] + state["width"] - 2, state["y"] + state["height"] - 2),
            ]
            send_pointer("move", *points[action["point"]])
            return
        if op == "drag_resize":
            state = prepare_frame(window_id)
            x, y = state["x"] + state["width"] - 2, state["y"] + state["height"] - 2
            send_pointer("move", x, y)
            eval_shell(
                "(()=>{const C=imports.gi.Clutter,G=imports.gi.GLib,p=global.lifecycleFuzzPointer;"
                "p.notify_button(G.get_monotonic_time(),1,C.ButtonState.PRESSED);"
                f"p.notify_absolute_motion(G.get_monotonic_time(),{x + 18},{y + 16});"
                "p.notify_button(G.get_monotonic_time(),1,C.ButtonState.RELEASED);return true;})()"
            )
            return

        operations = {
            "activate": f"{window_expr}.activate(global.get_current_time())",
            "minimize": f"{window_expr}.minimize()",
            "unminimize": f"{window_expr}.unminimize()",
            "maximize": f"{window_expr}.maximize()",
            "unmaximize": f"{window_expr}.unmaximize()",
            "fullscreen": f"{window_expr}.make_fullscreen()",
            "unfullscreen": f"{window_expr}.unmake_fullscreen()",
            "resize": f"{window_expr}.move_resize_frame(false,{action['x']},{action['y']},{action['width']},{action['height']})",
        }
        if op not in operations:
            raise ValueError(f"unsupported operation: {op}")
        eval_shell(f"(()=>{{{operations[op]};return true;}})()")

    failure = None
    event_path.write_text("")
    shell_stopped = False
    try:
        for index, action in enumerate(plan["actions"]):
            with event_path.open("a") as events:
                events.write(json.dumps({"phase": "start", "index": index, "action": action}) + "\n")
            operate(action)
            if action["op"] == "shell_shutdown":
                shell_stopped = True
                with event_path.open("a") as events:
                    events.write(json.dumps({"phase": "done", "index": index, "action": action}) + "\n")
                break
            time.sleep(action.get("delay_ms", 0) / 1000)
            eval_shell("true")
            with event_path.open("a") as events:
                events.write(json.dumps({"phase": "done", "index": index, "action": action}) + "\n")
            if index % 25 == 0:
                print(f"lifecycle fuzz: action {index + 1}/{len(plan['actions'])}", flush=True)
        if not shell_stopped:
            for window_id, process in processes.items():
                if process.poll() is None:
                    close_client(window_id, "graceful_close")
            time.sleep(0.3)
            eval_shell("true")
    except Exception as error:
        failure = {"error": str(error), "traceback": traceback.format_exc(), "seed": plan["seed"]}
        save_json(plan_path.parent / "client-failure.json", failure)
        print(f"lifecycle fuzz failure: {error}\n{failure['traceback']}", file=sys.stderr, flush=True)
    finally:
        for process in processes.values():
            if process.poll() is None:
                process.kill()
                try:
                    process.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass

    if failure:
        return 1
    print(f"PASS: survived {len(plan['actions'])} lifecycle operations and compositor shutdown", flush=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, help="reproducible seed (default: random 32-bit seed)")
    parser.add_argument("--steps", type=int, default=300, help="random operations after the first window")
    parser.add_argument("--max-windows", type=int, default=6, help="maximum live fixture windows")
    parser.add_argument("--replay", type=Path, help="replay a saved plan.json or repro.json")
    parser.add_argument("--artifact-dir", type=Path, help="where to keep logs and reproduction data")
    parser.add_argument("--timeout", type=int, default=300, help="whole-session timeout in seconds")
    return parser.parse_args()


def main() -> int:
    if os.environ.get("GNOBLIN_LIFECYCLE_FUZZ_INNER") == "1":
        return run_inside()
    return run_parent(parse_args())


if __name__ == "__main__":
    sys.exit(main())
