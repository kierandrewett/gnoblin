#!/usr/bin/env python3
"""Verify native workspace animation events reach socket clients."""

import json
import os
from pathlib import Path
import select
import socket
import subprocess
import time


EVENTS = ["gnoblin.animation.started", "gnoblin.animation.finished"]
ANIMATION = "gnome-workspace-switch"
WINDOW_TITLE = "Untrusted Activation Target"
runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
socket_path = Path(os.environ["GNOBLIN_COMPOSITOR_SOCKET"])
ctl = os.environ["GNOBLIN_DEVKIT_CTL"]
client_binary = os.environ["GNOBLIN_FOCUS_TEST_CLIENT"]
request_path = runtime_dir / "request-workspace-animation"
request_path.unlink(missing_ok=True)


class JsonLines:
    def __init__(self, connection):
        self.connection = connection
        self.buffer = bytearray()

    def send(self, value):
        self.connection.sendall(json.dumps(value).encode("utf-8") + b"\n")

    def receive(self, timeout):
        deadline = time.monotonic() + timeout
        while True:
            newline = self.buffer.find(b"\n")
            if newline >= 0:
                line = bytes(self.buffer[:newline])
                del self.buffer[: newline + 1]
                return json.loads(line)
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.connection], [], [], remaining)[0]:
                raise TimeoutError("timed out waiting for a compositor event")
            chunk = self.connection.recv(65536)
            if not chunk:
                raise RuntimeError("compositor closed the event connection")
            self.buffer.extend(chunk)


def run_ctl(*arguments):
    return subprocess.run(
        [ctl, "--timeout", "2", *arguments],
        check=True,
        capture_output=True,
        text=True,
        timeout=4,
    )


def workspace_records():
    result = run_ctl("--json", "workspace", "list")
    value = json.loads(result.stdout)
    if isinstance(value, list):
        return value
    if isinstance(value, dict) and isinstance(value.get("workspaces"), list):
        return value["workspaces"]
    raise AssertionError(f"unexpected workspace snapshot: {value!r}")


def window_is_ready():
    result = run_ctl("--json", "window", "list")
    value = json.loads(result.stdout)
    windows = value if isinstance(value, list) else value.get("windows", [])
    return any(window.get("title") == WINDOW_TITLE for window in windows)


def receive_animation(stream, name, timeout=8):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        event = stream.receive(max(0.1, deadline - time.monotonic()))
        if event.get("event") not in EVENTS:
            continue
        if event.get("animation_event") != "workspace-switch":
            continue
        if event.get("event") != name:
            raise AssertionError(f"expected {name}, received {event!r}")
        return event
    raise TimeoutError(f"timed out waiting for {name}")


connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
connection.connect(str(socket_path))
stream = JsonLines(connection)
hello = stream.receive(3)
assert hello.get("event") == "hello", hello
api_minor = hello.get("api_minor", -1)
assert api_minor >= 18, f"animation events require API 1.18; got {api_minor}"
assert all(name in hello.get("events", []) for name in EVENTS), hello
stream.send(
    {
        "op": "events",
        "api_version": {"major": 1, "minor": api_minor},
        "events": EVENTS,
    }
)
subscribed = stream.receive(3)
assert subscribed.get("event") == "subscribed", subscribed

client = subprocess.Popen(
    [client_binary, str(request_path)],
    env=os.environ.copy(),
    stdout=subprocess.DEVNULL,
    stderr=subprocess.PIPE,
    text=True,
)
switches = []
try:
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline and not window_is_ready():
        if client.poll() is not None:
            stderr = client.stderr.read() if client.stderr else ""
            raise RuntimeError(f"workspace animation client exited early: {stderr}")
        time.sleep(0.05)
    assert window_is_ready(), "Wayland test window did not appear"

    active = next((workspace for workspace in workspace_records() if workspace.get("active")), None)
    assert active and active.get("id") and active.get("number"), f"active workspace missing from snapshot: {active!r}"
    source_id = active["id"]
    source_number = active["number"]
    destination_id = "devkit-animation-target"
    run_ctl(
        "workspace",
        "create",
        "--id",
        destination_id,
        "--name",
        "Animation Target",
    )

    first_switch = subprocess.Popen(
        [ctl, "workspace", "switch", "--id", destination_id],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    switches.append(first_switch)
    first_started = receive_animation(stream, EVENTS[0])
    assert first_started.get("animation") == ANIMATION, first_started
    assert first_started.get("cancelled") is False, first_started
    assert first_started.get("from_workspace") == source_id, first_started
    assert first_started.get("to_workspace") == destination_id, first_started
    assert first_started.get("target") == destination_id, first_started
    assert first_started.get("direction") in {
        "left",
        "right",
        "up",
        "down",
        "up-left",
        "up-right",
        "down-left",
        "down-right",
    }, first_started
    assert first_started.get("sequence", 0) > 0, first_started
    assert first_started.get("time", 0) > 0, first_started

    second_switch = subprocess.Popen(
        [ctl, "workspace", "switch", "--number", str(source_number)],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    switches.append(second_switch)
    first_finished = receive_animation(stream, EVENTS[1])
    assert first_finished.get("animation") == ANIMATION, first_finished
    assert first_finished.get("from_workspace") == source_id, first_finished
    assert first_finished.get("to_workspace") == destination_id, first_finished
    assert first_finished.get("cancelled") is True, first_finished

    second_started = receive_animation(stream, EVENTS[0])
    assert second_started.get("from_workspace") == destination_id, second_started
    assert second_started.get("to_workspace") == source_id, second_started
    second_finished = receive_animation(stream, EVENTS[1])
    assert second_finished.get("from_workspace") == destination_id, second_finished
    assert second_finished.get("to_workspace") == source_id, second_finished
    assert second_finished.get("cancelled") is False, second_finished

    for process in switches:
        process.wait(timeout=8)
        if process.returncode != 0:
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"workspace switch failed: {stderr}")

    print("PASS: workspace animation lifecycle events reach socket clients")
finally:
    request_path.unlink(missing_ok=True)
    for process in switches:
        if process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=2)
    client.terminate()
    try:
        client.wait(timeout=3)
    except subprocess.TimeoutExpired:
        client.kill()
        client.wait(timeout=3)
    connection.close()
