#!/usr/bin/env python3
"""Verify Mutter keeps configured Clutter effects alive on a mapped window."""

import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import time

assert os.environ.get("GNOBLIN_COMPOSITOR_SOCKET"), "Run inside a supervised Gnoblin session"
client = os.environ["GNOBLIN_INPUT_SOURCE_FOCUS_CLIENT"]
gnoblinctl = os.environ.get("GNOBLIN_DEVKIT_CTL") or shutil.which("gnoblinctl") or "gnoblinctl"
title = "Gnoblin effect ownership fixture"
runtime_log = Path(os.environ["GNOBLIN_DEVKIT_RUNTIME_LOG"])
client_log = Path(os.environ["XDG_RUNTIME_DIR"]) / "effect-ownership-client.log"


def listed_window():
    result = subprocess.run(
        [gnoblinctl, "--json", "window", "list", "--title", title],
        check=True,
        capture_output=True,
        text=True,
        timeout=3,
    )
    return json.loads(result.stdout)["windows"]


def assert_runtime_clean():
    output = runtime_log.read_text(errors="replace")
    matches = re.findall(
        r".*(?:Clutter-CRITICAL|SIGSEGV|Segmentation fault|invalid unclassed pointer).*",
        output,
    )
    assert not matches, "Mutter reported a Clutter effect lifetime failure:\n" + "\n".join(matches)


with client_log.open("w") as log:
    process = subprocess.Popen([client, "window", title], stdout=log, stderr=log)
    try:
        deadline = time.monotonic() + 10
        windows = []
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise AssertionError(f"Wayland fixture exited early with {process.returncode}")
            windows = listed_window()
            if windows:
                break
            time.sleep(0.1)
        assert windows, "effect ownership fixture did not appear in the window list"
        subprocess.run(
            [gnoblinctl, "config", "reload"],
            check=True,
            capture_output=True,
            text=True,
            timeout=5,
        )
        time.sleep(0.5)
        subprocess.run([gnoblinctl, "ping"], check=True, capture_output=True, text=True, timeout=3)
        assert listed_window(), "window disappeared while its effects were attached"
        assert_runtime_clean()
        process.send_signal(signal.SIGTERM)
        process.wait(timeout=5)
        subprocess.run([gnoblinctl, "ping"], check=True, capture_output=True, text=True, timeout=3)
        assert_runtime_clean()
        print("PASS: rounded clip, replacement shadow and shader survive map, repaint and close")
    finally:
        if process.poll() is None:
            process.terminate()
            process.wait(timeout=5)
