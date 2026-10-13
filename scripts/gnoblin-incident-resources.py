#!/usr/bin/env python3
r"""Record bounded DRM ownership observations for #170, without opening GPU devices.

Opt-in companion to the incident-20261011 resource-recorder.py; no service or
desktop integration. Example (private output directory, one sample by default):

  python3 scripts/gnoblin-incident-resources.py --output /private/path/gpu-incident
  python3 scripts/gnoblin-incident-resources.py --output /private/path/gpu-incident \
      --samples 360 --interval 10

Each JSONL row contains memKiB, gpuClients and coverage. A client has identity
(drm-driver, drm-pdev, drm-client-id), memoryBytes, engineNs and holders
(pid, startTicks, name, fdCount). Counters come from the first observed fd for
that client; duplicate fds add holders, not bytes. identityScope is "device"
when drm-pdev is present. Without it, identityScope is "fd": observedFd and the
holder identify a separate observation, never merged with another fd, and
coverage.errors.ambiguousDeviceFds reports the ambiguity. These rows may count
the same allocation again and must not be summed. startTicks is proc stat field
22 and distinguishes PID reuse within a boot. No arguments, executable paths,
environments, client-supplied names or file payloads are recorded.

Memory categories overlap; do not add total/resident/shared/purgeable together.
Different clients can share buffers, so their sum is not physical GPUActive and
holders are not proof of who allocated a buffer. Missing optional counters are
unknown, not zero. Reads are non-atomic; process identity is rechecked after its
fd scan. Visibility is limited to this collector's procfs mount and permissions;
even an error-free scan cannot establish whole-machine coverage. Exited clients,
kernel-only allocations and activity between samples remain unobservable.

Only character-device fds are fdinfo candidates, determined by stat on procfs
fd targets, never by path or major number. Unavailable targets fall back to
fdinfo and increment fdTargetUnavailable. Device files themselves are not opened.
Limits per sample: 4096 processes, 65536 fd entries, 8192 fdinfo files, 256 clients, 512 holder
entries, 16 KiB per read, 64 DRM counters per fd, 250 ms cooperative scan budget.
coverage.errors and limitsHit distinguish unavailable/truncated data from zero.
The deadline is checked between reads; a slow kernel read can exceed it. The
default 10-second interval is a wait AFTER scanning. At most 360 samples per run;
no caches, subprocesses or background work survive a sample/run. SIGINT/SIGTERM
interrupt the wait and close the writer; a signal during scanning is handled at
the next budget check. Four files of at most 4 MiB each bound retained output.
Output requires a caller-owned 0700 directory; files are 0600, with an exclusive
directory lock. Rotation occurs before appending. Remove that explicit output
directory after diagnosis to remove the records; this script installs nothing.

The existing recorder's PSS/CPU/PSI and native-session gate are not replaced.
Main must choose integration and validate runtime overhead separately.
Kernel field semantics: https://docs.kernel.org/gpu/drm-usage-stats.html
"""

import argparse
from collections import Counter
import fcntl
import json
import os
from pathlib import Path
import re
import signal
import stat
import threading
import time


def read_bounded(path, limit=16384):
    with open(path, "rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise OverflowError()
    return data.decode("utf-8", errors="replace")


def parse_fdinfo(text, errors):
    """Allowlist numeric usage fields; ignore client names and unknown payloads."""
    identity = {}
    memory = {}
    engines = {}
    aliases = {}
    counters = 0
    saw_drm = False
    invalid_identity = False
    for line in text.splitlines():
        key, sep, value = line.partition(":")
        if not sep:
            continue
        saw_drm |= key.startswith("drm-")
        value = value.strip()
        if key in ("drm-driver", "drm-pdev", "drm-client-id"):
            pattern = {
                "drm-driver": r"[a-zA-Z0-9_-]{1,64}",
                "drm-pdev": r"[0-9a-fA-F]{4}:[0-9a-fA-F]{2}:[0-9a-fA-F]{2}\.[0-7]",
                "drm-client-id": r"[0-9]{1,20}",
            }[key]
            if re.fullmatch(pattern, value):
                identity[key] = value
            else:
                errors["invalidDrmFields"] += 1
                invalid_identity = True
            continue
        is_memory = re.fullmatch(r"drm-(total|shared|resident|memory|purgeable|active)-[a-zA-Z0-9_-]{1,48}", key)
        is_engine = re.fullmatch(r"drm-engine-[a-zA-Z0-9_-]{1,48}", key)
        if not (is_memory or is_engine):
            continue
        # Capacity is a separate optional field, not engine time.
        if key.startswith("drm-engine-capacity-"):
            continue
        counters += 1
        if counters > 64:
            errors["counterLimit"] += 1
            break
        match = re.fullmatch(r"([0-9]{1,20})(?:\s+(KiB|MiB))?" if is_memory else r"([0-9]{1,20})\s+(ns)", value)
        if not match:
            errors["invalidDrmFields"] += 1
            continue
        number = int(match[1]) * {None: 1, "KiB": 1024, "MiB": 1048576, "ns": 1}[match[2]]
        if key.startswith("drm-memory-"):
            aliases[key.replace("drm-memory-", "drm-resident-", 1)] = number
        else:
            (memory if is_memory else engines)[key] = number
    for key, value in aliases.items():
        memory.setdefault(key, value)
    if invalid_identity:
        return None
    if "drm-driver" not in identity or "drm-client-id" not in identity:
        if saw_drm:
            errors["unidentifiedDrmFds"] += 1
        return None
    identity.setdefault("drm-pdev", None)
    return {"drm": identity, "memoryBytes": memory, "engineNs": engines}


def process_identity(text, pid):
    before, rest = text.rsplit(") ", 1)
    number, name = before.split(" (", 1)
    if int(number) != pid:
        raise ValueError()
    fields = rest.split()
    return {"pid": pid, "startTicks": int(fields[19]), "name": name[:64]}


def snapshot(
    proc_root=Path("/proc"),
    *,
    max_processes=4096,
    max_fd_entries=65536,
    max_fds=8192,
    max_clients=256,
    max_holders=512,
    budget_seconds=0.25,
    stop=None,
):
    """One bounded observation, with no state retained between samples."""
    root = Path(proc_root)
    started = time.monotonic()
    errors = Counter()
    limits = set()
    coverage = {
        "processes": 0,
        "fdEntries": 0,
        "nonDeviceFds": 0,
        "fdinfoFiles": 0,
        "traversalComplete": False,
        "errors": errors,
    }
    clients = {}
    holders = 0

    def failed(exc):
        key = (
            "permissionDenied"
            if isinstance(exc, PermissionError)
            else "vanished"
            if isinstance(exc, FileNotFoundError)
            else "oversizedFiles"
            if isinstance(exc, OverflowError)
            else "malformedFiles"
            if isinstance(exc, (ValueError, IndexError))
            else "readErrors"
        )
        errors[key] += 1

    def budget():
        if stop is not None and stop.is_set():
            limits.add("cancelled")
            return False
        if time.monotonic() - started >= budget_seconds:
            limits.add("time")
            return False
        return True

    mem = {}
    try:
        for line in read_bounded(root / "meminfo").splitlines():
            match = re.fullmatch(r"(MemAvailable|SwapFree|Shmem|GPUActive|GPUReclaim|AnonPages):\s+([0-9]+) kB", line)
            if match:
                mem[match[1]] = int(match[2])
    except (OSError, OverflowError) as exc:
        failed(exc)
    try:
        with os.scandir(root) as entries:
            for entry_count, entry in enumerate(entries):
                if not budget():
                    break
                if entry_count >= max_processes * 2:
                    limits.add("directoryEntries")
                    break
                if not entry.name.isdecimal():
                    continue
                if coverage["processes"] >= max_processes:
                    limits.add("processes")
                    break
                if coverage["fdinfoFiles"] >= max_fds:
                    limits.add("fds")
                    break
                if coverage["fdEntries"] >= max_fd_entries:
                    limits.add("fdEntries")
                    break
                coverage["processes"] += 1
                p = root / entry.name
                pending = {}
                try:
                    owner = process_identity(read_bounded(p / "stat"), int(entry.name))
                    with os.scandir(p / "fdinfo") as fds:
                        for fd in fds:
                            if not budget():
                                break
                            if coverage["fdinfoFiles"] >= max_fds:
                                limits.add("fds")
                                break
                            if coverage["fdEntries"] >= max_fd_entries:
                                limits.add("fdEntries")
                                break
                            coverage["fdEntries"] += 1
                            try:
                                mode = os.stat(f"{p}/fd/{fd.name}").st_mode
                            except OSError:
                                # A failed candidate check must not hide an
                                # otherwise readable DRM fdinfo record.
                                errors["fdTargetUnavailable"] += 1
                            else:
                                if not stat.S_ISCHR(mode):
                                    coverage["nonDeviceFds"] += 1
                                    continue
                            coverage["fdinfoFiles"] += 1
                            try:
                                client = parse_fdinfo(read_bounded(Path(fd.path)), errors)
                            except (OSError, OverflowError) as exc:
                                failed(exc)
                                continue
                            if client is None:
                                continue
                            key = tuple(client["drm"][k] for k in ("drm-driver", "drm-pdev", "drm-client-id"))
                            client["identityScope"] = "device"
                            if client["drm"]["drm-pdev"] is None:
                                # IDs can be device-scoped. Missing pdev does not
                                # prove global uniqueness for this driver/kernel.
                                # Avoid even same-process cross-device merges.
                                errors["ambiguousDeviceFds"] += 1
                                client["identityScope"] = "fd"
                                client["observedFd"] = int(fd.name)
                                key += (owner["pid"], owner["startTicks"], fd.name)
                            if key in pending:
                                pending[key]["holders"][0]["fdCount"] += 1
                                continue
                            if (
                                key not in clients
                                and len(clients) + sum(k not in clients for k in pending) >= max_clients
                            ):
                                limits.add("clients")
                                continue
                            if holders + len(pending) >= max_holders:
                                limits.add("holders")
                                continue
                            client["holders"] = [{**owner, "fdCount": 1}]
                            pending[key] = client
                    after = process_identity(read_bounded(p / "stat"), int(entry.name))
                    if owner != after:
                        errors["processChanged"] += 1
                        continue
                except (OSError, OverflowError, ValueError, IndexError) as exc:
                    failed(exc)
                    continue
                for key, client in pending.items():
                    if key in clients:
                        clients[key]["holders"].extend(client["holders"])
                    else:
                        clients[key] = client
                    holders += 1
            else:
                coverage["traversalComplete"] = not limits
    except OSError as exc:
        failed(exc)
    coverage.update(limitsHit=sorted(limits), durationMs=round((time.monotonic() - started) * 1000, 3))
    return {
        "schema": "gnoblin-gpu-incident-v1",
        "time": time.time(),
        "collectorUid": os.getuid(),
        "memKiB": mem,
        "gpuClients": list(clients.values()),
        "coverage": coverage,
    }


class RecordWriter:
    """Single writer, private bounded files, no open log fd across samples."""

    def __init__(self, directory, max_bytes=4 * 1024 * 1024, backups=3):
        self.directory = Path(directory)
        self.max_bytes = max_bytes
        self.backups = backups
        self.lock = None

    def __enter__(self):
        self.directory.mkdir(mode=0o700, parents=True, exist_ok=True)
        self.lock = os.open(self.directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            info = os.fstat(self.lock)
            if info.st_uid != os.getuid() or info.st_mode & 0o077:
                raise ValueError("output directory must be caller-owned and private (0700)")
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            return self
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_):
        if self.lock is not None:
            os.close(self.lock)
            self.lock = None

    def write(self, row):
        data = (json.dumps(row, separators=(",", ":")) + "\n").encode()
        if len(data) > self.max_bytes:
            raise ValueError("sample exceeds output file budget")
        names = ["gpu-resources.jsonl"] + [f"gpu-resources.{i}.jsonl" for i in range(1, self.backups + 1)]
        sizes = {}
        for name in names:
            try:
                info = os.stat(name, dir_fd=self.lock, follow_symlinks=False)
            except FileNotFoundError:
                continue
            if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid() or info.st_nlink != 1:
                raise ValueError("output must contain only caller-owned regular log files")
            if info.st_mode & 0o077 or info.st_size > self.max_bytes:
                raise ValueError("existing log exceeds privacy or size limits")
            sizes[name] = info.st_size
        if sizes.get(names[0], 0) + len(data) > self.max_bytes:
            for i in range(self.backups, 0, -1):
                if names[i - 1] in sizes:
                    os.rename(names[i - 1], names[i], src_dir_fd=self.lock, dst_dir_fd=self.lock)
        fd = os.open(
            names[0], os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600, dir_fd=self.lock
        )
        with os.fdopen(fd, "ab") as stream:
            stream.write(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--output", type=Path, required=True, help="private record directory")
    parser.add_argument(
        "--proc-root", type=Path, default=Path("/proc"), help="procfs root; override only for offline fixtures"
    )
    parser.add_argument("--samples", type=int, default=1, help="sample count, 1..360 (default: 1)")
    parser.add_argument(
        "--interval", type=float, default=10, help="wait seconds between samples, 10..3600 (default: 10)"
    )
    args = parser.parse_args()
    if not 1 <= args.samples <= 360 or not 10 <= args.interval <= 3600:
        parser.error("samples must be 1..360 and interval must be 10..3600")
    stop = threading.Event()
    old_handlers = {}
    try:
        for sig in (signal.SIGINT, signal.SIGTERM):
            old_handlers[sig] = signal.signal(sig, lambda *_: stop.set())
        with RecordWriter(args.output) as writer:
            for index in range(args.samples):
                if stop.is_set():
                    break
                writer.write(snapshot(args.proc_root, stop=stop))
                if index + 1 < args.samples and stop.wait(args.interval):
                    break
    finally:
        for sig, handler in old_handlers.items():
            signal.signal(sig, handler)


if __name__ == "__main__":
    main()
