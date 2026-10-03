#!/usr/bin/env python3
"""Verify that untrusted Wayland activation cannot steal focus in Gnoblin."""

import json
import os
from pathlib import Path
import select
import socket
import subprocess
import time


EVENT = "gnoblin.window.activation-denied"
TITLE = "Untrusted Activation Target"
runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
socket_path = Path(
    os.environ.get(
        "GNOBLIN_COMPOSITOR_SOCKET",
        str(runtime_dir / "gnoblin" / "compositor-v1.sock"),
    )
)
ctl = os.environ["GNOBLIN_DEVKIT_CTL"]
client_binary = os.environ["GNOBLIN_FOCUS_TEST_CLIENT"]
request_path = runtime_dir / "request-untrusted-activation"
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


def window_snapshot():
    result = subprocess.run(
        [ctl, "--timeout", "1", "--json", "window", "list"],
        check=True,
        capture_output=True,
        text=True,
        timeout=3,
    )
    snapshot = json.loads(result.stdout)
    if isinstance(snapshot, list):
        return snapshot
    if isinstance(snapshot, dict) and isinstance(snapshot.get("windows"), list):
        return snapshot["windows"]
    raise AssertionError(f"unexpected window-list response: {snapshot!r}")


connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
connection.connect(str(socket_path))
stream = JsonLines(connection)
hello = stream.receive(3)
assert hello.get("event") == "hello", hello
api_minor = hello.get("api_minor", -1)
assert api_minor >= 69, f"activation denial events require API 1.69; got {api_minor}"
assert EVENT in hello.get("events", []), f"{EVENT} was not advertised"

stream.send(
    {
        "op": "events",
        "api_version": {"major": 1, "minor": api_minor},
        "events": [EVENT],
    }
)
subscribed = stream.receive(3)
assert subscribed.get("event") == "subscribed", subscribed
assert EVENT in subscribed.get("events", []), subscribed

client_env = os.environ.copy()
client_env.pop("DESKTOP_STARTUP_ID", None)
client_env.pop("XDG_ACTIVATION_TOKEN", None)
client = subprocess.Popen(
    [client_binary, str(request_path)],
    env=client_env,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.PIPE,
    text=True,
)
try:
    deadline = time.monotonic() + 12
    windows = []
    while time.monotonic() < deadline:
        if client.poll() is not None:
            stderr = client.stderr.read() if client.stderr else ""
            raise RuntimeError(f"activation test client exited early: {stderr}")
        windows = window_snapshot()
        if any(window.get("title") == TITLE for window in windows):
            break
        time.sleep(0.05)
    else:
        raise AssertionError(f"Wayland test window did not appear: {windows!r}")

    request_path.touch()
    event = stream.receive(12)
    assert event.get("event") == EVENT, event
    target = next(window for window in windows if window.get("title") == TITLE)
    assert event.get("window_id") == target.get("id"), (event, target)
    assert event.get("reason") in {
        "missing_context",
        "invalid_context",
        "stale_context",
    }, event

    deadline = time.monotonic() + 3
    while time.monotonic() < deadline:
        windows = window_snapshot()
        target = next(window for window in windows if window.get("title") == TITLE)
        if not target.get("focused"):
            break
        time.sleep(0.05)
    else:
        raise AssertionError("untrusted activation focused its target window")

    print("PASS: Gnoblin denied activation without user context and emitted the denial event")
finally:
    request_path.unlink(missing_ok=True)
    client.terminate()
    try:
        client.wait(timeout=3)
    except subprocess.TimeoutExpired:
        client.kill()
        client.wait(timeout=3)
    connection.close()
