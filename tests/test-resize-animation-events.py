#!/usr/bin/env python3
"""Verify native window resize animations through Lua and DevKit screencopy."""

import hashlib
import json
import os
from pathlib import Path
import select
import socket
import subprocess
import tempfile
import time


EVENTS = ["gnoblin.animation.started", "gnoblin.animation.finished"]
ANIMATION = "devkit-window-resize-visual"
ANIMATION_DURATION_MS = 1000
WINDOW_TITLE = "Resize Animation Surface"
WINDOW_ARGB = "0xffe01b24"
TARGET_WIDTH = 620
TARGET_HEIGHT = 360
runtime_dir = Path(os.environ["XDG_RUNTIME_DIR"])
socket_path = Path(os.environ["GNOBLIN_COMPOSITOR_SOCKET"])
ctl = os.environ["GNOBLIN_DEVKIT_CTL"]
client_binary = os.environ["GNOBLIN_FOCUS_TEST_CLIENT"]
request_path = runtime_dir / "request-resize-animation"
request_path.unlink(missing_ok=True)
capture_root = Path(os.environ["GNOBLIN_TEST_ROOT"]) / "build/tmp/resize-animation-captures"
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


def window_records():
    value = json.loads(run_ctl("--json", "window", "list").stdout)
    windows = value if isinstance(value, list) else value.get("windows", [])
    if not isinstance(windows, list):
        raise AssertionError(f"unexpected window snapshot: {value!r}")
    return windows


def wait_for_window(client):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        window = next(
            (window for window in window_records() if window.get("title") == WINDOW_TITLE),
            None,
        )
        if window:
            return window
        if client.poll() is not None:
            stderr = client.stderr.read() if client.stderr else ""
            raise RuntimeError(f"resize animation client exited early: {stderr}")
        time.sleep(0.05)
    raise AssertionError(f"Wayland test window did not appear: {WINDOW_TITLE!r}")


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
    red_pixels = sum(
        1
        for offset in range(0, len(pixels), 3)
        if pixels[offset] > 180 and pixels[offset + 1] < 100 and pixels[offset + 2] < 100
    )
    return {
        "path": str(path),
        "width": width,
        "height": height,
        "red_pixels": red_pixels,
        "sha256": hashlib.sha256(pixels).hexdigest(),
    }


def wait_for_visible_surface(name):
    deadline = time.monotonic() + 5
    last_frame = None
    while time.monotonic() < deadline:
        last_frame = capture_frame(name)
        if last_frame["red_pixels"] > 50_000:
            return last_frame
        time.sleep(0.1)
    raise AssertionError(f"resize test surface never appeared: {last_frame!r}")


def install_test_animation():
    config_path = Path(os.environ["XDG_CONFIG_HOME"]) / "gnoblin/init.lua"
    config_path.write_text(
        config_path.read_text()
        + f"""
gnoblin.animation {{
    name = "{ANIMATION}",
    event = "resize",
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
        if event.get("animation_event") != "resize":
            continue
        if event.get("event") != name:
            raise AssertionError(f"expected {name}, received {event!r}")
        return event
    raise TimeoutError(f"timed out waiting for {name}")


install_test_animation()
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

client = None
resize = None
try:
    env = os.environ.copy()
    env["GNOBLIN_TEST_WINDOW_TITLE"] = WINDOW_TITLE
    env["GNOBLIN_TEST_WINDOW_ARGB"] = WINDOW_ARGB
    client = subprocess.Popen(
        [client_binary, str(request_path)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
    )
    window = wait_for_window(client)
    before = wait_for_visible_surface("before")
    assert before["red_pixels"] > 50_000, before

    lua_path = runtime_dir / "resize-window.lua"
    lua_path.write_text(
        f"""
local window = gnoblin.windows.by_id({json.dumps(window["id"])})
assert(window and window.title == {json.dumps(WINDOW_TITLE)})
window:resize {{width = {TARGET_WIDTH}, height = {TARGET_HEIGHT}}}
"""
    )
    resize = subprocess.Popen(
        [ctl, "lua", str(lua_path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    started = receive_animation(stream, EVENTS[0])
    assert started.get("animation") == ANIMATION, started
    assert started.get("cancelled") is False, started

    visual_frames = []
    for index in range(12):
        time.sleep(0.055)
        visual_frames.append(capture_frame(f"frame-{index + 1}"))
    finished = receive_animation(stream, EVENTS[1])
    assert finished.get("animation") == ANIMATION, finished
    assert finished.get("cancelled") is False, finished

    stdout, stderr = resize.communicate(timeout=5)
    if resize.returncode != 0:
        raise RuntimeError(f"Lua resize failed: {stderr or stdout}")
    after = wait_for_visible_surface("after")
    frame_areas = [frame["red_pixels"] for frame in visual_frames]
    distinct_areas = {area for area in frame_areas}
    assert len(distinct_areas) >= 3, f"resize animation produced too few area steps: {visual_frames!r}"
    assert after["red_pixels"] > before["red_pixels"] * 1.5, (
        f"Lua resize did not visibly enlarge the client surface: before={before!r}, after={after!r}"
    )
    (capture_dir / "summary.json").write_text(
        json.dumps(
            {
                "before": before,
                "frames": visual_frames,
                "after": after,
                "started": started,
                "finished": finished,
            },
            indent=2,
        )
        + "\n"
    )
    print(f"PASS: resize animation rendered {len(distinct_areas)} visible size steps; captures: {capture_dir}")
finally:
    request_path.unlink(missing_ok=True)
    connection.close()
    if resize and resize.poll() is None:
        resize.terminate()
        try:
            resize.wait(timeout=2)
        except subprocess.TimeoutExpired:
            resize.kill()
            resize.wait(timeout=2)
    if client and client.poll() is None:
        client.terminate()
        try:
            client.wait(timeout=2)
        except subprocess.TimeoutExpired:
            client.kill()
            client.wait(timeout=2)
