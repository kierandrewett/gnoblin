#!/usr/bin/env python3
"""Verify native workspace animation events reach socket clients."""

import json
import hashlib
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time


EVENTS = ["gnoblin.animation.started", "gnoblin.animation.finished"]
ANIMATION = "devkit-workspace-switch-visual"
ANIMATION_DURATION_MS = 1200
SOURCE_TITLE = "Workspace Animation Source"
DESTINATION_TITLE = "Workspace Animation Destination"
SOURCE_ARGB = "0xffe01b24"
DESTINATION_ARGB = "0xff3584e4"
runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
socket_path = Path(os.environ["GNOBLIN_COMPOSITOR_SOCKET"])
ctl = os.environ["GNOBLIN_DEVKIT_CTL"]
client_binary = os.environ["GNOBLIN_FOCUS_TEST_CLIENT"]
request_path = runtime_dir / "request-workspace-animation"
request_path.unlink(missing_ok=True)
capture_root = Path(os.environ["GNOBLIN_TEST_ROOT"]) / "build/tmp/workspace-animation-captures"
capture_root.mkdir(parents=True, exist_ok=True)
capture_dir = Path(tempfile.mkdtemp(prefix="devkit-", dir=capture_root))


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


def window_records():
    result = run_ctl("--json", "window", "list")
    value = json.loads(result.stdout)
    windows = value if isinstance(value, list) else value.get("windows", [])
    if not isinstance(windows, list):
        raise AssertionError(f"unexpected window snapshot: {value!r}")
    return windows


def wait_for_windows(titles):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        windows = window_records()
        found = {title: next((window for window in windows if window.get("title") == title), None) for title in titles}
        if all(found.values()):
            return found
        for process in clients:
            if process.poll() is not None:
                stderr = process.stderr.read() if process.stderr else ""
                raise RuntimeError(f"workspace animation client exited early: {stderr}")
        time.sleep(0.05)
    raise AssertionError(f"Wayland test windows did not appear: {titles!r}")


def ppm_pixels(path):
    data = path.read_bytes()
    offset = 0

    def token():
        nonlocal offset
        while offset < len(data):
            if data[offset] in b" \t\r\n":
                offset += 1
            elif data[offset] == ord("#"):
                newline = data.find(b"\n", offset)
                if newline < 0:
                    raise AssertionError("unterminated PPM comment")
                offset = newline + 1
            else:
                break
        start = offset
        while offset < len(data) and data[offset] not in b" \t\r\n#":
            offset += 1
        if start == offset:
            raise AssertionError("incomplete PPM header")
        return data[start:offset]

    if token() != b"P6":
        raise AssertionError(f"unexpected screenshot format in {path}")
    width = int(token())
    height = int(token())
    if token() != b"255":
        raise AssertionError(f"unexpected PPM channel depth in {path}")
    if data[offset : offset + 2] == b"\r\n":
        offset += 2
    elif offset < len(data) and data[offset] in b" \t\r\n":
        offset += 1
    pixels = data[offset:]
    if len(pixels) != width * height * 3:
        raise AssertionError(f"invalid PPM pixel data in {path}")
    return width, height, pixels


def capture_frame(name):
    path = capture_dir / f"{name}.ppm"
    subprocess.run(["grim", "-t", "ppm", str(path)], check=True, timeout=5)
    width, height, pixels = ppm_pixels(path)
    red_pixels = 0
    blue_pixels = 0
    for offset in range(0, len(pixels), 3):
        red, green, blue = pixels[offset : offset + 3]
        if red > 180 and green < 100 and blue < 100:
            red_pixels += 1
        elif blue > 170 and green > 90 and red < 110:
            blue_pixels += 1
    return {
        "path": str(path),
        "width": width,
        "height": height,
        "red_pixels": red_pixels,
        "blue_pixels": blue_pixels,
        "sha256": hashlib.sha256(pixels).hexdigest(),
    }


def wait_for_visible_surface(name, color_name):
    deadline = time.monotonic() + 5
    last_frame = None
    while time.monotonic() < deadline:
        last_frame = capture_frame(name)
        if last_frame[color_name] > 5000:
            return last_frame
        time.sleep(0.1)
    raise AssertionError(f"{color_name} surface never appeared in screencopy: {last_frame!r}")


def install_test_animation():
    config_path = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin/init.lua"
    config_path.write_text(
        config_path.read_text()
        + f"""
gnoblin.animation {{
    name = "{ANIMATION}",
    event = "workspace-switch",
    duration = {ANIMATION_DURATION_MS},
    ease = "linear",
    from = {{progress = 0}},
    to = {{progress = 1}},
}}
"""
    )
    run_ctl("config", "reload")


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
install_test_animation()
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

clients = []
switches = []
try:
    for title, color, request in (
        (SOURCE_TITLE, SOURCE_ARGB, "workspace-animation-source-request"),
        (DESTINATION_TITLE, DESTINATION_ARGB, "workspace-animation-destination-request"),
    ):
        env = os.environ.copy()
        env["GNOBLIN_TEST_WINDOW_TITLE"] = title
        env["GNOBLIN_TEST_WINDOW_ARGB"] = color
        clients.append(
            subprocess.Popen(
                [client_binary, str(runtime_dir / request)],
                env=env,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.PIPE,
                text=True,
            )
        )
    windows = wait_for_windows((SOURCE_TITLE, DESTINATION_TITLE))

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

    run_ctl("window", "workspace", windows[DESTINATION_TITLE]["id"], "--id", destination_id)
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        windows = wait_for_windows((SOURCE_TITLE, DESTINATION_TITLE))
        if windows[DESTINATION_TITLE].get("workspace_id") == destination_id:
            break
        time.sleep(0.05)
    assert windows[SOURCE_TITLE].get("workspace_id") == source_id, windows[SOURCE_TITLE]
    assert windows[DESTINATION_TITLE].get("workspace_id") == destination_id, windows[DESTINATION_TITLE]

    before = wait_for_visible_surface("before", "red_pixels")
    assert before["blue_pixels"] < 500, f"destination surface is visible on the source workspace: {before!r}"

    visual_switch = subprocess.Popen(
        [ctl, "--timeout", "2", "workspace", "switch", "--id", destination_id],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    switches.append(visual_switch)
    visual_started = receive_animation(stream, EVENTS[0])
    assert visual_started.get("animation") == ANIMATION, visual_started
    assert visual_started.get("cancelled") is False, visual_started
    visual_frames = []
    for index in range(8):
        time.sleep(0.045)
        visual_frames.append(capture_frame(f"frame-{index + 1}"))
    visual_finished = receive_animation(stream, EVENTS[1])
    assert visual_finished.get("animation") == ANIMATION, visual_finished
    assert visual_finished.get("cancelled") is False, visual_finished
    visual_switch.wait(timeout=8)
    if visual_switch.returncode != 0:
        stderr = visual_switch.stderr.read() if visual_switch.stderr else ""
        raise RuntimeError(f"visual workspace switch failed: {stderr}")

    after = wait_for_visible_surface("after", "blue_pixels")
    assert after["red_pixels"] < 500, f"source surface remains visible after the switch: {after!r}"
    distinct_frames = {frame["sha256"] for frame in visual_frames}
    assert len(distinct_frames) >= 3, f"workspace animation produced too few distinct frames: {visual_frames!r}"
    assert any(frame["red_pixels"] + frame["blue_pixels"] > 5000 for frame in visual_frames), (
        f"workspace animation frames contain neither test surface: {visual_frames!r}"
    )
    (capture_dir / "summary.json").write_text(
        json.dumps(
            {"before": before, "frames": visual_frames, "after": after},
            indent=2,
        )
        + "\n"
    )

    return_switch = subprocess.Popen(
        [ctl, "--timeout", "2", "workspace", "switch", "--id", source_id],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    switches.append(return_switch)
    return_started = receive_animation(stream, EVENTS[0])
    assert return_started.get("animation") == ANIMATION, return_started
    return_finished = receive_animation(stream, EVENTS[1])
    assert return_finished.get("cancelled") is False, return_finished
    return_switch.wait(timeout=8)
    if return_switch.returncode != 0:
        stderr = return_switch.stderr.read() if return_switch.stderr else ""
        raise RuntimeError(f"return workspace switch failed: {stderr}")

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

    print(f"PASS: workspace animation rendered {len(distinct_frames)} distinct frames; captures: {capture_dir}")
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
    for client in clients:
        client.terminate()
        try:
            client.wait(timeout=3)
        except subprocess.TimeoutExpired:
            client.kill()
            client.wait(timeout=3)
    connection.close()
