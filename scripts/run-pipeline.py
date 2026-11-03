#!/usr/bin/env python3
"""Run the release pipeline on this machine for every target that scripts/supported-targets.py lists.

usage: scripts/run-pipeline.py [--root DIR] [--only SLUGS] [--jobs N] [--vms N] [--force | --rerun STAGES] ...
       scripts/run-pipeline.py --matrix-only [--root DIR]
       scripts/run-pipeline.py --clean [--root DIR]

It does locally what .github/workflows/packages.yml and graphical.yml do in CI, one target at a time and several
targets at once:

  prefix     scripts/build-prefix.sh in a podman container of the prefix image of the target, after the bootstrap
             command and the build requirements script of the target (a target with its own prefix only)
  packages   scripts/build-packages.sh with the package arguments of the target, or for a target without a package
             lane of its own, the shared packages: --lanes other on every shared prefix of the architecture
  suite      scripts/run-package-tests.sh: the static, install, runtime and repository layers
  graphical  tests/graphical/harness.py on the cloud image of the release, for the graphical targets with packages

A stage runs only when the stage before it passed. A stage that does not run writes no result file, and the matrix
says "not run: <stage> failed". The driver names no release: the targets, images and arguments come from the
generated list. A target on another architecture than this machine is listed as not run.

Every stage writes the file that the release gate reads (design/release-pipeline.md):

  ROOT/<slug>-<arch>/prefix/stage.json         prefix build, also for a failure
  ROOT/<slug>-<arch>/packages/stage.json       package writing, also for a failure
  ROOT/<slug>-<arch>/package-tests/report.json the package suite (run-package-tests.sh --report)
  ROOT/<slug>-<arch>/graphical/result.json     the graphical VM test (harness.py --out)
  ROOT/artifacts/<artifact>/                   the packages, with the artifact names of CI
  ROOT/targets.json                            the generated target list

and its own record and log: ROOT/<slug>-<arch>/logs/<stage>.json and <stage>.log. The run can be stopped and
started again: a stage whose record exists is skipped, unless --force or --rerun names it, or a stage before it ran
again in this invocation. At the end it runs the release gate (tests/packages/gate.py) and the issue report in dry
run (tests/pipeline/issues.py --dry-run), and writes the matrix to ROOT/MATRIX.md.

The limits of the machine: --jobs container stages (prefix builds and package suites) and --vms guests at once, and
no heavy stage starts while the free space under ROOT is below --min-free-gb plus the room the stage needs.
GNOBLIN_PKGSMITH_REF (or --pkgsmith-ref) replaces the pinned pkgsmith for a trial run (scripts/install-pkgsmith.sh).
"""

import argparse
import datetime
import heapq
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import signal
import subprocess
import sys
import threading
import time

REPO = Path(__file__).resolve().parent.parent
STAGES = ("prefix", "packages", "suite", "graphical")
LAYERS = ("static", "install", "runtime", "repository")
PASS, FAIL, NOT_RUN = "pass", "fail", "not run"
HOST_ARCH = {"x86_64": "amd64", "aarch64": "arm64"}.get(platform.machine(), platform.machine())
TIMEOUTS = {"prefix": 150 * 60, "packages": 30 * 60, "suite": 90 * 60, "graphical": 90 * 60, "shared": 60 * 60}
# Room in GiB that a stage needs under ROOT on top of the floor, measured on the first runs: a build tree and its
# prefix (under 1 GiB), a package suite (a few hundred MiB), a cloud image and its guest disk (1 to 2 GiB).
ROOM = {"prefix": 2, "suite": 1, "graphical": 2}
PREFIX_SCRIPT = """
eval "$BOOTSTRAP"
git config --global --add safe.directory '*'
"./scripts/$DEPS_SCRIPT"
./scripts/build-prefix.sh --prefix /out/prefix
tar --zstd -cf /out/prefix.tar.zst -C /out/prefix .
"""
FAILURE_LINE = re.compile(
    r"(error[:\]]|ERROR|FAILED|Problem: |E: |fatal:|No match for argument|not found|Traceback|exited with)", re.I
)


def now():
    return datetime.datetime.now(datetime.UTC).isoformat(timespec="seconds")


def read_json(path):
    try:
        return json.loads(Path(path).read_text())
    except (OSError, ValueError):
        return None


def write_json(path, data):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(data, indent=1) + "\n")
    temporary.replace(path)


def remove(path):
    path = Path(path)
    if path.is_symlink() or path.is_file():
        path.unlink()
    elif path.is_dir():
        try:
            shutil.rmtree(path)
        except PermissionError:
            # Files that a container wrote with another user of its namespace.
            subprocess.run(["podman", "unshare", "rm", "-rf", "--", str(path)], check=False)


def free_gb(path):
    usage = shutil.disk_usage(path)
    return usage.free / 2**30


def memory_gb():
    values = {}
    for line in Path("/proc/meminfo").read_text().splitlines():
        name, value = line.split(":", 1)
        values[name] = int(value.split()[0]) / 2**20
    return values.get("MemAvailable", 0.0), values.get("SwapTotal", 0.0) - values.get("SwapFree", 0.0)


def failure_line(log, limit=220):
    """The last line of a log that names a failure, for a one-line detail."""
    try:
        lines = Path(log).read_text(errors="replace").splitlines()
    except OSError:
        return ""
    for line in reversed(lines[-400:]):
        if FAILURE_LINE.search(line) and "warning" not in line.lower():
            return line.strip()[:limit]
    return (lines[-1].strip() if lines else "")[:limit]


def minutes(seconds):
    if seconds is None:
        return ""
    seconds = int(seconds)
    return f"{seconds // 60}m{seconds % 60:02d}s" if seconds >= 60 else f"{seconds}s"


class PrioritySlots:
    """A counting semaphore that wakes the waiter with the lowest priority number first, so that the stages of an
    earlier target win over the prefix builds of later ones."""

    def __init__(self, count):
        self.free = count
        self.waiting = []
        self.condition = threading.Condition()
        self.order = 0

    def acquire(self, priority):
        with self.condition:
            self.order += 1
            ticket = (priority, self.order)
            heapq.heappush(self.waiting, ticket)
            while self.free == 0 or self.waiting[0] != ticket:
                self.condition.wait()
            heapq.heappop(self.waiting)
            self.free -= 1
            self.condition.notify_all()

    def release(self):
        with self.condition:
            self.free += 1
            self.condition.notify_all()


class Target:
    def __init__(self, entry, data, tag):
        self.entry = entry
        self.slug, self.arch = entry["slug"], entry["arch"]
        self.key = f"{self.slug}-{self.arch}"
        self.tag = tag

        def find(name):
            return next((e for e in data.get(name, []) if e["slug"] == self.slug and e["arch"] == self.arch), None)

        self.prefix = find("prefixes")
        self.lane = find("packages")
        self.graphical = next(
            (g for g in data.get("graphical", []) if g["slug"] == self.slug and g.get("arch", HOST_ARCH) == self.arch),
            None,
        )
        self.row = next((r for r in data.get("rows", []) if r["slug"] == self.slug), None)

    @property
    def artifact(self):
        """The folder of packages that this target installs, named as packages.yml names the artifact."""
        return f"{self.entry['install_artifact']}-{self.tag}{self.entry['install_suffix']}-{self.arch}"

    @property
    def prefix_name(self):
        return f"{self.prefix['prefix_artifact']}-{self.tag}-{self.slug}-{self.arch}" if self.prefix else ""

    def eol(self, today):
        if self.graphical is not None and "eol" in self.graphical:
            return bool(self.graphical["eol"]), self.graphical.get("eol_date") or ""
        date = (self.row or {}).get("eol") or ""
        return (bool(re.fullmatch(r"\d{4}-\d{2}-\d{2}", date)) and date < today), date


class Record:
    """What a stage did: the driver's record in ROOT/<target>/logs/<stage>.json."""

    def __init__(self, stage, status, detail="", seconds=0.0, evidence="", ran=False):
        self.stage, self.status, self.detail, self.seconds, self.evidence, self.ran = (
            stage,
            status,
            detail,
            seconds,
            evidence,
            ran,
        )
        self.queued = 0.0

    def data(self):
        return {
            "stage": self.stage,
            "status": self.status,
            "detail": self.detail,
            "seconds": round(self.seconds, 1),
            "queued_seconds": round(self.queued, 1),
            "evidence": self.evidence,
            "finished": now(),
        }


class Pipeline:
    def __init__(self, args, data):
        self.args = args
        self.data = data
        self.root = Path(args.root).resolve()
        # Sources, build trees and guest disks: outside ROOT, because the gate reads every result file under ROOT and
        # a source copy holds test fixtures with those names.
        self.scratch = Path(args.work).resolve()
        self.tag = args.release_tag
        self.today = datetime.date.today().isoformat()
        self.targets = [Target(entry, data, self.tag) for entry in data["matrix"]]
        self.build_slots = PrioritySlots(args.jobs)
        self.vm_slots = PrioritySlots(args.vms)
        self.lock = threading.Lock()
        self.setup_lock = threading.Lock()
        self.local = threading.local()
        self.prefix_done = {t.key: threading.Event() for t in self.targets}
        self.prefix_records = {}
        self.shared_done = {}
        self.shared_records = {}
        self.source_ready = False
        self.pkgsmith = None
        self.stopping = threading.Event()
        self.processes = set()
        self.env = dict(os.environ, TMPDIR=os.environ.get("TMPDIR", "/var/tmp"))
        if args.pkgsmith_ref:
            self.env["GNOBLIN_PKGSMITH_REF"] = args.pkgsmith_ref

    # -- paths

    def dir(self, key):
        return self.root / key

    def work(self, key):
        return self.scratch / key

    def record_path(self, key, stage):
        return self.dir(key) / "logs" / f"{stage}.json"

    def log_path(self, key, stage):
        path = self.dir(key) / "logs" / f"{stage}.log"
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def outputs(self, target, stage):
        """The files that a stage writes, removed before it runs again or when it does not run."""
        key = target.key if isinstance(target, Target) else target
        paths = {
            "prefix": [self.dir(key) / "prefix"],
            "packages": [self.dir(key) / "packages"],
            "suite": [self.dir(key) / "package-tests"],
            "graphical": [self.dir(key) / "graphical"],
        }[stage]
        return paths + [self.record_path(key, stage)]

    # -- processes

    def log(self, message):
        line = f"[pipeline {time.strftime('%H:%M:%S')}] {message}"
        with self.lock:
            print(line, flush=True)
            with open(self.root / "pipeline.log", "a") as handle:
                handle.write(line + "\n")

    def run(self, command, log, timeout, env=None, stdin=None, cwd=None, container=None):
        """Run a command with its output in log. Returns the exit status, 124 for a timeout, 130 once stopping."""
        if self.stopping.is_set():
            return 130
        with open(log, "a") as handle:
            handle.write(f"$ {shlex.join(str(part) for part in command)}\n")
            handle.flush()
            process = subprocess.Popen(
                [str(part) for part in command],
                stdout=handle,
                stderr=subprocess.STDOUT,
                stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
                env=env or self.env,
                cwd=cwd or REPO,
                start_new_session=True,
                text=True,
            )
            with self.lock:
                self.processes.add(process)
            try:
                process.communicate(stdin, timeout=timeout)
                status = process.returncode
            except subprocess.TimeoutExpired:
                handle.write(f"\n[pipeline] timed out after {timeout} s\n")
                self.kill(process, container)
                status = 124
            finally:
                with self.lock:
                    self.processes.discard(process)
            handle.write(f"[pipeline] exit status {status}\n")
        return status

    def kill(self, process, container=None):
        if container:
            subprocess.run(["podman", "rm", "-f", "-t", "5", container], capture_output=True, check=False)
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(30)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass

    def wait_for_room(self, stage, key):
        """Hold a heavy stage until the disk and the memory have room for it."""
        need = self.args.min_free_gb + ROOM.get(stage, 0)
        said = False
        while not self.stopping.is_set():
            disk = free_gb(self.root)
            memory, _ = memory_gb()
            if disk >= need and memory >= self.args.min_memory_gb:
                return
            if not said:
                self.log(
                    f"{key} {stage}: waiting for room ({disk:.1f} GiB free of {need} needed, {memory:.1f} GiB RAM)"
                )
                said = True
            time.sleep(30)

    # -- the source of the prefix builds

    def prepare_source(self):
        """A private copy of this checkout, with its uncommitted changes and standalone submodules, that each prefix
        build copies (a reflink on btrfs). ./build.sh resets and patches the submodules, so no two builds and not the
        checkout itself may share them."""
        with self.setup_lock:
            if self.source_ready:
                return
            golden = self.scratch / "source"
            log = self.root / "source.log"
            golden.parent.mkdir(parents=True, exist_ok=True)

            def git(*command, cwd=REPO):
                return subprocess.run(
                    ["git", *command], cwd=cwd, check=True, capture_output=True, text=True
                ).stdout.strip()

            common = git("rev-parse", "--path-format=absolute", "--git-common-dir")
            head = git("rev-parse", "HEAD")
            with open(log, "a") as handle:
                if not (golden / ".git").is_dir():
                    remove(golden)
                    subprocess.run(
                        ["git", "clone", "-q", "--shared", "--no-checkout", common, str(golden)],
                        check=True,
                        stdout=handle,
                        stderr=subprocess.STDOUT,
                    )
                subprocess.run(["git", "-C", str(golden), "checkout", "-q", "-f", "--detach", head], check=True)
                # The build writes the remote into version.json. It must be the remote of this checkout, as in CI, and
                # not the folder that the copy shares its objects with (a path of this machine, package suite S7).
                remote = git("remote", "get-url", "origin")
                subprocess.run(["git", "-C", str(golden), "remote", "set-url", "origin", remote], check=True)
                excludes = ["/.git", "/build", "/prefix", "/dist", "/install", "__pycache__", ".ruff_cache"]
                submodules = [
                    line.split(" ", 1)[1]
                    for line in git("config", "-f", ".gitmodules", "--get-regexp", r"submodule\..*\.path").splitlines()
                ]
                # The submodules come from their own Git folders, at the commits that the superproject records, as a
                # CI checkout has them. The files in this checkout may be a patched build state of an earlier build.
                excludes += [f"/{path}/" for path in submodules]
                subprocess.run(
                    ["rsync", "-a", "--delete", *[f"--exclude={item}" for item in excludes], f"{REPO}/", f"{golden}/"],
                    check=True,
                    stdout=handle,
                    stderr=subprocess.STDOUT,
                )
                for path in submodules:
                    if not (REPO / path / ".git").exists():
                        continue
                    gitdir = git("rev-parse", "--absolute-git-dir", cwd=REPO / path)
                    recorded = git("rev-parse", f"HEAD:{path}")
                    remove(golden / path)
                    (golden / path).mkdir(parents=True)
                    subprocess.run(["cp", "-a", "--reflink=auto", gitdir, str(golden / path / ".git")], check=True)
                    module = ["git", "-C", str(golden / path)]
                    subprocess.run([*module, "config", "--unset", "core.worktree"], check=False)
                    subprocess.run([*module, "checkout", "-q", "-f", "--detach", recorded], check=True)
                    subprocess.run([*module, "clean", "-q", "-f", "-d", "-x"], check=True)
                    # The Meson wraps that this checkout has fetched already (gvdb, libgxdp). The source step takes a
                    # wrap checkout at its pinned revision as it is and fetches only a missing or moved one, so this
                    # copy needs no network. GNOME GitLab answers some networks with HTTP 406.
                    for wrap in sorted((REPO / path / "subprojects").glob("*/.git")):
                        copy = golden / path / "subprojects" / wrap.parent.name
                        subprocess.run(["cp", "-a", "--reflink=auto", str(wrap.parent), str(copy)], check=True)
                        # Without the patches that an earlier build of this checkout applied to it.
                        subprocess.run(["git", "-C", str(copy), "reset", "-q", "--hard", "HEAD"], check=True)
                        subprocess.run(["git", "-C", str(copy), "clean", "-q", "-f", "-d", "-x"], check=True)
                        handle.write(f"{now()} {path}/subprojects/{wrap.parent.name} copied from {REPO}\n")
                    handle.write(f"{now()} {path} at {recorded}\n")
                # The pinned gsettings-desktop-schemas checkout that the source step keeps in build/source-inputs. It
                # is checked against its pinned revision and fetched again only when it differs.
                inputs = REPO / "build" / "source-inputs"
                remove(golden / "build")
                if inputs.is_dir():
                    (golden / "build").mkdir()
                    subprocess.run(["cp", "-a", "--reflink=auto", str(inputs), str(golden / "build")], check=True)
                    handle.write(f"{now()} build/source-inputs copied from {REPO}\n")
                handle.write(f"{now()} source {head} from {REPO}, objects shared with {common}\n")
            self.common_dir = common
            self.source_date = git("log", "-1", "--format=%ct")
            self.source_ready = True

    def pkgsmith_bin(self):
        with self.setup_lock:
            if self.pkgsmith is None:
                result = subprocess.run(
                    [str(REPO / "scripts/install-pkgsmith.sh")], env=self.env, capture_output=True, text=True
                )
                if result.returncode != 0:
                    raise RuntimeError(f"install-pkgsmith.sh failed: {result.stderr.strip()[-300:]}")
                self.pkgsmith = result.stdout.strip().splitlines()[-1]
            return self.pkgsmith

    def package_env(self):
        env = dict(self.env)
        env["PATH"] = f"{self.pkgsmith_bin()}:{env['PATH']}"
        env.setdefault("SOURCE_DATE_EPOCH", self.source_date_epoch())
        return env

    def source_date_epoch(self):
        return subprocess.run(
            ["git", "log", "-1", "--format=%ct"], cwd=REPO, capture_output=True, text=True, check=True
        ).stdout.strip()

    # -- the stage machinery

    def stage(self, key, stage, function, upstream_ran, *function_args):
        """Run one stage, or take its earlier record when it has one and nothing before it ran again."""
        previous = read_json(self.record_path(key, stage))
        forced = self.args.force or stage in self.args.rerun
        if previous and previous.get("status") in (PASS, FAIL) and not forced and not upstream_ran:
            self.log(f"{key} {stage}: kept the earlier result ({previous['status']})")
            return Record(stage, previous["status"], previous.get("detail", ""), previous.get("seconds", 0.0),
                          previous.get("evidence", ""))  # fmt: skip
        if self.stopping.is_set():
            return Record(stage, NOT_RUN, "the run was stopped")
        # The results of this stage and of the stages after it belong to the earlier run, and go before it starts, so
        # a stopped run never leaves a later result without the record of the stage it came from.
        for later in STAGES[STAGES.index(stage) :]:
            for path in self.outputs(key, later):
                remove(path)
        self.log(f"{key} {stage}: queued")
        started = time.monotonic()
        self.local.queued = 0.0
        try:
            status, detail, evidence = function(*function_args)
        except Exception as exception:  # noqa: BLE001 (every stage must end with a record)
            status, detail, evidence = FAIL, f"driver error: {type(exception).__name__}: {exception}", ""
        if self.stopping.is_set():
            # A stage that the stop killed has no result: the next run does it again.
            self.log(f"{key} {stage}: stopped")
            return Record(stage, NOT_RUN, "the run was stopped")
        queued = self.local.queued
        record = Record(stage, status, detail, time.monotonic() - started - queued, evidence, ran=True)
        record.queued = queued
        write_json(self.record_path(key, stage), record.data())
        self.log(f"{key} {stage}: {status} in {minutes(record.seconds)}{': ' + detail if detail else ''}")
        return record

    def not_run(self, target, stage, reason):
        """A stage that does not run leaves no result file behind, so no older result can pass for it."""
        for path in self.outputs(target, stage):
            remove(path)
        return Record(stage, NOT_RUN, reason)

    def contract(self, target, stage, record):
        """The stage.json of a build stage, which the release gate reads (design/release-pipeline.md)."""
        if record.status == NOT_RUN:
            return
        log = self.dir(target.key) / "logs" / f"{stage}.log"
        write_json(
            self.dir(target.key) / stage / "stage.json",
            {
                "stage": stage,
                "slug": target.slug,
                "arch": target.arch,
                "status": record.status,
                "detail": record.detail,
                "log": str(record.evidence or log),
            },
        )

    def run_target(self, target, priority, full):
        records = {}
        upstream = False
        try:
            if target.prefix:
                record = self.stage(target.key, "prefix", self.build_prefix, False, target, priority)
                self.contract(target, "prefix", record) if record.status != NOT_RUN else None
                upstream = record.ran
                records["prefix"] = record
                self.prefix_records[target.key] = record
            self.prefix_done[target.key].set()
            if not full:
                return
            blocked = next((r.stage for r in records.values() if r.status != PASS), None)
            if target.lane:
                if blocked:
                    records["packages"] = self.not_run(target, "packages", f"{blocked} failed")
                else:
                    record = self.stage(target.key, "packages", self.write_packages, upstream, target)
                    self.contract(target, "packages", record)
                    records["packages"] = record
                    upstream = upstream or record.ran
            else:
                self.shared_done[target.arch].wait()
                shared = self.shared_records[target.arch]
                if blocked:
                    records["packages"] = self.not_run(target, "packages", f"{blocked} failed")
                else:
                    record = Record("packages", shared.status, shared.detail, shared.seconds, shared.evidence)
                    if shared.status == NOT_RUN:
                        records["packages"] = self.not_run(target, "packages", shared.detail)
                    else:
                        self.contract(target, "packages", record)
                        write_json(self.record_path(target.key, "packages"), record.data())
                        records["packages"] = record
                    upstream = upstream or shared.ran
            blocked = next((r.stage for r in records.values() if r.status != PASS), None)
            if blocked:
                records["suite"] = self.not_run(target, "suite", f"{blocked} failed")
            else:
                records["suite"] = self.stage(target.key, "suite", self.package_suite, upstream, target, priority)
                upstream = upstream or records["suite"].ran
            if target.graphical is None or not target.graphical.get("packaged"):
                records["graphical"] = self.not_run(target, "graphical", "not a graphical target")
            elif self.args.no_graphical:
                records["graphical"] = Record("graphical", NOT_RUN, "--no-graphical")
            else:
                blocked = next((r.stage for r in records.values() if r.status != PASS), None)
                if blocked:
                    records["graphical"] = self.not_run(target, "graphical", f"{blocked} failed")
                else:
                    records["graphical"] = self.stage(
                        target.key, "graphical", self.graphical_test, upstream, target, priority
                    )
        finally:
            self.prefix_done[target.key].set()
            if full:
                write_json(
                    self.dir(target.key) / "verdict.json",
                    {
                        "slug": target.slug,
                        "arch": target.arch,
                        "stages": {name: record.data() for name, record in records.items()},
                        "verdict": verdict(records),
                    },
                )

    def run_shared(self, arch, providers):
        """The packages-shared job: --lanes other on every shared prefix of the architecture."""
        try:
            for provider in providers:
                self.prefix_done[provider.key].wait()
            failed = [
                p.slug for p in providers if self.prefix_records.get(p.key, Record("prefix", NOT_RUN)).status != PASS
            ]
            ran = any(self.prefix_records.get(p.key, Record("prefix", NOT_RUN)).ran for p in providers)
            key = f"shared-{arch}"
            if failed:
                record = Record("packages", NOT_RUN, f"shared prefix {', '.join(failed)} failed")
                remove(self.record_path(key, "packages"))
            else:
                record = self.stage(key, "packages", self.write_shared, ran, arch, providers)
            self.shared_records[arch] = record
        finally:
            self.shared_records.setdefault(arch, Record("packages", NOT_RUN, "the shared packages did not run"))
            self.shared_done[arch].set()

    # -- the stages

    def take(self, slots, priority, stage, key):
        """Wait for a slot and for room on the disk. The wait counts as queued time, not as time of the stage."""
        begun = time.monotonic()
        slots.acquire(priority)
        self.wait_for_room(stage, key)
        self.local.queued = getattr(self.local, "queued", 0.0) + time.monotonic() - begun
        self.log(f"{key} {stage}: running")

    def build_prefix(self, target, priority):
        self.prepare_source()
        log = self.log_path(target.key, "prefix")
        work = self.work(target.key)
        source, out = work / "source", work / "out"
        destination = self.root / "prefixes" / target.prefix_name / "prefix.tar.zst"
        remove(destination.parent)
        name = f"gnoblin-pipeline-prefix-{target.key}"
        self.take(self.build_slots, priority, "prefix", target.key)
        try:
            remove(work)
            out.mkdir(parents=True)
            subprocess.run(["cp", "-a", "--reflink=auto", str(self.scratch / "source"), str(source)], check=True)
            subprocess.run(["podman", "rm", "-f", name], capture_output=True, check=False)
            jobs = self.args.prefix_jobs or max(2, (os.cpu_count() or 4) // self.args.jobs)
            command = [
                "podman", "run", "--rm", "-i", "--name", name, "--security-opt", "label=disable",
                "--volume", f"{source}:/work", "--volume", f"{out}:/out",
                "--volume", f"{self.common_dir}:{self.common_dir}:ro", "--workdir", "/work",
                "--env", "GIT_COMMITTER_NAME=Gnoblin release automation",
                "--env", "GIT_COMMITTER_EMAIL=release@gnoblin.local",
                "--env", f"BOOTSTRAP={target.prefix['bootstrap']}",
                "--env", f"DEPS_SCRIPT={target.prefix['deps_script']}",
                "--env", f"GNOBLIN_PREFIX_JOBS={jobs}",
                "--env", f"SOURCE_DATE_EPOCH={self.source_date}",
                target.prefix["prefix_image"], "sh", "-eu",
            ]  # fmt: skip
            status = self.run(command, log, TIMEOUTS["prefix"], stdin=PREFIX_SCRIPT, container=name)
            if (source / "build" / "logs").is_dir():
                remove(self.dir(target.key) / "prefix" / "build-logs")
                shutil.copytree(source / "build" / "logs", self.dir(target.key) / "prefix" / "build-logs")
            archive = out / "prefix.tar.zst"
            if status == 0 and archive.is_file():
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.move(archive, destination)
                done = next((line for line in reversed(log.read_text().splitlines()) if "[prefix] done" in line), "")
                return PASS, done.split("done: ", 1)[-1][:200], str(log)
            reason = "timed out" if status == 124 else f"exited {status}"
            return FAIL, f"prefix build {reason}: {failure_line(log)}", str(log)
        finally:
            self.build_slots.release()
            remove(work)

    def unpack(self, archive, into):
        remove(into)
        into.mkdir(parents=True)
        subprocess.run(["tar", "--zstd", "-xf", str(archive), "-C", str(into)], check=True)

    def write_packages(self, target):
        """The packages job: the package lane of the target, from its own prefix."""
        log = self.log_path(target.key, "packages")
        out = self.root / "artifacts" / f"{target.lane['packages_artifact']}-{self.tag}-{target.slug}-{target.arch}"
        tree = self.work(target.key) / "prefix-tree"
        remove(out)
        try:
            self.unpack(self.root / "prefixes" / target.prefix_name / "prefix.tar.zst", tree)
            env = self.package_env()
            command = [REPO / "scripts/build-packages.sh", "--prefix", tree, "--output", out]
            status = self.run(command + shlex.split(target.lane["package_args"]), log, TIMEOUTS["packages"], env=env)
            if status != 0:
                return FAIL, f"build-packages.sh exited {status}: {failure_line(log)}", str(log)
            rename_epochs(out)
            if target.lane["kind"] == "rpm":
                rpms = sorted(out.glob("*.rpm"))
                status = self.run(["python3", REPO / "scripts/check-rpm-isolation.py", *rpms], log, 600, env=env)
                if status != 0:
                    return FAIL, f"check-rpm-isolation.py exited {status}: {failure_line(log)}", str(log)
            files = sorted(path.name for path in out.iterdir())
            return PASS, f"{len(files)} files in {out.name}", str(log)
        finally:
            remove(tree)

    def write_shared(self, arch, providers):
        key = f"shared-{arch}"
        log = self.log_path(key, "packages")
        out = self.root / "artifacts" / f"packages-{self.tag}-{arch}"
        remove(out)
        env = self.package_env()
        for provider in providers:
            tree = self.work(key) / provider.slug
            try:
                self.unpack(self.root / "prefixes" / provider.prefix_name / "prefix.tar.zst", tree)
                command = [REPO / "scripts/build-packages.sh", "--prefix", tree, "--output", out, "--lanes", "other"]
                status = self.run(command, log, TIMEOUTS["shared"], env=env)
            finally:
                remove(tree)
            if status != 0:
                return (
                    FAIL,
                    f"build-packages.sh --lanes other on {provider.slug} exited {status}: {failure_line(log)}",
                    str(log),
                )
        for command, name in (
            (["python3", REPO / "tests/pkgsmith-packages.test.py"], "pkgsmith-packages.test.py"),
            (
                ["python3", REPO / "scripts/check-rpm-isolation.py", *sorted(out.glob("*.rpm"))],
                "check-rpm-isolation.py",
            ),
        ):
            status = self.run(command, log, 1200, env=env)
            if status != 0:
                return FAIL, f"{name} exited {status}: {failure_line(log)}", str(log)
        rename_epochs(out)
        return PASS, f"{sum(1 for _ in out.iterdir())} files in {out.name}", str(log)

    def package_suite(self, target, priority):
        log = self.log_path(target.key, "suite")
        report = self.dir(target.key) / "package-tests"
        view = self.work(target.key) / "prefixes"
        remove(view)
        view.mkdir(parents=True)
        # The prefixes that wrote these packages, as packages.yml downloads them for the reproducibility check.
        for folder in sorted((self.root / "prefixes").glob("*")):
            own = target.lane and folder.name == target.prefix_name
            shared = (
                not target.lane
                and folder.name.startswith(f"prefix-shared-{self.tag}-")
                and folder.name.endswith(f"-{target.arch}")
            )
            if own or shared:
                (view / folder.name).symlink_to(folder)
        self.take(self.build_slots, priority, "suite", target.key)
        try:
            env = dict(self.env, CONTAINER_ENGINE="podman")
            command = [
                REPO / "scripts/run-package-tests.sh", "--target", target.slug, "--arch", target.arch,
                "--targets", self.root / "targets.json", "--artifacts", self.root / "artifacts",
                "--release-tag", self.tag, "--report", report, "--prefixes", view,
            ]  # fmt: skip
            status = self.run(command, log, TIMEOUTS["suite"], env=env)
        finally:
            self.build_slots.release()
            remove(view)
        result = read_json(report / "report.json")
        if result is None:
            return FAIL, f"run-package-tests.sh exited {status} and wrote no report.json: {failure_line(log)}", str(log)
        if result.get("result") == PASS:
            return PASS, "every layer passed", str(report / "report.json")
        return FAIL, suite_failure(result), str(report / "report.json")

    def graphical_test(self, target, priority):
        log = self.log_path(target.key, "graphical")
        out = self.dir(target.key) / "graphical"
        work = self.work(target.key) / "vm"
        entry = self.work(target.key) / "graphical-entry.json"
        write_json(entry, target.graphical)
        work.mkdir(parents=True, exist_ok=True)
        self.take(self.vm_slots, priority, "graphical", target.key)
        try:
            command = [
                "python3", REPO / "tests/graphical/harness.py", "--entry", entry,
                "--packages", self.root / "artifacts" / target.artifact, "--out", out, "--work", work,
                "--cache", self.scratch / "images", "--memory", str(self.args.vm_memory),
                "--min-free-gb", str(self.args.min_free_gb + 2), "--artifact", f"graphical-{target.key}",
                "--run-url", "local",
            ]  # fmt: skip
            status = self.run(command, log, TIMEOUTS["graphical"])
        finally:
            self.vm_slots.release()
            remove(work)
        result = read_json(out / "result.json")
        if result is None:
            return FAIL, f"harness.py exited {status} and wrote no result.json: {failure_line(log)}", str(log)
        if result.get("result") == PASS:
            return PASS, f"{len(result.get('checks', []))} checks passed", str(out / "result.json")
        first = result.get("first_failure") or {}
        return (
            FAIL,
            f"{first.get('name', result.get('result'))}: {first.get('detail', '')}"[:300],
            str(out / "result.json"),
        )

    # -- the whole run

    def selected(self):
        only = [item.strip() for item in self.args.only.split(",") if item.strip()] if self.args.only else []
        targets = [t for t in self.targets if t.arch == HOST_ARCH and (not only or t.slug in only)]
        unknown = [slug for slug in only if slug not in {t.slug for t in self.targets}]
        if unknown:
            raise SystemExit(f"run-pipeline: not a target of the list: {', '.join(unknown)}")
        targets.sort(key=lambda t: only.index(t.slug) if only else 0)
        return targets

    def monitor(self):
        lowest = {"disk": None, "memory": None, "swap": 0.0}
        while not self.stopping.wait(60):
            disk = free_gb(self.root)
            memory, swap = memory_gb()
            lowest["disk"] = disk if lowest["disk"] is None else min(lowest["disk"], disk)
            lowest["memory"] = memory if lowest["memory"] is None else min(lowest["memory"], memory)
            lowest["swap"] = max(lowest["swap"], swap)
            with open(self.root / "resources.log", "a") as handle:
                handle.write(f"{now()} free {disk:.1f} GiB, RAM available {memory:.1f} GiB, swap used {swap:.1f} GiB\n")
            write_json(self.root / "resources.json", lowest)

    def run_all(self):
        selected = self.selected()
        started = time.monotonic()
        threading.Thread(target=self.monitor, daemon=True).start()
        # The copy of the source first, so that the prefix builds queue for their slots in the order of priority.
        if any(t.prefix for t in selected):
            self.prepare_source()
        threads = []
        providers = {}
        for arch in {t.arch for t in selected}:
            consumers = [t for t in selected if t.arch == arch and not t.lane]
            shared = [
                t for t in self.targets if t.arch == arch and t.prefix and "shared" in t.prefix["prefix_artifact"]
            ]
            if consumers:
                providers[arch] = shared
            self.shared_done[arch] = threading.Event()
            if consumers:
                threads.append(threading.Thread(target=self.run_shared, args=(arch, shared), name=f"shared-{arch}"))
            else:
                self.shared_done[arch].set()
        extra = [t for arch in providers for t in providers[arch] if t not in selected]
        for priority, target in enumerate(selected):
            threads.append(threading.Thread(target=self.run_target, args=(target, priority, True), name=target.key))
        for target in extra:
            threads.append(
                threading.Thread(target=self.run_target, args=(target, len(selected), False), name=target.key)
            )
        for thread in threads:
            # A stopped run must not wait for stages that it has killed.
            thread.daemon = True
            thread.start()
            time.sleep(0.2)
        signal.signal(signal.SIGTERM, stop_on_term)
        try:
            for thread in threads:
                while thread.is_alive():
                    thread.join(5)
        except KeyboardInterrupt:
            self.log("stopping: killing the running stages")
            self.stopping.set()
            with self.lock:
                processes = list(self.processes)
            for process in processes:
                self.kill(process)
            subprocess.run(
                "podman ps -aq --filter name=gnoblin-pipeline- | xargs -r podman rm -f", shell=True, check=False
            )
            raise
        finally:
            self.stopping.set()
        wall = time.monotonic() - started
        write_json(
            self.root / "wall.json",
            {"seconds": round(wall, 1), "finished": now(), "targets": [t.key for t in selected]},
        )
        self.log(f"run finished in {minutes(wall)}")


def stop_on_term(number, frame):
    """A SIGTERM stops the run as Ctrl-C does: the running stages are killed and their containers removed."""
    raise KeyboardInterrupt


def rename_epochs(folder):
    """The Arch packages carry the epoch in their name (gnoblin-1:0.1.13-1-x86_64.pkg.tar.zst). An artifact takes no
    colon, so CI names them with a dot, and so does this run."""
    for path in Path(folder).glob("*:*"):
        path.rename(path.with_name(path.name.replace(":", ".")))


def suite_failure(report):
    for name in LAYERS:
        layer = (report.get("layers") or {}).get(name) or {}
        if layer.get("status") != PASS:
            case = next((c for c in layer.get("cases", []) if c.get("status") == FAIL), None)
            if case:
                return f"{name} {case['id']}: {case.get('title', '')}: {case.get('message', '').splitlines()[0] if case.get('message') else ''}"[
                    :300
                ]
            return f"{name}: {layer.get('status', 'missing')}"
    return "failed"


def verdict(records):
    """pass when every stage that applies passed. A graphical test that does not apply does not count."""
    for name in STAGES:
        record = records.get(name)
        if record is None:
            continue
        if name == "graphical" and record.status == NOT_RUN and record.detail == "not a graphical target":
            continue
        if record.status != PASS:
            return f"{FAIL if record.status == FAIL else NOT_RUN}: {name}"
    return PASS


# -- the matrix


def stage_cell(root, key, stage):
    """The cell of one stage, from the result file that the gate reads. A record without that file is not a pass."""
    record = read_json(root / key / "logs" / f"{stage}.json") or {}
    seconds = record.get("seconds")
    contract = {
        "prefix": root / key / "prefix" / "stage.json",
        "packages": root / key / "packages" / "stage.json",
        "graphical": root / key / "graphical" / "result.json",
    }
    if stage in contract:
        data = read_json(contract[stage])
        if data is None:
            return NOT_RUN, "", ""
        status = data.get("status") if stage != "graphical" else data.get("result")
        if stage == "graphical" and status != PASS:
            first = data.get("first_failure") or {}
            return FAIL, f"{first.get('name', status)}: {first.get('detail', '')}"[:160], minutes(record.get("seconds"))
        return (
            (PASS if status == PASS else FAIL),
            ("" if status == PASS else data.get("detail", "")[:160]),
            minutes(seconds),
        )
    raise ValueError(stage)


def layer_cells(root, key):
    report = read_json(root / key / "package-tests" / "report.json")
    if report is None:
        return {name: (NOT_RUN, "") for name in LAYERS}
    cells = {}
    for name in LAYERS:
        layer = (report.get("layers") or {}).get(name) or {}
        status = layer.get("status", "missing")
        detail = ""
        if status == FAIL:
            case = next((c for c in layer.get("cases", []) if c.get("status") == FAIL), None)
            detail = f"{case['id']} {case.get('title', '')}"[:120] if case else "no cases"
        cells[name] = (PASS if status == PASS else FAIL if status == FAIL else NOT_RUN, detail)
    return cells


def gate_decisions(root):
    """The publish decision of each target (coverage.json of tests/packages/gate.py) and the issue action of each
    release (actions.json of tests/pipeline/issues.py), when they ran."""
    decisions, actions = {}, {}
    coverage = read_json(root / "coverage" / "coverage.json") or {}
    for item in coverage.get("targets", []):
        if item.get("arch") in (None, "-"):
            continue
        key = f"{item['slug']}-{item['arch']}"
        reason = item.get("reason") or "install check only, no published lane"
        decisions[key] = "publish" if item.get("published") else f"held: {reason}"[:160]
    for item in read_json(root / "issues" / "actions.json") or []:
        action = item.get("action", "")
        text = "EOL: no issue" if action == "notice" else action
        if item.get("number"):
            text += f" #{item['number']}"
        actions[item.get("release", "")] = text
    return decisions, actions, bool(coverage)


def matrix(root, data, today, tag="local"):
    """The matrix as Markdown, and the list of failures with their evidence."""
    root = Path(root)
    targets = [Target(entry, data, tag) for entry in data["matrix"]]
    decisions, actions, gated = gate_decisions(root)
    header = (
        "| Target | Arch | EOL | State | Prefix | Packages | Suite | Static | Install | Runtime | Repository "
        "| Graphical | Verdict | Gate | Issue action | Time | Evidence |"
    )
    lines = [header, "|" + "---|" * (header.count("|") - 1)]
    failures = []
    for target in targets:
        eol, date = target.eol(today)
        eol_cell = f"EOL {date}" if eol else (f"until {date}" if date else "rolling")
        state = (target.row or {}).get("state", target.entry.get("state", ""))
        if target.arch != HOST_ARCH:
            reason = f"not run: no {target.arch} runner here"
            lines.append(
                f"| {target.entry['name']} | {target.arch} | {eol_cell} | {state} | {reason} |"
                + " |" * 7
                + f" {NOT_RUN} | {decisions.get(target.key, '')} | | | |"
            )
            continue
        cells, total = [], 0.0
        stage_records = (read_json(root / target.key / "verdict.json") or {}).get("stages", {})
        for stage in ("prefix", "packages"):
            if stage == "prefix" and not target.prefix:
                cells.append("shared prefix")
                continue
            status, detail, took = stage_cell(root, target.key, stage)
            if status == NOT_RUN:
                reason = stage_records.get(stage, {}).get("detail", "")
                cells.append(f"not run{': ' + reason if reason else ''}")
            else:
                cells.append(f"{PASS} {took}".strip() if status == PASS else f"**FAIL** {took}".strip())
                if status == FAIL:
                    failures.append((target.key, stage, detail, root / target.key / "logs" / f"{stage}.log"))
        report = root / target.key / "package-tests" / "report.json"
        suite_record = read_json(root / target.key / "logs" / "suite.json") or {}
        layer = layer_cells(root, target.key)
        if not report.is_file():
            reason = stage_records.get("suite", {}).get("detail", "") or suite_record.get("detail", "")
            cells.append(f"not run{': ' + reason if reason else ''}" if suite_record.get("status") != FAIL else
                         f"**FAIL** no report.json: {reason}")  # fmt: skip
            if suite_record.get("status") == FAIL:
                failures.append((target.key, "suite", reason, root / target.key / "logs" / "suite.log"))
        else:
            passed = all(layer[name][0] == PASS for name in LAYERS)
            cells.append(f"{PASS if passed else '**FAIL**'} {minutes(suite_record.get('seconds'))}".strip())
        for name in LAYERS:
            status, detail = layer[name]
            cells.append({PASS: PASS, FAIL: f"**FAIL** {detail.split(' ')[0]}"}.get(status, NOT_RUN))
            if status == FAIL:
                failures.append((target.key, f"suite {name}", detail, report))
        if target.graphical is None or not target.graphical.get("packaged"):
            cells.append("not a graphical target")
        else:
            status, detail, took = stage_cell(root, target.key, "graphical")
            if status == NOT_RUN:
                reason = stage_records.get("graphical", {}).get("detail", "")
                cells.append(f"not run{': ' + reason if reason else ''}")
            else:
                cells.append(f"{PASS} {took}" if status == PASS else f"**FAIL** {detail.split(':')[0]} {took}")
                if status == FAIL:
                    failures.append((target.key, "graphical", detail, root / target.key / "graphical" / "result.json"))
        for stage in STAGES:
            total += float((read_json(root / target.key / "logs" / f"{stage}.json") or {}).get("seconds") or 0)
        action = actions.get(target.entry["name"], "none" if gated else "")
        lines.append(
            f"| {target.entry['name']} | {target.arch} | {eol_cell} | {state} | " + " | ".join(cells) + " | "
            f"{verdict_from_cells(cells)} | {decisions.get(target.key, '')} | {action} | {minutes(total)} | "
            f"{root / target.key} |"
        )
    out = ["## Matrix", "", *lines, "", "## Failures", ""]
    if not failures:
        out.append("None.")
    for key, stage, detail, evidence in failures:
        out.append(f"- `{key}` {stage}: {detail or 'see the evidence'} ({evidence})")
    return "\n".join(out) + "\n"


def verdict_from_cells(cells):
    """pass only when every cell that applies says pass: a cell that is not run or that failed is no pass."""
    applies = [cell for cell in cells if cell not in ("shared prefix", "not a graphical target")]
    if any("FAIL" in cell for cell in applies):
        return "**fail**"
    if all(cell.startswith(PASS) for cell in applies):
        return PASS
    return NOT_RUN


def generated_targets(args):
    if args.targets:
        return json.loads(Path(args.targets).read_text())
    command = [sys.executable, str(REPO / "scripts/supported-targets.py"), "--rows"]
    if args.today:
        command += ["--today", args.today]
    return json.loads(subprocess.run(command, check=True, capture_output=True, text=True).stdout)


def gate_and_report(root, tag, log):
    """The release gate and the issue report in dry run, as the release workflows run them. Their exit status is
    recorded, not raised: a failed target makes the gate fail, and the matrix shows why."""
    root = Path(root)
    no_issues = root / "open-issues.json"
    write_json(no_issues, [])
    steps = [
        [
            sys.executable, REPO / "tests/packages/gate.py", "--targets", root / "targets.json", "--stages", root,
            "--reports", root, "--graphical", root, "--artifacts", root / "artifacts", "--release-tag", tag,
            "--out", root / "coverage",
        ],
        [
            sys.executable, REPO / "tests/pipeline/issues.py", "--coverage", root / "coverage" / "coverage.json",
            "--issues-json", no_issues, "--dry-run", "--run-url", "local", "--out", root / "issues",
        ],
    ]  # fmt: skip
    statuses = []
    with open(log, "a") as handle:
        for command in steps:
            handle.write(f"$ {shlex.join(str(part) for part in command)}\n")
            handle.flush()
            result = subprocess.run([str(part) for part in command], stdout=handle, stderr=subprocess.STDOUT, cwd=REPO)
            handle.write(f"[pipeline] exit status {result.returncode}\n")
            statuses.append(result.returncode)
    return statuses


def clean(root, scratch):
    """Remove what the run made that is not a result: sources, prefixes, packages, guest disks, cloud images, the
    containers it named and the images it pulled."""
    root = Path(root)
    remove(scratch)
    for name in ("prefixes", "artifacts"):
        remove(root / name)
    subprocess.run("podman ps -aq --filter name=gnoblin-pipeline- | xargs -r podman rm -f", shell=True, check=False)
    before = root / "images-before.txt"
    if before.is_file():
        kept = set(before.read_text().split())
        listed = subprocess.run(
            ["podman", "images", "--format", "{{.ID}}"], capture_output=True, text=True, check=False
        ).stdout.split()
        new = sorted(set(listed) - kept)
        if new:
            subprocess.run(["podman", "rmi", "-f", *new], check=False)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n", 1)[0])
    parser.add_argument("--root", default="/var/tmp/pipeline-run", help="where the results, evidence and logs go")
    parser.add_argument("--work", default="/var/tmp/pipeline-run-work", help="where sources, builds and guests go")
    parser.add_argument("--targets", help="the JSON of scripts/supported-targets.py --rows; default: run it now")
    parser.add_argument("--today", help="the date for the target list and the end of life")
    parser.add_argument("--only", help="slugs to run, in this order of priority; default: every target")
    parser.add_argument("--jobs", type=int, default=2, help="container stages at once (prefix builds, package suites)")
    parser.add_argument("--vms", type=int, default=2, help="graphical guests at once")
    parser.add_argument("--prefix-jobs", type=int, default=0, help="compile jobs of one prefix build")
    parser.add_argument("--vm-memory", type=int, default=4096, help="guest memory in MiB")
    parser.add_argument("--min-free-gb", type=float, default=30.0, help="free space under ROOT to keep")
    parser.add_argument("--min-memory-gb", type=float, default=6.0, help="available memory a heavy stage waits for")
    parser.add_argument("--force", action="store_true", help="run every stage again")
    parser.add_argument("--rerun", default="", help="stages to run again, such as suite,graphical")
    parser.add_argument("--no-graphical", action="store_true")
    parser.add_argument("--release-tag", default="local")
    parser.add_argument("--pkgsmith-ref", default=os.environ.get("GNOBLIN_PKGSMITH_REF", ""))
    parser.add_argument("--matrix-only", action="store_true", help="run the gate and the report, print the matrix")
    parser.add_argument("--clean", action="store_true", help="remove everything but the results and the evidence")
    args = parser.parse_args(argv)
    args.rerun = {item.strip() for item in args.rerun.split(",") if item.strip()}
    unknown = args.rerun - set(STAGES)
    if unknown:
        parser.error(f"--rerun takes {', '.join(STAGES)}, not {', '.join(sorted(unknown))}")
    root = Path(args.root).resolve()
    root.mkdir(parents=True, exist_ok=True)
    if args.clean:
        clean(root, Path(args.work).resolve())
        return 0
    if args.matrix_only and (root / "targets.json").is_file():
        data = read_json(root / "targets.json")
    else:
        data = generated_targets(args)
        write_json(root / "targets.json", data)
    if not args.matrix_only:
        before = root / "images-before.txt"
        if not before.is_file():
            listed = subprocess.run(["podman", "images", "--format", "{{.ID}}"], capture_output=True, text=True)
            before.write_text(listed.stdout)
        pipeline = Pipeline(args, data)
        pipeline.run_all()
    statuses = gate_and_report(root, args.release_tag, root / "gate.log")
    today = args.today or datetime.date.today().isoformat()
    text = matrix(root, data, today, args.release_tag)
    wall = read_json(root / "wall.json") or {}
    resources = read_json(root / "resources.json") or {}
    head = [
        "# Local pipeline run",
        "",
        f"Written {now()} by scripts/run-pipeline.py. Root {root}. Release tag {args.release_tag}.",
        f"Last run wall time {minutes(wall.get('seconds'))}; gate exit {statuses[0]}, issue report exit {statuses[1]}"
        f" (log {root / 'gate.log'}).",
        "A stage time excludes the wait for a slot and for disk room when its record (logs/<stage>.json) has"
        " queued_seconds; an older record without that field includes the wait. Stages kept from an earlier"
        " invocation keep their earlier time.",
    ]
    if resources:
        head.append(
            f"Lowest free disk {resources.get('disk', 0):.1f} GiB, lowest available RAM "
            f"{resources.get('memory', 0):.1f} GiB, most swap used {resources.get('swap', 0):.1f} GiB."
        )
    if args.pkgsmith_ref:
        head.append(f"pkgsmith {args.pkgsmith_ref} replaces the pin for this run (GNOBLIN_PKGSMITH_REF).")
    text = "\n".join(head) + "\n\n" + text
    (root / "MATRIX.md").write_text(text)
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
