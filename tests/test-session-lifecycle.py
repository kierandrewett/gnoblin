#!/usr/bin/env python3
"""Verify the guardian publishes session lifecycle over the native socket."""

from __future__ import annotations

import json
import os
from pathlib import Path
import signal
import socket
import time


class JsonLines:
    def __init__(self, connection: socket.socket) -> None:
        self.connection = connection
        self.buffer = b""

    def send(self, message: dict[str, object]) -> None:
        self.connection.sendall(json.dumps(message).encode() + b"\n")

    def receive(self, timeout: float = 5) -> dict[str, object]:
        self.connection.settimeout(timeout)
        while b"\n" not in self.buffer:
            chunk = self.connection.recv(4096)
            if not chunk:
                raise EOFError("compositor socket closed before the expected response")
            self.buffer += chunk
        line, self.buffer = self.buffer.split(b"\n", 1)
        return json.loads(line)


runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
socket_path = runtime_dir / "compositor-v1.sock"
connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
connection.connect(str(socket_path))
stream = JsonLines(connection)
hello = stream.receive()
assert hello.get("event") == "hello", hello
assert "gnoblin.session.state-changed" in hello.get("events", []), hello

stream.send(
    {
        "op": "events",
        "api_version": {"major": 1, "minor": 76},
        "events": ["gnoblin.session.state-changed"],
    }
)
subscribed = stream.receive()
assert subscribed.get("event") == "subscribed", subscribed

request_id = 0


def session_status() -> dict[str, object]:
    global request_id
    request_id += 1
    current_id = f"session-status-{request_id}"
    stream.send(
        {
            "op": "api",
            "api_version": {"major": 1, "minor": 76},
            "id": current_id,
            "method": "session.status",
            "arguments": {},
        }
    )
    while True:
        response = stream.receive()
        if response.get("event") == "reply" and response.get("id") == current_id:
            assert "result" in response, response
            return response["result"]


deadline = time.monotonic() + 10
status: dict[str, object] = {}
while time.monotonic() < deadline:
    status = session_status()
    if status.get("session_state") == "running":
        break
    time.sleep(0.05)
assert status.get("session_state") == "running", status
running_revision = status.get("session_revision")
assert isinstance(running_revision, int) and running_revision > 0, status

os.kill(int(os.environ["GNOBLIN_DEVKIT_HOST_PID"]), signal.SIGTERM)
deadline = time.monotonic() + 8
while time.monotonic() < deadline:
    event = stream.receive(max(0.1, deadline - time.monotonic()))
    if event.get("event") != "gnoblin.session.state-changed":
        continue
    if event.get("state") == "stopping":
        assert event.get("revision", 0) > running_revision, event
        print("SESSION_LIFECYCLE:stopping-socket-event")
        break
else:
    raise TimeoutError("guardian did not publish the stopping event before shutdown")

connection.close()
