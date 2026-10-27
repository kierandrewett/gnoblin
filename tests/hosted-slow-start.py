#!/usr/bin/env python3
"""A hosted session that starts slowly must still run the user's login autostart.

usage: scripts/run-staged.sh PREFIX STAGE python3 tests/hosted-slow-start.py [--evidence DIR]

On a loaded machine Mutter's first start took 20 s or more. The Lua worker used to wait 10 s
for the compositor's handshake, restart, lose the login autostart snapshot and never start the
shell, so the session manager gave up. This test starves the compositor the way a loaded host
does: it stops the compositor process with SIGSTOP right after it appears and resumes it later.

Cases, each a fresh `gnoblin --hosted` with a private session bus and a user configuration that
autostarts a marker command:
  * stall 25 s with the default wait: no worker restart, Ready becomes true, the marker ran,
    and no built-in-defaults notice was written;
  * stall 25 s with GNOBLIN_HOSTED_STARTUP_WAIT_MS=5000: the wait is honoured, so the old failure
    appears (the handshake times out and the Lua runtime is lost). This proves the first case passes because of
    the longer wait and not because the stall was too short;
  * an out-of-range wait value is ignored with a message.
"""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import shutil
import signal
import subprocess
import sys
import tempfile
import time

failures: list[str] = []


def check(name: str, ok: bool, detail: str = "") -> None:
    print(("PASS" if ok else "FAIL") + f": {name}" + (f" ({detail})" if detail and not ok else ""), flush=True)
    if not ok:
        failures.append(name)


def wait_for(predicate, timeout: float, step: float = 0.2):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(step)
    return predicate()


def compositor_pid(root_pid: int) -> int | None:
    """The descendant of the guardian started with --gnoblin-runtime-fd."""
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            command = (entry / "cmdline").read_bytes().split(b"\0")
            parent = int((entry / "stat").read_text().rsplit(")", 1)[1].split()[1])
        except (OSError, IndexError, ValueError):
            continue
        if parent == root_pid and any(arg.startswith(b"--gnoblin-runtime-fd") for arg in command):
            return int(entry.name)
    return None


def run_case(gnoblin: str, evidence: Path, name: str, stall: float, wait_ms: str | None, expect_restart: bool) -> None:
    tmp = Path(tempfile.mkdtemp(prefix="gnoblin-slow-start-"))
    run = tmp / "run"
    run.mkdir(mode=0o700)
    (tmp / "xdg-config-dirs").mkdir()
    config = tmp / "home/.config/gnoblin"
    config.mkdir(parents=True)
    marker = tmp / "autostart-ran"
    (config / "init.lua").write_text(
        'gnoblin.configure {\n    autostart = { marker = { command = { "touch", "' + str(marker) + '" } } },\n}\n'
    )
    env = {
        "PATH": os.environ["PATH"],
        "HOME": str(tmp / "home"),
        "XDG_RUNTIME_DIR": str(run),
        "XDG_CONFIG_HOME": str(tmp / "home/.config"),
        "XDG_CONFIG_DIRS": str(tmp / "xdg-config-dirs"),
        "XDG_DATA_HOME": str(tmp / "home/.local/share"),
        "XDG_CACHE_HOME": str(tmp / "home/.cache"),
        "XDG_STATE_HOME": str(tmp / "home/.local/state"),
        "XDG_SESSION_TYPE": "wayland",
        "DBUS_SYSTEM_BUS_ADDRESS": "unix:path=/nonexistent",
        "DBUS_SESSION_BUS_ADDRESS": f"unix:path={run}/bus",
        "LANG": "C.UTF-8",
    }
    if wait_ms:
        env["GNOBLIN_HOSTED_STARTUP_WAIT_MS"] = wait_ms
    log_path = evidence / f"slow-start-{name}.log"
    log = open(log_path, "w")
    bus = subprocess.Popen(
        ["dbus-daemon", "--session", "--nofork", f"--address=unix:path={run}/bus"], env=env, stdout=log, stderr=log
    )
    guardian = None
    try:
        wait_for(lambda: (run / "bus").exists(), 10)
        guardian = subprocess.Popen([gnoblin, "--hosted", "--wayland-display", "slow"], env=env, stdout=log, stderr=log)
        pid = wait_for(lambda: compositor_pid(guardian.pid), 30, 0.05)
        check(f"[{name}] the compositor process appeared", bool(pid))
        if not pid:
            return
        os.kill(pid, signal.SIGSTOP)
        stopped_at = time.monotonic()
        time.sleep(stall)
        os.kill(pid, signal.SIGCONT)

        def ready() -> bool:
            done = subprocess.run(
                [
                    "gdbus",
                    "call",
                    "--session",
                    "--dest",
                    "org.gnoblin.Hosted1",
                    "--object-path",
                    "/org/gnoblin/Hosted1",
                    "--method",
                    "org.freedesktop.DBus.Properties.Get",
                    "org.gnoblin.Hosted1",
                    "Ready",
                ],
                env=env,
                capture_output=True,
                text=True,
            )
            return "true" in done.stdout

        became_ready = wait_for(ready, 60, 0.5)
        note = f"stalled {time.monotonic() - stopped_at:.0f}s"
        if expect_restart:
            wait_for(lambda: "did not complete the private runtime handshake" in log_path.read_text(), 30)
        else:
            check(f"[{name}] Ready becomes true after the stall", bool(became_ready), note)
            check(f"[{name}] the login autostart ran", bool(wait_for(marker.exists, 30)), note)
        text = log_path.read_text()
        restarted = "restarting Lua runtime worker" in text or "restarting session supervisor" in text
        handshake = "did not complete the private runtime handshake" in text
        notice = "built-in defaults are being applied" in text or "fell back to the built-in defaults" in text
        if expect_restart:
            check(f"[{name}] a short wait still produces the old failure", handshake, text[-600:])
        else:
            check(f"[{name}] no handshake timeout, no worker restart", not restarted and not handshake, text[-600:])
            check(f"[{name}] no built-in defaults notice", not notice, text[-600:])
    finally:
        for process in (guardian, bus):
            if process and process.poll() is None:
                process.terminate()
        for process in (guardian, bus):
            if process:
                try:
                    process.wait(15)
                except subprocess.TimeoutExpired:
                    process.kill()
        log.close()
        shutil.rmtree(tmp, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--evidence", type=Path, default=Path(tempfile.mkdtemp(prefix="gnoblin-slow-evidence-")))
    parser.add_argument("--gnoblin", default=os.environ.get("GNOBLIN_BIN", "/usr/local/lib/gnoblin/bin/gnoblin"))
    args = parser.parse_args()
    args.evidence.mkdir(parents=True, exist_ok=True)
    run_case(args.gnoblin, args.evidence, "default-wait", 25, None, expect_restart=False)
    run_case(args.gnoblin, args.evidence, "short-wait", 25, "5000", expect_restart=True)
    bad = subprocess.run(
        [args.gnoblin, "--hosted", "--wayland-display", "x"],
        env={
            **os.environ,
            "GNOBLIN_HOSTED_STARTUP_WAIT_MS": "10",
            "XDG_RUNTIME_DIR": "/nonexistent",
            "WAYLAND_DISPLAY": "",
        },
        capture_output=True,
        text=True,
        timeout=30,
    )
    check(
        "an out-of-range wait is reported and ignored",
        "ignoring GNOBLIN_HOSTED_STARTUP_WAIT_MS=10" in bad.stderr,
        bad.stderr,
    )
    print(f"\n{len(failures)} failed" if failures else "\nall checks passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
