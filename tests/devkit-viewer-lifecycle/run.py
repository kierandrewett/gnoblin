#!/usr/bin/env python3
"""Compile the MetaMdk spawn seam and exercise Linux ownership without a GUI."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
import ctypes
import json
import os
from pathlib import Path
import re
import select
import shlex
import signal
import struct
import subprocess
import tempfile
import time

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
PATCH = ROOT / "patches/mutter/10-tooling/0008-devkit-bind-viewer-to-compositor.patch"
SOURCE = "src/core/meta-mdk.c"
EVENT = struct.Struct("=5i")
ENTER, INSTALLED, EXECUTED, SPAWNED, FAILED, THREAD_DONE = range(1, 7)
NAME_ACQUIRED, ENABLED, SPAWNING, DIED = range(7, 11)
TIMEOUT = 3.0


def command(*args: str, cwd: Path = ROOT) -> str:
    return subprocess.check_output(args, cwd=cwd, text=True)


def compile_fixture(source: str, directory: Path, *, prctl: bool = True) -> Path:
    """Extract the actual callback and launcher registration, preserving guards.

    Baseline source intentionally has no callback. The instrumented test
    wrapper still provides the same events and race barriers in that build.
    """
    helper = re.search(
        r"#ifdef HAVE_SYS_PRCTL\nstatic void\nset_devkit_parent_death_signal"
        r"\b.*?\n}\n#endif",
        source,
        re.S,
    )
    if "set_devkit_parent_death_signal" in source and not helper:
        raise RuntimeError("callback extraction needs updating for the source layout")
    start = source.index("  launcher = g_subprocess_launcher_new (")
    end = source.index("  if (mdk->external_wayland_display)", start)
    excerpt = (helper[0] if helper else "") + (
        "\nstatic GSubprocessLauncher *\nlauncher_under_test (void)\n{\n"
        "  GSubprocessLauncher *launcher;\n" + source[start:end] + "  return launcher;\n}\n"
    )
    directory.mkdir()
    (directory / "spawn-under-test.h").write_text(excerpt)
    binary = directory / "fixture"
    flags = shlex.split(command("pkg-config", "--cflags", "--libs", "gio-2.0"))
    subprocess.run(
        [
            "cc",
            "-std=gnu11",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-pthread",
            *(["-DHAVE_SYS_PRCTL=1"] if prctl else []),
            "-I",
            str(directory),
            str(HERE / "fixture.c"),
            "-o",
            str(binary),
            *flags,
        ],
        check=True,
        env={**os.environ, "CCACHE_DISABLE": "1"},
    )
    return binary


def wait_child(pid: int) -> int:
    deadline = time.monotonic() + TIMEOUT
    while time.monotonic() < deadline:
        waited, status = os.waitpid(pid, os.WNOHANG)
        if waited:
            return os.waitstatus_to_exitcode(status)
        time.sleep(0.01)
    raise AssertionError(f"viewer {pid} survived owner death")


class Pair:
    def __init__(self, binary: Path, mode: str, bus: str = "unix:path=/nonexistent"):
        self.events: list[tuple[int, ...]] = []
        self.read_fd, write_fd = os.pipe()
        gate_read, self.gate_fd = os.pipe()
        try:
            self.owner = subprocess.Popen(
                [str(binary), mode],
                stdin=subprocess.PIPE,
                pass_fds=(write_fd, gate_read),
                start_new_session=True,
                env={
                    **os.environ,
                    "DBUS_SESSION_BUS_ADDRESS": bus,
                    "DISPLAY": "",
                    "WAYLAND_DISPLAY": "",
                    "FIXTURE_EVENT_FD": str(write_fd),
                    "FIXTURE_GATE_FD": str(gate_read),
                },
            )
        finally:
            os.close(write_fd)
            os.close(gate_read)

    def event(self, wanted: int) -> tuple[int, ...]:
        deadline = time.monotonic() + TIMEOUT
        # Events from the owner and viewer may arrive in either order.
        while True:
            for record in self.events:
                if record[0] == wanted:
                    return record
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not select.select([self.read_fd], [], [], remaining)[0]:
                raise AssertionError(f"timed out waiting for event {wanted}")
            data = os.read(self.read_fd, EVENT.size)
            if len(data) != EVENT.size:
                raise AssertionError(f"event pipe closed before event {wanted}")
            self.events.append(EVENT.unpack(data))

    def ready(self) -> int:
        spawned = self.event(SPAWNED)
        executed = self.event(EXECUTED)
        assert spawned[4] == executed[1], "spawned PID differs from executed child"
        assert executed[3] == self.owner.pid, "viewer is not a direct owner child"
        assert self.live(executed[1]), "viewer exited before owner teardown"
        return executed[1]

    @staticmethod
    def live(pid: int) -> bool:
        try:
            state = Path(f"/proc/{pid}/stat").read_text().rsplit(")", 1)[1].split()[0]
            return state not in ("Z", "X")
        except FileNotFoundError:
            return False

    def release(self) -> None:
        os.write(self.gate_fd, b"x")

    def stop(self, sig: int = 0) -> None:
        if sig:
            self.owner.send_signal(sig)
        else:
            self.owner.stdin.write(b"x")
            self.owner.stdin.flush()
        assert self.owner.wait(timeout=TIMEOUT) == -sig

    def close(self) -> None:
        # Only the process group created for this pair. Reap every adopted
        # descendant, including baseline or injected-failure survivors.
        try:
            os.killpg(self.owner.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        self.owner.wait(timeout=TIMEOUT)
        deadline = time.monotonic() + TIMEOUT
        while True:
            try:
                pid, _ = os.waitpid(-self.owner.pid, os.WNOHANG)
            except ChildProcessError:
                break
            if not pid:
                if time.monotonic() >= deadline:
                    raise RuntimeError("fixture descendants did not reap")
                time.sleep(0.01)
        self.owner.stdin.close()
        os.close(self.read_fd)
        os.close(self.gate_fd)

    def __enter__(self) -> Pair:
        return self

    def __exit__(self, *_: object) -> None:
        self.close()


@contextmanager
def private_bus(directory: Path):
    address = f"unix:path={directory}/bus"
    daemon = subprocess.Popen(
        ["dbus-daemon", "--session", "--nofork", "--nopidfile", f"--address={address}", "--print-address=1"],
        stdout=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        assert select.select([daemon.stdout], [], [], TIMEOUT)[0], "bus startup timeout"
        assert daemon.stdout.readline().strip(), "bus failed to start"
        yield address
    finally:
        daemon.terminate()
        daemon.wait(timeout=TIMEOUT)
        daemon.stdout.close()


def run_case(binary: Path, case: str, directory: Path) -> None:
    if case == "isolation":
        with Pair(binary, "normal") as a, Pair(binary, "normal") as b:
            viewer_a, viewer_b = a.ready(), b.ready()
            a.stop(signal.SIGKILL)
            assert wait_child(viewer_a) == -signal.SIGTERM
            assert b.live(viewer_b) and b.owner.poll() is None
            b.stop()
            assert wait_child(viewer_b) == -signal.SIGTERM
        return
    if case == "context":
        with private_bus(directory) as bus, Pair(binary, case, bus) as pair:
            viewer = pair.ready()
            callbacks = [e for e in pair.events if e[0] in (NAME_ACQUIRED, ENABLED, SPAWNING)]
            assert [e[0] for e in callbacks] == [NAME_ACQUIRED, ENABLED] * 2 + [SPAWNING]
            assert all(e[1] == e[2] == pair.owner.pid for e in callbacks)
            # The production callback is reached in the directly forked child.
            entered = pair.event(ENTER)
            assert entered[1] == entered[2] == viewer and entered[3] == pair.owner.pid
            print(f"  main PID/TID={pair.owner.pid}; name/enabled/spawn TIDs match; child={viewer}")
            pair.stop(signal.SIGKILL)
            assert wait_child(viewer) == -signal.SIGTERM
        return
    mode = "normal" if case in ("normal", "sigterm", "sigkill") else case
    with Pair(binary, mode) as pair:
        if case == "spawn-failure":
            assert pair.event(FAILED)[4] == 0
            assert not any(e[0] == SPAWNED for e in pair.events)
            pair.stop()
            return
        if case == "prctl-failure":
            assert pair.event(DIED)[4] == 127, "setup failure must exit before exec"
            assert not any(e[0] == EXECUTED for e in pair.events)
            pair.stop()
            return
        if case in ("before", "after"):
            viewer = pair.event(ENTER if case == "before" else INSTALLED)[1]
            pair.stop(signal.SIGKILL)
            if case == "before":
                pair.release()
            expected = 127 if case == "before" else -signal.SIGTERM
        else:
            viewer = pair.ready()
            if case == "thread":
                spawning = pair.event(SPAWNING)
                assert spawning[2] != pair.owner.pid, "fixture did not spawn from worker"
                pair.release()
                pair.event(THREAD_DONE)
                assert pair.event(DIED)[4] == signal.SIGTERM
                assert pair.owner.poll() is None, "owner process also exited"
                print(f"  owner={pair.owner.pid} alive; creator TID={spawning[2]} exited; viewer got SIGTERM")
                pair.stop()
                return
            sig = {"normal": 0, "sigterm": signal.SIGTERM, "sigkill": signal.SIGKILL}[case]
            pair.stop(sig)
            expected = -signal.SIGTERM
        assert wait_child(viewer) == expected


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", action="store_true", help="run unpatched source; expect failures")
    args = parser.parse_args()
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(36, 1, 0, 0, 0) != 0:  # PR_SET_CHILD_SUBREAPER
        raise OSError(ctypes.get_errno(), "cannot become fixture subreaper")
    revision = json.loads((ROOT / "src/gnome-versions.json").read_text())["components"]["mutter"]["commit"]
    baseline = command("git", "-C", str(ROOT / "subprojects/mutter"), "show", f"{revision}:{SOURCE}")
    # Never apply to or stage the shared submodule, even when its tree is dirty.
    with tempfile.TemporaryDirectory(prefix=".run-", dir=HERE) as scratch:
        directory = Path(scratch)
        subprocess.run(["git", "init", "--quiet", str(directory)], check=True)
        source_path = directory / SOURCE
        source_path.parent.mkdir(parents=True)
        source_path.write_text(baseline)
        previous = PATCH.with_name("0007-devkit-keep-session-after-viewer-exit.patch")
        subprocess.run(["git", "apply", str(previous)], cwd=directory, check=True)
        if not args.baseline:
            subprocess.run(["git", "apply", "--check", "--whitespace=error", str(PATCH)], cwd=directory, check=True)
            subprocess.run(["git", "apply", str(PATCH)], cwd=directory, check=True)
        source = source_path.read_text()
        binary = compile_fixture(source, directory / "linux")
        compile_fixture(source, directory / "no-prctl", prctl=False)
        print("PASS compile with and without HAVE_SYS_PRCTL", flush=True)
        failures = []
        for case in (
            "normal",
            "sigterm",
            "sigkill",
            "before",
            "after",
            "spawn-failure",
            "prctl-failure",
            "isolation",
            "thread",
            "context",
        ):
            try:
                run_case(binary, case, directory)
                print(f"PASS {case}", flush=True)
            except (AssertionError, subprocess.TimeoutExpired) as error:
                failures.append(case)
                print(f"FAIL {case}: {error}", flush=True)
        print(f"{'BASELINE' if args.baseline else 'PATCHED'}: {len(failures)} failed; all fixture groups reaped")
        return bool(failures)


if __name__ == "__main__":
    raise SystemExit(main())
