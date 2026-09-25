#!/usr/bin/env python3
"""Exercise installed desktop applications in real isolated Gnoblin sessions."""

from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import select
import shutil
import signal
import subprocess
import sys
import threading
import time
import traceback

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = Path(__file__).resolve()
sys.path.insert(0, str(ROOT / "tests"))
from gnoblin_test_session import (  # noqa: E402
    compile_minimal_testing_shell,
    eval_shell,
    send_pointer,
    shell_windows,
    wait_for,
    window_by_sequence_expression,
)

FATAL_LOG = re.compile(
    r"GNOME Shell-CRITICAL|(?:Clutter|Mutter|Meta)-CRITICAL|JS ERROR|"
    r"Traceback \(most recent call last\)|assertion .* failed|SIG(SEGV|ABRT)|"
    r"segmentation fault|runtime check failed|core dumped",
    re.IGNORECASE,
)
FRAME_POLICY = [3, 0, 0, 0, 0, 36, 2, 2, 2]


def save_json(path: Path, value: object) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def default_artifact_dir(index: int) -> Path:
    state_home = Path(os.environ.get("XDG_STATE_HOME", Path.home() / ".local/state"))
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    return state_home / "gnoblin" / "app-e2e" / f"{stamp}-shard-{index:02d}"


def run_parent() -> int:
    index = int(os.environ["GNOBLIN_E2E_SHARD_INDEX"])
    count = int(os.environ["GNOBLIN_E2E_SHARD_COUNT"])
    catalog_path = Path(os.environ["GNOBLIN_E2E_CATALOG"])
    catalog = json.loads(catalog_path.read_text())
    artifact_dir = Path(os.environ.get("GNOBLIN_E2E_ARTIFACT_DIR", default_artifact_dir(index)))
    artifact_dir.mkdir(parents=True, exist_ok=True)
    prepared_shard = os.environ.get("GNOBLIN_E2E_PREPARED_SHARD")
    shard_path = Path(prepared_shard) if prepared_shard else artifact_dir / "shard.json"
    if not prepared_shard:
        subprocess.run(
            [
                sys.executable,
                str(ROOT / "tests/e2e/app-catalog.py"),
                "--catalog-in",
                str(catalog_path),
                "--shard-index",
                str(index),
                "--shard-count",
                str(count),
                "--shard-output",
                str(shard_path),
            ],
            check=True,
        )
    shard = json.loads(shard_path.read_text())
    if shard.get("shard") != {"index": index, "count": count} or not shard.get("apps"):
        raise RuntimeError(f"invalid or empty catalog shard {index}/{count}")
    apps = shard["apps"]
    run_info = {
        "shard": shard["shard"],
        "app_count": len(apps),
        "git_head": subprocess.run(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True
        ).stdout.strip(),
        "started_utc": datetime.now(timezone.utc).isoformat(),
        "prefix": os.environ.get("GNOBLIN_PREFIX", str(ROOT / "install")),
        "catalog_generated_utc": catalog["generated_utc"],
        "fedora_appstream_sha256": catalog.get("fedora_appstream_sha256"),
        "extra_monitor": os.environ.get("GNOBLIN_E2E_EXTRA_MONITOR") or None,
    }
    save_json(artifact_dir / "run.json", run_info)

    state_dir = artifact_dir / "private-state"
    state_dir.mkdir(mode=0o700)
    env = os.environ.copy()
    env.update(
        {
            "GNOBLIN_TEST_UNSAFE_MODE": "1",
            "GNOBLIN_TEST_XWAYLAND": "1",
            "GNOBLIN_TEST_CLIENT": str(SCRIPT),
            "GNOBLIN_APP_E2E_INNER": "1",
            "GNOBLIN_E2E_SHARD_FILE": str(shard_path),
            "GNOBLIN_E2E_EVENTS": str(artifact_dir / "events.jsonl"),
            "GNOBLIN_E2E_SUMMARY": str(artifact_dir / "summary.json"),
            "GNOBLIN_E2E_SCREENSHOTS": str(artifact_dir / "screenshots"),
            "GNOBLIN_E2E_ARTIFACT_DIR": str(artifact_dir),
            "GNOBLIN_STATE_DIR": str(state_dir),
            "GNOBLIN_TEST_CLIENT_EXPECTS_SHELL_EXIT": "0",
            "MONITOR": os.environ.get("GNOBLIN_E2E_MONITOR", "1280x800"),
            "PYTHONUNBUFFERED": "1",
        }
    )
    if os.environ.get("GNOBLIN_E2E_EXTRA_MONITOR"):
        env["EXTRA_MONITOR"] = os.environ["GNOBLIN_E2E_EXTRA_MONITOR"]
    timeout = int(os.environ.get("GNOBLIN_E2E_TIMEOUT", "7200"))
    print(f"Gnoblin app E2E: shard={index}/{count}, apps={len(apps)}", flush=True)
    print(f"Artifacts: {artifact_dir}", flush=True)

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
        try:
            return_code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            process.send_signal(signal.SIGTERM)
            try:
                return_code = process.wait(timeout=8)
            except subprocess.TimeoutExpired:
                process.kill()
                return_code = process.wait()
            save_json(artifact_dir / "timeout.json", {"timeout_seconds": timeout})
        reader.join()

    published_log = state_dir / "gnome-shell-last.log"
    if published_log.exists():
        shutil.copy2(published_log, artifact_dir / "shell.log")
    shell_log = (
        (artifact_dir / "shell.log").read_text(errors="replace") if (artifact_dir / "shell.log").exists() else ""
    )
    diagnostics = [line for line in shell_log.splitlines() if FATAL_LOG.search(line)]
    if return_code or diagnostics or (artifact_dir / "timeout.json").exists():
        save_json(
            artifact_dir / "failure.json",
            {
                "return_code": return_code,
                "diagnostics": diagnostics[-80:],
                "events": str(artifact_dir / "events.jsonl"),
                "shell_log": str(artifact_dir / "shell.log"),
            },
        )
        repair_request = artifact_dir / "repair-request.md"
        if not repair_request.exists():
            repair_request.write_text(
                "# Gnoblin application compatibility failure\n\n"
                f"Shard: `{index}/{count}`\n\n"
                f"Replay: `GNOBLIN_E2E_CATALOG={catalog_path} GNOBLIN_E2E_SHARD_INDEX={index} "
                f"GNOBLIN_E2E_SHARD_COUNT={count} python3 {SCRIPT}`\n\n"
                "Inspect the event stream, per-application logs, screenshots and shell log. "
                "Reduce the failing app/state sequence to a small reproducer before changing "
                "Mutter or Gnoblin. Re-run the exact app and operation after the fix.\n"
            )
        print(f"FAIL: Gnoblin app compatibility shard failed; artifacts: {artifact_dir}", file=sys.stderr)
        return 1
    summary = json.loads((artifact_dir / "summary.json").read_text())
    print(f"PASS: compositor survived shard {index}/{count}; outcomes={summary['outcomes']}", flush=True)
    return 0


def window_expression(sequence: int) -> str:
    return window_by_sequence_expression(sequence)


def window_state(sequence: int) -> dict | None:
    expression = window_expression(sequence)
    return eval_shell(
        f"(()=>{{const w={expression};if(!w)return null;const r=w.get_frame_rect();"
        f"const a=global.get_window_actors().find(a=>a.meta_window===w);"
        "return {sequence:w.get_stable_sequence(),title:w.get_title(),pid:w.get_pid(),"
        "type:w.get_window_type(),x:r.x,y:r.y,width:r.width,height:r.height,"
        "monitor:w.get_monitor(),"
        "ready:w.is_ready(),mapped:a?.is_mapped()??false,"
        "minimized:w.minimized,fullscreen:w.fullscreen,"
        "maximized:!!w.get_maximize_flags(),"
        "can_move:w.allows_move(),can_resize:w.allows_resize(),"
        "can_maximize:w.can_maximize(),can_minimize:w.can_minimize(),"
        "focused:global.display.focus_window===w,"
        "layout:imports.gi.Meta.gnoblin_window_frame_get(w).recursiveUnpack()};})()"
    )


def mutate(sequence: int, body: str) -> object:
    expression = window_expression(sequence)
    return eval_shell(
        f"(()=>{{const w={expression};if(!w)throw new Error('window disappeared');{body};return true;}})()"
    )


def write_event(path: Path, event: dict) -> None:
    event["time_utc"] = datetime.now(timezone.utc).isoformat()
    with path.open("a") as stream:
        stream.write(json.dumps(event, sort_keys=True) + "\n")


def screenshot(app: dict, directory: Path) -> str | None:
    directory.mkdir(parents=True, exist_ok=True)
    digest = hashlib.sha256(app["app_id"].encode()).hexdigest()[:12]
    path = directory / f"{app['source']}-{digest}.png"
    result = subprocess.run(
        [
            "gdbus",
            "call",
            "--session",
            "--dest",
            "org.gnome.Shell.Screenshot",
            "--object-path",
            "/org/gnome/Shell/Screenshot",
            "--method",
            "org.gnome.Shell.Screenshot.Screenshot",
            "false",
            "false",
            str(path),
        ],
        capture_output=True,
        text=True,
        timeout=12,
    )
    return str(path) if result.returncode == 0 and path.is_file() else None


def shell_drag(start_x: int, start_y: int, end_x: int, end_y: int) -> None:
    send_pointer("move", start_x, start_y)
    time.sleep(0.04)
    send_pointer("press", start_x, start_y)
    time.sleep(0.06)
    steps = max(1, min(12, max(abs(end_x - start_x), abs(end_y - start_y)) // 12))
    try:
        for step in range(1, steps + 1):
            x = round(start_x + (end_x - start_x) * step / steps)
            y = round(start_y + (end_y - start_y) * step / steps)
            send_pointer("move", x, y)
            time.sleep(0.035)
    finally:
        send_pointer("release", end_x, end_y)
    time.sleep(0.06)


def app_command(app: dict) -> list[str]:
    if app["source"] == "flathub-popular":
        return ["flatpak", "run", "--unshare=network", app["launch"]]
    return ["gtk-launch", app["launch"]]


def close_sequence(sequence: int, timeout: float = 10) -> str:
    state = window_state(sequence)
    if state is None:
        return "already-closed"
    if state["layout"]["border"][0]:
        close_x = state["x"] + max(12, state["width"] - 20)
        close_y = state["y"] + 18
        send_pointer("move", close_x, close_y)
        time.sleep(0.025)
        send_pointer("click", close_x, close_y)
        try:
            wait_for(lambda: window_state(sequence) is None, "titlebar close button", timeout=2)
            send_pointer("move", 4, 780)
            return "titlebar-close-button"
        except TimeoutError:
            pass
    if window_state(sequence) is not None:
        mutate(sequence, "w.delete(global.get_current_time())")
        wait_for(lambda: window_state(sequence) is None, "window close", timeout=timeout)
    send_pointer("move", 4, 780)
    return "window-delete-fallback"


def run_one_app(app: dict, events_path: Path, screenshot_dir: Path, console_dir: Path, launch_timeout: float) -> dict:
    baseline = {window["sequence"] for window in shell_windows()}
    if app["source"] == "flathub-popular":
        # The private document-portal stub returns this mount point. Flatpak's
        # bubblewrap expects a per-app source directory even when it is empty.
        document_mount = Path(os.environ["XDG_CONFIG_HOME"]).parent / "doc" / "by-app" / app["install"]
        document_mount.mkdir(parents=True, exist_ok=True)
    log_path = console_dir / f"{hashlib.sha256(app['app_id'].encode()).hexdigest()[:12]}.log"
    console_dir.mkdir(parents=True, exist_ok=True)
    started = time.monotonic()
    state = {
        "app_id": app["app_id"],
        "name": app["name"],
        "source": app["source"],
        "status": "started",
        "windows": [],
        "operations": [],
        "console_log": str(log_path),
    }
    with log_path.open("w") as log:
        try:
            process = subprocess.Popen(
                app_command(app), stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT, start_new_session=True
            )
        except Exception as error:
            state.update(status="launch-error", error=str(error))
            write_event(events_path, state)
            return state

        try:

            def app_windows() -> list[dict]:
                candidates = [
                    window for window in shell_windows() if window["sequence"] not in baseline and window["title"]
                ]
                if any(window["ready"] is not None for window in candidates):
                    return [window for window in candidates if window["ready"]]
                return [window for window in candidates if window["mapped"]]

            try:
                new_windows = wait_for(
                    app_windows, f"{app['app_id']} to map a Wayland/X11 window", timeout=launch_timeout
                )
                time.sleep(0.6)
                new_windows = app_windows()
                if not new_windows:
                    raise TimeoutError(f"{app['app_id']} closed its first window before the test began")
            except TimeoutError as error:
                state.update(status="no-window", error=str(error), process_exit_code=process.poll())
                return state

            new_windows.sort(key=lambda window: window["sequence"])
            state["screenshot"] = screenshot(app, screenshot_dir)
            state["windows"] = [window["sequence"] for window in new_windows]
            state["window_observations"] = [window_state(window["sequence"]) for window in new_windows]
            state["status"] = "window-mapped"
            write_event(
                events_path,
                {**state, "phase": "window-mapped", "elapsed_seconds": round(time.monotonic() - started, 3)},
            )

            control_failed = False
            for mapped in new_windows:
                sequence = mapped["sequence"]
                current = window_state(sequence)
                if current is None:
                    state["operations"].append(
                        {"window": sequence, "operation": "observe", "status": "closed-before-test"}
                    )
                    control_failed = True
                    continue

                def record(operation: str, status: str, **details: object) -> None:
                    result = {"window": sequence, "operation": operation, "status": status, **details}
                    state["operations"].append(result)
                    write_event(events_path, {"phase": "operation", "app_id": app["app_id"], **result})

                def invoke(
                    operation: str, body: str, verify, timeout: float = 2.5, capability: str | None = None
                ) -> str:
                    nonlocal control_failed
                    current_state = window_state(sequence)
                    if capability and current_state and not current_state[capability]:
                        record(operation, "not-supported", capability=capability)
                        return "not-supported"
                    try:
                        mutate(sequence, body)
                        observed = wait_for(verify, operation, timeout=timeout)
                        record(operation, "observed", observed=observed)
                        return "observed"
                    except TimeoutError as error:
                        record(operation, "not-observed", error=str(error))
                        if capability and current_state and current_state[capability]:
                            control_failed = True
                        return "not-observed"
                    except Exception as error:
                        record(operation, "unsupported-or-error", error=str(error))
                        if capability and current_state and current_state[capability]:
                            control_failed = True
                        return "unsupported-or-error"

                invoke(
                    "activate",
                    "w.activate(global.get_current_time())",
                    lambda: (lambda after: after is not None and after["focused"])(window_state(sequence)),
                    timeout=2,
                )

                # App defaults may map maximized or fullscreen. Return to the
                # normal state before asserting that move/resize controls work.
                current = window_state(sequence)
                if current and current["fullscreen"]:
                    invoke(
                        "restore-initial-fullscreen",
                        "w.unmake_fullscreen()",
                        lambda: (lambda after: after is not None and not after["fullscreen"])(window_state(sequence)),
                        capability="can_move",
                    )
                current = window_state(sequence)
                if current and current["maximized"]:
                    invoke(
                        "restore-initial-maximized",
                        "w.unmaximize()",
                        lambda: (lambda after: after is not None and not after["maximized"])(window_state(sequence)),
                        capability="can_move",
                    )

                try:
                    mutate(
                        sequence,
                        "imports.gi.Meta.gnoblin_window_frame_set(w,"
                        f"new imports.gi.GLib.Variant('(iiiiiiiii)',{json.dumps(FRAME_POLICY)}))",
                    )
                    wait_for(
                        lambda: (window_state(sequence) or {}).get("layout", {}).get("border", [0])[0] == 36,
                        "Gnoblin native frame",
                        timeout=2,
                    )
                    record("native-frame", "observed")
                except Exception as error:
                    record("native-frame", "unsupported-or-error", error=str(error))

                for name, x, y in (
                    ("move-center", 48, 76),
                    ("move-edge", 300, 140),
                    ("move-small", 24, 48),
                ):
                    before = window_state(sequence)
                    if not before:
                        record(name, "window-disappeared")
                        control_failed = True
                        break
                    invoke(
                        name,
                        f"w.move_frame(false,{x},{y})",
                        lambda: (
                            lambda after: after is not None and (after["x"], after["y"]) != (before["x"], before["y"])
                        )(window_state(sequence)),
                        timeout=1.5,
                        capability="can_move",
                    )
                    for size_name, width, height in (("center", 700, 440), ("edge", 960, 620), ("small", 300, 220)):
                        before = window_state(sequence)
                        if not before:
                            record(f"resize-{size_name}", "window-disappeared")
                            break
                        invoke(
                            f"resize-{size_name}",
                            f"w.move_resize_frame(false,{x},{y},{width},{height})",
                            lambda: (
                                lambda after: (
                                    after is not None
                                    and (after["width"], after["height"]) != (before["width"], before["height"])
                                )
                            )(window_state(sequence)),
                            timeout=1.5,
                            capability="can_resize",
                        )

                monitor_count = eval_shell("global.display.get_n_monitors()")
                if monitor_count > 1:
                    invoke(
                        "move-to-secondary-monitor",
                        "w.move_to_monitor(1)",
                        lambda: (lambda after: after is not None and after["monitor"] == 1)(window_state(sequence)),
                        timeout=2,
                        capability="can_move",
                    )
                    invoke(
                        "move-back-to-primary-monitor",
                        "w.move_to_monitor(0)",
                        lambda: (lambda after: after is not None and after["monitor"] == 0)(window_state(sequence)),
                        timeout=2,
                        capability="can_move",
                    )

                current = window_state(sequence)
                if current and current["layout"]["border"][0]:
                    try:
                        x, y, width, height = (current[k] for k in ("x", "y", "width", "height"))
                        shell_drag(
                            x + min(100, width // 2), y + 18, min(1230, x + min(100, width // 2) + 44), min(750, y + 54)
                        )
                        dragged = wait_for(
                            lambda: (lambda after: after if after and (after["x"], after["y"]) != (x, y) else None)(
                                window_state(sequence)
                            ),
                            "titlebar drag movement",
                            timeout=1.5,
                        )
                        record("titlebar-drag", "observed", state=dragged)
                    except Exception as error:
                        record("titlebar-drag", "unsupported-or-error", error=str(error))
                        if current["can_move"]:
                            control_failed = True
                    before = None
                    try:
                        before = window_state(sequence)
                        if before and before["can_resize"]:
                            x, y, width, height = (before[k] for k in ("x", "y", "width", "height"))
                            shell_drag(
                                x + width - 2, y + height - 2, min(1276, x + width + 38), min(796, y + height + 30)
                            )
                            resized = wait_for(
                                lambda: (
                                    lambda after: (
                                        after
                                        if after and (after["width"], after["height"]) != (width, height)
                                        else None
                                    )
                                )(window_state(sequence)),
                                "native-frame resize handle",
                                timeout=1.5,
                            )
                            record("resize-handle-drag", "observed", state=resized)
                        elif before:
                            record("resize-handle-drag", "not-supported", capability="can_resize")
                    except Exception as error:
                        record("resize-handle-drag", "unsupported-or-error", error=str(error))
                        if before and before["can_resize"]:
                            control_failed = True

                invoke(
                    "maximize",
                    "w.maximize()",
                    lambda: (lambda after: after is not None and after["maximized"])(window_state(sequence)),
                    capability="can_maximize",
                )
                invoke(
                    "unmaximize",
                    "w.unmaximize()",
                    lambda: (lambda after: after is not None and not after["maximized"])(window_state(sequence)),
                    capability="can_maximize",
                )
                invoke(
                    "minimize",
                    "w.minimize()",
                    lambda: (lambda after: after is not None and after["minimized"])(window_state(sequence)),
                    capability="can_minimize",
                )
                invoke(
                    "unminimize",
                    "w.unminimize()",
                    lambda: (lambda after: after is not None and not after["minimized"])(window_state(sequence)),
                    capability="can_minimize",
                )
                invoke(
                    "fullscreen",
                    "w.make_fullscreen()",
                    lambda: (lambda after: after is not None and after["fullscreen"])(window_state(sequence)),
                )
                invoke(
                    "unfullscreen",
                    "w.unmake_fullscreen()",
                    lambda: (lambda after: after is not None and not after["fullscreen"])(window_state(sequence)),
                )

            for sequence in state["windows"]:
                try:
                    initial = window_state(sequence)
                    close_method = close_sequence(sequence)
                    close_result = {"window": sequence, "operation": "close", "status": close_method}
                    state["operations"].append(close_result)
                    write_event(events_path, {"phase": "operation", "app_id": app["app_id"], **close_result})
                    if initial and initial["layout"]["border"][0] and close_method != "titlebar-close-button":
                        control_failed = True
                except Exception as error:
                    state["operations"].append(
                        {"window": sequence, "operation": "close", "status": "error", "error": str(error)}
                    )
            close_failed = any(
                operation.get("operation") == "close" and operation.get("status") == "error"
                for operation in state["operations"]
            )
            state["status"] = (
                "control-failed"
                if close_failed or control_failed
                else ("exercised" if state["windows"] else "no-window")
            )
            state["elapsed_seconds"] = round(time.monotonic() - started, 3)
            return state
        except BaseException:
            process.send_signal(signal.SIGTERM) if process.poll() is None else None
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                process.kill()
            raise
        finally:
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=3)


def run_inside() -> int:
    if not os.environ.get("WAYLAND_DISPLAY", "").startswith("gnoblin-gs-"):
        raise RuntimeError("the app suite must run inside scripts/run-gnome-shell.sh")
    shard_path = Path(os.environ["GNOBLIN_E2E_SHARD_FILE"])
    shard = json.loads(shard_path.read_text())
    events_path = Path(os.environ["GNOBLIN_E2E_EVENTS"])
    summary_path = Path(os.environ["GNOBLIN_E2E_SUMMARY"])
    screenshot_dir = Path(os.environ["GNOBLIN_E2E_SCREENSHOTS"])
    console_dir = Path(os.environ["GNOBLIN_E2E_ARTIFACT_DIR"]) / "application-logs"
    event_path = events_path
    event_path.parent.mkdir(parents=True, exist_ok=True)
    event_path.write_text("")
    panel_binary = compile_minimal_testing_shell(Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin-app-e2e")
    panel = subprocess.Popen([str(panel_binary)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
    try:
        assert panel.stdout is not None
        readable, _, _ = select.select([panel.stdout], [], [], 12)
        line = panel.stdout.readline().strip() if readable else ""
        if line != "GNOBLIN_TEST_SHELL_READY":
            if panel.poll() is None:
                panel.terminate()
                try:
                    panel.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    panel.kill()
                    panel.wait(timeout=2)
            stderr = panel.stderr.read() if panel.stderr else ""
            raise RuntimeError(f"minimal layer-shell panel did not map: {line!r} {stderr.strip()}")
        write_event(events_path, {"phase": "test-shell-ready", "panel_pid": panel.pid})
        outcomes = []
        install_report = os.environ.get("GNOBLIN_E2E_INSTALL_REPORT")
        install_results = {}
        if install_report and Path(install_report).is_file():
            install_results = {
                result["app_id"]: result for result in json.loads(Path(install_report).read_text()).get("apps", [])
            }
        launch_timeout = float(os.environ.get("GNOBLIN_E2E_LAUNCH_TIMEOUT", "25"))
        for index, app in enumerate(shard["apps"], start=1):
            print(f"app E2E [{index}/{len(shard['apps'])}] {app['source']} {app['app_id']}", flush=True)
            installation = install_results.get(app["app_id"])
            if installation and installation.get("status") != "installed":
                outcome = {
                    "app_id": app["app_id"],
                    "name": app["name"],
                    "source": app["source"],
                    "status": "install-failed",
                    "installation": installation,
                }
                outcomes.append(outcome)
                write_event(events_path, {"phase": "application-complete", **outcome})
                continue
            outcome = run_one_app(app, events_path, screenshot_dir, console_dir, launch_timeout)
            outcomes.append(outcome)
            write_event(events_path, {"phase": "application-complete", **outcome})
            if index % 10 == 0:
                eval_shell("true")
        counts: dict[str, int] = {}
        for outcome in outcomes:
            counts[outcome["status"]] = counts.get(outcome["status"], 0) + 1
        failed_apps = [outcome["app_id"] for outcome in outcomes if outcome["status"] != "exercised"]
        summary = {
            "shard": shard["shard"],
            "catalog_generated_utc": shard["catalog_generated_utc"],
            "fedora_appstream_sha256": shard.get("fedora_appstream_sha256"),
            "requested_apps": len(shard["apps"]),
            "outcomes": counts,
            "failed_apps": failed_apps,
            "apps": outcomes,
            "completed_utc": datetime.now(timezone.utc).isoformat(),
        }
        save_json(summary_path, summary)
        print(f"E2E shard outcomes: {json.dumps(counts, sort_keys=True)}", flush=True)
        if failed_apps:
            repair_path = Path(os.environ["GNOBLIN_E2E_ARTIFACT_DIR"]) / "repair-request.md"
            repair_path.write_text(
                "# Gnoblin application compatibility failures\n\n"
                f"Shard: `{shard['shard']['index']}/{shard['shard']['count']}`\n\n"
                "Failed application IDs:\n\n"
                + "\n".join(f"- `{app_id}`" for app_id in failed_apps)
                + "\n\nInspect each application console log, operation event, screenshot and the shell log. "
                "Reproduce one app at a time, then reduce the failing state sequence before patching.\n"
            )
        return 1 if failed_apps else 0
    finally:
        if panel.poll() is None:
            panel.terminate()
            try:
                panel.wait(timeout=2)
            except subprocess.TimeoutExpired:
                panel.kill()
                panel.wait(timeout=2)


def main() -> int:
    if os.environ.get("GNOBLIN_APP_E2E_INNER") == "1":
        return run_inside()
    return run_parent()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        failure = {"error": str(error), "traceback": traceback.format_exc()}
        summary = os.environ.get("GNOBLIN_E2E_SUMMARY")
        if summary:
            save_json(Path(summary).with_name("client-failure.json"), failure)
        print(f"app E2E failure: {error}\n{failure['traceback']}", file=sys.stderr, flush=True)
        raise SystemExit(1)
