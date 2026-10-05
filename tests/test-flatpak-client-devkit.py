#!/usr/bin/env python3
"""Exercise a sandboxed Flatpak client's window lifecycle in nested Gnoblin."""

from __future__ import annotations

import json
import os
import pathlib
import shlex
import signal
import subprocess
import sys
import time

ROOT = pathlib.Path(__file__).resolve().parents[1]
APP_ID = "org.gnome.TextEditor"
APP_DESKTOP_ID = f"{APP_ID}.desktop"


def gnoblinctl(*args: str) -> subprocess.CompletedProcess[str]:
    prefix = pathlib.Path(os.environ.get("GNOBLIN_PREFIX", ROOT / "install"))
    cli = pathlib.Path(os.environ.get("GNOBLINCTL", prefix / "bin" / "gnoblinctl"))
    return subprocess.run(
        [str(cli), "--timeout", "5", *args],
        check=True,
        capture_output=True,
        text=True,
        timeout=8,
    )


def windows_for_app() -> list[dict[str, object]]:
    result = gnoblinctl("window", "list", "--json")
    windows = json.loads(result.stdout).get("windows", [])
    return [window for window in windows if window.get("app_id") in (APP_ID, APP_DESKTOP_ID)]


def wait_for(description: str, predicate, timeout: float = 60.0):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(0.25)
    raise TimeoutError(f"timed out waiting for {description}")


def run_inside_devkit() -> int:
    if not os.environ.get("GNOBLIN_DEVKIT_RUNTIME_LOG"):
        raise RuntimeError("the inner smoke test must run from the Gnoblin devkit")
    if not os.environ.get("WAYLAND_DISPLAY"):
        raise RuntimeError("the Gnoblin devkit did not set WAYLAND_DISPLAY")

    artifact_dir = pathlib.Path(os.environ.get("GNOBLIN_ARTIFACT_DIR", ROOT / "build" / "logs"))
    artifact_dir.mkdir(parents=True, exist_ok=True)
    app_log = artifact_dir / "flatpak-text-editor.log"
    document_mount = pathlib.Path(os.environ["XDG_RUNTIME_DIR"]) / "doc"
    (document_mount / "by-app" / APP_ID).mkdir(parents=True, exist_ok=True)
    client: subprocess.Popen[bytes] | None = None
    open_window_id: str | None = None

    try:
        with app_log.open("wb") as output:
            # This window-lifecycle check does not need network isolation.
            client = subprocess.Popen(
                ["flatpak", "run", "--system", "--share=network", APP_ID],
                stdout=output,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )

            def find_window() -> dict[str, object] | None:
                if client and client.poll() not in (None, 0):
                    raise RuntimeError(f"Flatpak client exited with status {client.returncode}; see {app_log}")
                matches = windows_for_app()
                return matches[0] if matches else None

            window = wait_for("the Flatpak application window", find_window)
            open_window_id = str(window["id"])
            if window.get("minimized") is not False:
                raise AssertionError(f"new application window was unexpectedly minimized: {window}")

            gnoblinctl("window", "minimize", open_window_id)
            wait_for(
                "the window to become minimized",
                lambda: next(
                    (item for item in windows_for_app() if item["id"] == open_window_id and item.get("minimized")),
                    None,
                ),
            )

            gnoblinctl("window", "unminimize", open_window_id)
            wait_for(
                "the window to be restored",
                lambda: next(
                    (
                        item
                        for item in windows_for_app()
                        if item["id"] == open_window_id and item.get("minimized") is False
                    ),
                    None,
                ),
            )

            gnoblinctl("window", "close", open_window_id)
            wait_for("the closed window to disappear", lambda: not windows_for_app())
            open_window_id = None
            print(f"PASS: Flatpak {APP_ID} opened, minimized, restored, and closed in Gnoblin")
            return 0
    finally:
        if open_window_id is not None:
            try:
                gnoblinctl("window", "close", open_window_id)
            except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                pass
        if client is not None and client.poll() is None:
            try:
                os.killpg(client.pid, signal.SIGTERM)
                client.wait(timeout=5)
            except (ProcessLookupError, subprocess.TimeoutExpired):
                try:
                    os.killpg(client.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass


def main() -> int:
    if len(sys.argv) == 2 and sys.argv[1] == "--inside":
        return run_inside_devkit()
    if len(sys.argv) != 1:
        print(f"Usage: {sys.argv[0]} [--inside]", file=sys.stderr)
        return 2

    subprocess.run(
        ["flatpak", "info", "--system", APP_ID],
        check=True,
        stdout=subprocess.DEVNULL,
    )
    env = os.environ.copy()
    env["GNOBLIN_DEVKIT_KEEP_SESSION"] = "1"
    env["GNOBLIN_DEVKIT_EXEC"] = shlex.join([sys.executable, str(__file__), "--inside"])
    env["GNOBLIN_TEST_FLATPAK_PORTAL"] = "1"
    return subprocess.run([str(ROOT / "scripts" / "run-gnoblin-devkit.sh")], env=env).returncode


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (
        AssertionError,
        RuntimeError,
        TimeoutError,
        subprocess.CalledProcessError,
        subprocess.TimeoutExpired,
    ) as exc:
        print(f"Flatpak devkit smoke failed: {exc}", file=sys.stderr)
        raise SystemExit(1) from exc
