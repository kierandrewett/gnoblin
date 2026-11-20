#!/usr/bin/env python3
"""Record what a live Gnoblin session answers on its control socket, next to the contract inventory.

The inventory in tests/parity/contracts was read out of the Mutter-based C sources and never started a compositor. This script
connects to a running Mutter-based session (the patched Mutter, API 1.93), asks every
operation of the inventory once and records the answer, subscribes to every event the session offers and records the
events that arrive, and writes tests/parity/contracts/recorded-session.json. A parity target that implements the same
socket can then be held to what the reference really answered, not to what the sources suggest it would.

Usage:
    python3 scripts/contract-record.py --socket PATH [--drive CMD] [--out FILE]   # record from a live session
    python3 scripts/contract-record.py --check                                    # verify the recorded file offline

`--drive CMD` runs CMD (through the shell) after the event subscription is made and before it is read, so a client can
map a window and the window events arrive. The environment of the session is the caller's business: this script never
starts a compositor and only speaks the line-delimited JSON of the socket (the Mutter-based tree's gnoblinctl client,
cli/gnoblinctl-client.c, is the client whose requests it repeats).

An operation is asked with an empty argument table. That is a deliberate choice: it records the answer to the plain
call, which is the answer of every read operation and the refusal of every write operation that needs arguments. The
operations that act on the machine even without arguments are not asked and are recorded as skipped with the reason.
"""

from __future__ import annotations

import argparse
import json
import os
import socket
import subprocess
import sys
import time
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONTRACTS = ROOT / "tests/parity/contracts"
OPERATIONS = CONTRACTS / "operations.json"
EVENTS = CONTRACTS / "events.json"
RECORDED = CONTRACTS / "recorded-session.json"

# Operations that do something to the machine or block for a long time even when asked with no arguments. They are
# recorded as skipped, never asked.
NOT_ASKED = {
    "session.logout": "ends the session",
    "session.poweroff": "powers the machine off",
    "session.reboot": "reboots the machine",
    "session.suspend": "suspends the machine",
    "session.hibernate": "hibernates the machine",
    "session.lock": "locks the session, and a headless session has no unlocker",
    "shortcut.capture": "waits for a key press for up to 30 seconds",
    "process.spawn": "starts a process",
    "process.kill": "signals a process",
    "config.restore_default": "replaces the user's configuration",
}

# How long one operation may take to answer, and how long events are collected after the last call.
ANSWER_SECONDS = 5.0
EVENT_TAIL_SECONDS = 2.0
# Samples keep the shape of the answer and not its size: lists are cut to this many elements, strings to this length.
SAMPLE_LIST = 3
# The most event names one subscription may name (control/gnoblin-control-requests.c of the Mutter-based tree).
SUBSCRIBE_MAX = 64
# Names in the hello event's list that are streams started by an operation, not names a subscription accepts.
SNAPSHOT_STREAMS = {"windows", "monitors"}
SAMPLE_TEXT = 200


class RecordError(RuntimeError):
    """A recording that cannot be made or a recorded file that does not hold together."""


def load(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8"))


def trim(value, depth: int = 0):
    """Cut a JSON value to a sample: lists to SAMPLE_LIST elements, strings to SAMPLE_TEXT, depth to 6."""
    if depth > 6:
        return "..."
    if isinstance(value, dict):
        return {key: trim(item, depth + 1) for key, item in value.items()}
    if isinstance(value, list):
        cut = [trim(item, depth + 1) for item in value[:SAMPLE_LIST]]
        return cut + [f"... {len(value) - SAMPLE_LIST} more"] if len(value) > SAMPLE_LIST else cut
    if isinstance(value, str) and len(value) > SAMPLE_TEXT:
        return value[:SAMPLE_TEXT] + "..."
    return value


class Connection:
    """One client of the control socket."""

    def __init__(self, path: str):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(ANSWER_SECONDS)
        try:
            self.sock.connect(path)
        except OSError as error:
            raise RecordError(f"cannot connect to {path}: {error}") from error
        self.buffer = b""
        # Everything that arrived and was not the answer to the call being waited on: events and late completions.
        self.unclaimed: list[dict] = []

    def send(self, message: dict) -> None:
        self.sock.sendall(json.dumps(message, separators=(",", ":")).encode() + b"\n")

    def line(self, timeout: float) -> dict | None:
        """The next JSON line, or None when none arrives within `timeout` seconds."""
        deadline = time.monotonic() + timeout
        while b"\n" not in self.buffer:
            left = deadline - time.monotonic()
            if left <= 0:
                return None
            self.sock.settimeout(left)
            try:
                chunk = self.sock.recv(65536)
            except (TimeoutError, socket.timeout):
                return None
            if not chunk:
                raise RecordError("the compositor closed the socket")
            self.buffer += chunk
        raw, self.buffer = self.buffer.split(b"\n", 1)
        try:
            return json.loads(raw)
        except json.JSONDecodeError as error:
            raise RecordError(f"the compositor sent a line that is not JSON: {error}: {raw[:200]!r}") from error

    def subscribe(self, names: list[str], minor: int) -> None:
        """Subscribe this connection to `names` and require the session to accept it."""
        self.send(
            {"op": "events", "id": str(uuid.uuid4()), "api_version": {"major": 1, "minor": minor}, "events": names}
        )
        deadline = time.monotonic() + ANSWER_SECONDS
        while (left := deadline - time.monotonic()) > 0:
            message = self.line(left)
            if message is None:
                break
            # The session answers a subscription with a `subscribed` event listing the names, and a refusal with an
            # `error` event that carries no id.
            if message.get("event") == "error":
                raise RecordError(f"the session refused the subscription: {message.get('message')}")
            if message.get("event") == "subscribed":
                if message.get("events") != names:
                    raise RecordError("the session subscribed to other names than were asked for")
                return
            self.unclaimed.append(message)
        raise RecordError("the session did not answer the subscription")

    def drain(self, seconds: float) -> None:
        """Collect what arrives for `seconds` into `unclaimed`."""
        deadline = time.monotonic() + seconds
        while (left := deadline - time.monotonic()) > 0:
            message = self.line(left)
            if message is not None:
                self.unclaimed.append(message)

    def call(self, method: str, minor: int | None, arguments: dict | None = None) -> dict:
        """Ask one operation and return what the session answered, with its completion when it has one."""
        request_id = str(uuid.uuid4())
        request = {"op": "api", "id": request_id, "method": method}
        if minor is not None:
            request["api_version"] = {"major": 1, "minor": minor}
        request["arguments"] = arguments or {}
        started = time.monotonic()
        self.send(request)
        deadline = started + ANSWER_SECONDS
        reply = None
        descriptor_id = None
        while time.monotonic() < deadline:
            message = self.line(deadline - time.monotonic())
            if message is None:
                break
            event = message.get("event", "")
            if message.get("id") == request_id and event in ("reply", "error"):
                reply = message
                result = message.get("result")
                if event == "reply" and isinstance(result, dict) and "request_id" in result and "method" in result:
                    descriptor_id = result["request_id"]
                    continue
                break
            if descriptor_id is not None and event in (
                "gnoblin.operation.completed",
                "gnoblin.api.operation-completed",
            ):
                key = "request_id" if event == "gnoblin.api.operation-completed" else "operation_id"
                if message.get(key) == descriptor_id:
                    return {
                        "status": "ok" if message.get("ok") else "error",
                        "reply": reply,
                        "completion": message,
                        "milliseconds": round((time.monotonic() - started) * 1000),
                    }
                continue
            self.unclaimed.append(message)
        milliseconds = round((time.monotonic() - started) * 1000)
        if reply is None:
            return {"status": "timeout", "milliseconds": milliseconds}
        status = "ok" if reply.get("event") == "reply" else "error"
        return {"status": status, "reply": reply, "milliseconds": milliseconds}


def describe(operation: dict, outcome: dict) -> dict:
    """The recorded entry of one operation: how it answered, with a sample of the answer."""
    entry = {"name": operation["name"], "status": outcome["status"], "milliseconds": outcome["milliseconds"]}
    reply = outcome.get("reply")
    if reply is not None:
        if reply.get("event") == "error":
            entry["error"] = trim({key: reply[key] for key in ("code", "message") if key in reply})
        else:
            entry["result"] = trim(reply.get("result"))
    completion = outcome.get("completion")
    if completion is not None:
        entry["completion"] = trim({key: value for key, value in completion.items() if key not in ("event",)})
    return entry


def record(args: argparse.Namespace) -> int:
    operations = load(OPERATIONS)
    api_minor = operations["api"]["minor"]
    connection = Connection(args.socket)
    hello = connection.line(ANSWER_SECONDS)
    if not hello or hello.get("event") != "hello":
        raise RecordError(f"the first line of the socket is not a hello event: {hello!r}")
    if (hello.get("api_major"), hello.get("api_minor")) != (operations["api"]["major"], api_minor):
        raise RecordError(
            f"the session speaks API {hello.get('api_major')}.{hello.get('api_minor')}, "
            f"the inventory is for {operations['api']['major']}.{api_minor}: record from the matching build"
        )

    # Subscribe before anything is asked, so every event the calls cause arrives. A subscription is capped (SUBSCRIBE_MAX
    # names) and a later one replaces an earlier one on the same connection, so each slice of the offered names has a
    # connection of its own; the first one is also the connection the operations are asked on.
    names = [name for name in hello["events"] if name not in SNAPSHOT_STREAMS]
    slices = [names[start : start + SUBSCRIBE_MAX] for start in range(0, len(names), SUBSCRIBE_MAX)]
    listeners = [connection]
    for _ in slices[1:]:
        extra = Connection(args.socket)
        if not (extra_hello := extra.line(ANSWER_SECONDS)) or extra_hello.get("event") != "hello":
            raise RecordError(f"the first line of an extra connection is not a hello event: {extra_hello!r}")
        listeners.append(extra)
    for listener, names_for in zip(listeners, slices):
        listener.subscribe(names_for, api_minor)
    # The snapshot streams the hello event lists with the events are not subscribed to by name: an operation of the
    # same name starts them, and the session then sends a snapshot and every change as lines of that name.
    for stream in SNAPSHOT_STREAMS & set(hello["events"]):
        connection.send({"op": stream, "id": str(uuid.uuid4()), "api_version": {"major": 1, "minor": api_minor}})
    for listener in listeners:
        listener.drain(0.5)
    if args.drive:
        print(f"driving: {args.drive}", flush=True)
        subprocess.run(args.drive, shell=True, check=False, timeout=120)
    for listener in listeners:
        listener.drain(EVENT_TAIL_SECONDS)

    recorded = []
    for operation in operations["entries"]:
        name = operation["name"]
        if operation["is_family"]:
            # A family row is a prefix. Ask a name inside it that does not exist to record how the family answers.
            outcome = connection.call(name + "parity-probe", api_minor if "VERSIONED" in operation["flags"] else None)
            entry = describe(operation, outcome)
            entry["family"] = True
        elif name in NOT_ASKED:
            entry = {"name": name, "status": "skipped", "reason": NOT_ASKED[name]}
        else:
            minor = api_minor if "VERSIONED" in operation["flags"] else None
            entry = describe(operation, connection.call(name, minor))
        recorded.append(entry)
        print(f"{entry['status']:>8}  {name}", flush=True)
    for listener in listeners:
        listener.drain(EVENT_TAIL_SECONDS)

    seen: dict[str, dict] = {}
    for message in [message for listener in listeners for message in listener.unclaimed]:
        event = message.get("event")
        if not event or event in ("reply", "error", "hello"):
            continue
        slot = seen.setdefault(event, {"count": 0, "sample": trim(message)})
        slot["count"] += 1
    inventory_events = {entry["name"] for entry in load(EVENTS)["entries"]}
    events = [
        {
            "name": name,
            "count": seen[name]["count"],
            "in_inventory": name in inventory_events,
            "sample": seen[name]["sample"],
        }
        for name in sorted(seen)
    ]

    document = {
        "format": 1,
        "kind": "recorded-session",
        "generated_by": "scripts/contract-record.py",
        "api": operations["api"] | {"source": "the session's own hello event"},
        "recorded_at": time.strftime("%Y-%m-%d", time.gmtime()),
        "hello": trim(hello)
        | {"methods": hello["methods"], "events": hello["events"], "capabilities": hello["capabilities"]},
        "drive": args.drive or None,
        "operations": recorded,
        "events_seen": events,
    }
    out = Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(document, indent=2, sort_keys=False) + "\n", encoding="utf-8")
    counts = {}
    for entry in recorded:
        counts[entry["status"]] = counts.get(entry["status"], 0) + 1
    print(f"wrote {out}: {len(recorded)} operations {counts}, {len(events)} event names seen")
    return 0


def check(path: Path) -> int:
    """Verify the recorded file against the inventory, offline: names, counts and the API version."""
    problems = []
    if not path.is_file():
        print(f"contract-record: {path.relative_to(ROOT)} does not exist", file=sys.stderr)
        return 1
    document = load(path)
    operations = load(OPERATIONS)
    inventory = {entry["name"]: entry for entry in operations["entries"]}
    if document.get("kind") != "recorded-session" or document.get("format") != 1:
        problems.append("not a format 1 recorded-session document")
    if (document["api"]["major"], document["api"]["minor"]) != (operations["api"]["major"], operations["api"]["minor"]):
        problems.append("the recorded API version is not the inventory's")
    names = [entry["name"] for entry in document["operations"]]
    if names != sorted(set(names), key=names.index):
        problems.append("an operation is recorded twice")
    for name in names:
        if name not in inventory:
            problems.append(f"recorded operation {name} is not in the inventory")
    for name in inventory:
        if name not in names:
            problems.append(f"inventory operation {name} was not recorded")
    for entry in document["operations"]:
        if entry["status"] not in ("ok", "error", "timeout", "skipped"):
            problems.append(f"{entry['name']}: unknown status {entry['status']}")
        if entry["status"] == "skipped" and entry["name"] not in NOT_ASKED:
            problems.append(f"{entry['name']}: skipped without being on the not-asked list")
    hello_methods = set(document["hello"]["methods"])
    for name in sorted(hello_methods - set(inventory)):
        problems.append(f"the hello event lists {name}, which the inventory does not")
    inventory_hello = {name for name, entry in inventory.items() if entry["hello"]}
    for name in sorted(inventory_hello - hello_methods):
        problems.append(f"the inventory says {name} is in the hello event, the session did not list it")
    inventory_events = {entry["name"] for entry in load(EVENTS)["entries"]}
    for name in sorted(set(document["hello"]["events"]) - inventory_events):
        problems.append(f"the hello event offers {name}, which the inventory does not list")
    for entry in document["events_seen"]:
        if entry["in_inventory"] != (entry["name"] in inventory_events):
            problems.append(f"event {entry['name']}: in_inventory does not match the inventory")
    for problem in problems:
        print(f"contract-record: {problem}", file=sys.stderr)
    return 1 if problems else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--socket",
        default=os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"),
        help="the control socket of the live session (default: $GNOBLIN_COMPOSITOR_SOCKET)",
    )
    parser.add_argument("--drive", help="a shell command to run after subscribing, to cause events")
    parser.add_argument("--out", default=str(RECORDED), help="where to write the recording")
    parser.add_argument("--check", action="store_true", help="verify the recorded file against the inventory, offline")
    args = parser.parse_args()
    try:
        if args.check:
            return check(Path(args.out))
        if not args.socket:
            parser.error("--socket or GNOBLIN_COMPOSITOR_SOCKET is required")
        return record(args)
    except RecordError as error:
        print(f"contract-record: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
