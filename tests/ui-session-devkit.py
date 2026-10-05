#!/usr/bin/env python3
"""Exercise the shell UI session relay through a fresh Gnoblin devkit socket."""

import json
import os
from pathlib import Path
import select
import socket
import time


runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
socket_path = Path(
    os.environ.get(
        "GNOBLIN_COMPOSITOR_SOCKET",
        str(runtime_dir / "gnoblin" / "compositor-v1.sock"),
    )
)


class Client:
    def __init__(self):
        self.connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.connection.connect(str(socket_path))
        self.buffer = bytearray()
        self.pending = []
        hello = self.receive(5)
        assert hello.get("event") == "hello", hello
        assert hello.get("api_minor", 0) >= 75, hello
        assert "ui-sessions" in hello.get("capabilities", []), hello

    def send(self, value):
        self.connection.sendall(json.dumps(value, separators=(",", ":")).encode() + b"\n")

    def receive(self, timeout=3):
        deadline = time.monotonic() + timeout
        while True:
            newline = self.buffer.find(b"\n")
            if newline >= 0:
                line = bytes(self.buffer[:newline])
                del self.buffer[: newline + 1]
                return json.loads(line)
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.connection], [], [], remaining)[0]:
                raise TimeoutError("timed out waiting for a compositor UI session record")
            chunk = self.connection.recv(65536)
            if not chunk:
                raise RuntimeError("Gnoblin closed the UI session connection")
            self.buffer.extend(chunk)

    def wait_for(self, predicate, timeout=3):
        deadline = time.monotonic() + timeout
        for index, record in enumerate(self.pending):
            if predicate(record):
                return self.pending.pop(index)
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("timed out waiting for expected UI session event")
            record = self.receive(remaining)
            if predicate(record):
                return record
            self.pending.append(record)

    def request(self, action, **values):
        request_id = f"ui-session-{action}-{time.monotonic_ns()}"
        self.send({"op": "ui-session", "action": action, "id": request_id, **values})
        return self.wait_for(lambda record: record.get("id") == request_id)

    def watch(self):
        reply = self.request("watch")
        assert reply.get("event") == "reply", reply
        assert reply.get("result", {}).get("accepted") is True, reply

    def expect_no(self, predicate, timeout=0.15):
        try:
            record = self.receive(timeout)
        except TimeoutError:
            return
        assert not predicate(record), f"unexpected UI session record: {record!r}"
        self.pending.append(record)

    def close(self):
        self.connection.close()


def event_is(event, name):
    return lambda record: record.get("event") == event and record.get("name") == name


def send_state(client, name, state):
    reply = client.request("state", name=name, state=state)
    assert reply.get("event") == "reply", reply


def main():
    owner = Client()
    watcher = Client()
    contender = Client()
    clients = [owner, watcher, contender]
    try:
        for client in clients:
            client.watch()
        print("UI_SESSION:capability-and-watch")

        name = "bingux.search"
        initial = {"open": False, "query": ""}
        send_state(owner, name, initial)
        for client in clients:
            state = client.wait_for(event_is("ui-state", name))
            assert state.get("state") == initial, state

        late_watcher = Client()
        clients.append(late_watcher)
        late_watcher.watch()
        snapshot = late_watcher.wait_for(event_is("ui-state", name))
        assert snapshot.get("state") == initial, snapshot
        # Registering watch a second time is idempotent and does not replay state.
        late_watcher.watch()
        late_watcher.expect_no(event_is("ui-state", name))

        updated = {"open": True, "query": "terminal"}
        send_state(owner, name, updated)
        for client in clients:
            state = client.wait_for(event_is("ui-state", name))
            assert state.get("state") == updated, state
        print("UI_SESSION:state-update-and-late-snapshot")

        rejected = contender.request("state", name=name, state={"open": False})
        assert rejected.get("event") == "error", rejected
        invalid_name = contender.request("state", name="not a session", state={})
        assert invalid_name.get("event") == "error", invalid_name
        invalid_payload = contender.request("command", name=name, command=[])
        assert invalid_payload.get("event") == "error", invalid_payload
        too_large = contender.request("state", name="large", state={"value": "x" * 9000})
        assert too_large.get("event") == "error", too_large
        for client in clients:
            client.expect_no(event_is("ui-state", name))
        print("UI_SESSION:ownership-and-payload-validation")

        command = {"action": "close"}
        reply = watcher.request("command", name=name, command=command)
        assert reply.get("event") == "reply", reply
        for client in clients:
            record = client.wait_for(event_is("ui-command", name))
            assert record.get("command") == command, record
        print("UI_SESSION:command-broadcast")

        owner.close()
        clients.remove(owner)
        for client in clients:
            cleared = client.wait_for(event_is("ui-state", name))
            assert cleared.get("state") is None, cleared
        replacement = Client()
        clients.append(replacement)
        replacement.watch()
        replacement.expect_no(event_is("ui-state", name))
        print("UI_SESSION:disconnect-clears-state")
    finally:
        for client in clients:
            client.close()

    print("PASS: native shell UI session relay works in a fresh Gnoblin devkit")


if __name__ == "__main__":
    main()
