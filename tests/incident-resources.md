# Incident recorder usage and evidence

Use [the recorder](../scripts/gnoblin-incident-resources.py) to collect bounded
DRM-client holder observations for [#170](https://github.com/kierandrewett/gnoblin/issues/170).
The historical 31 GiB GPU system-memory owner remains unknown. Earlier bounded
blur, capture and hardware churn did not reproduce that allocation.

This is an opt-in diagnostic companion. It does not install a service, detect
the active desktop, or replace the existing incident recorder's PSS, CPU and PSI
measurements. Main owns integration and acceptance of its runtime cost.

## Candidate selection

Before reading fdinfo, the reader calls `stat()` on `/proc/PID/fd/N` and checks
whether the referenced inode is a character device. It does not inspect the
device pathname or require a particular major number. Renamed nodes, bind
mounts and nonstandard device paths therefore remain candidates. It only reads
metadata; it never opens the referenced device. A failed `stat()` increments
`fdTargetUnavailable` and falls back to reading fdinfo instead of dropping it.

Linux DRM registers character devices in
[`drm_drv.c`](https://github.com/torvalds/linux/blob/v6.17/drivers/gpu/drm/drm_drv.c).
DRM leases use `file_clone_open()` in
[`drm_lease.c`](https://github.com/torvalds/linux/blob/v6.17/drivers/gpu/drm/drm_lease.c).
The filter depends on this character-device interface, not on driver names.
`stat()` selects candidates only; it does not supply a guessed DRM identity.
Identity and deduplication still come from fdinfo as described below.

`fdEntries` counts candidate checks, `nonDeviceFds` counts skipped entries, and
`fdinfoFiles` counts attempted fdinfo reads. `traversalComplete` means the visible
process-directory walk ended without a limit hit. It does not mean every fd was
readable. Read failures still appear under `errors`. Fd reuse between metadata
and fdinfo reads remains a sampling race, just as process exit does.

## Run and stop

From the repository root, create a private output directory and collect one
sample as the current user:

```sh
mkdir -p "${XDG_STATE_HOME:-$HOME/.local/state}"
incident_dir="$(mktemp -d "${XDG_STATE_HOME:-$HOME/.local/state}/gnoblin-gpu-incident.XXXXXX")"
python3 scripts/gnoblin-incident-resources.py --output "$incident_dir"
```

The command writes `gpu-resources.jsonl` and exits without printing process
records. To collect ten samples, waiting ten seconds after each completed scan:

```sh
python3 scripts/gnoblin-incident-resources.py --output "$incident_dir" --samples 10 --interval 10
```

`--samples` accepts 1–360, default 1. `--interval` accepts 10–3600 seconds,
default 10. `--proc-root` defaults to `/proc`; use an override only for offline
fixtures. Neither command requests elevated permissions or opens a GPU device.

Ctrl+C or SIGTERM interrupts the wait. During scanning, cancellation takes
effect at the next budget check. File and directory handles close before exit;
no child processes or background jobs are started. After retaining any required
evidence, remove only the records from this explicitly created directory:

```sh
rm -- "$incident_dir"/gpu-resources*.jsonl
rmdir -- "$incident_dir"
```

## Interpret identity and coverage

Each row includes `memKiB`, `gpuClients` and `coverage`. Each client observation
has numeric `memoryBytes` and `engineNs`, plus holders identified by `pid`,
`startTicks`, `name` and `fdCount`. Process identity is checked again after its
fd scan. `startTicks` is field 22 of `/proc/PID/stat`; it distinguishes PID reuse
within a boot, not across boots.

With `identityScope: "device"`, the key is driver, PCI device address and client
ID. Repeated fds for that key add holders without adding usage counters. The
first observed fd supplies the counters; observations are not atomic.

With `identityScope: "fd"`, `drm-pdev` was absent. Each fd remains a separate
observation, distinguished by `observedFd` and its holder. The recorder increments
`coverage.errors.ambiguousDeviceFds`. Such rows may describe the same allocation
more than once and must not be summed or treated as distinct GPU clients.

The [kernel fdinfo contract](https://docs.kernel.org/gpu/drm-usage-stats.html#identification)
permits globally unique IDs or IDs unique within a device, with `drm-pdev`
required for the latter. The common allocator in
[Linux v6.17](https://github.com/torvalds/linux/blob/v6.17/drivers/gpu/drm/drm_file.c)
uses a global atomic counter and emits `drm-pdev` for PCI devices. That source
does not establish every driver's or installed kernel's behavior. Missing
`drm-pdev` is therefore treated conservatively, without inferring a device.

Memory categories overlap. Do not add total, resident, shared and purgeable
values. The deprecated AMD `drm-memory-*` alias is normalized to resident;
explicit resident values take precedence. Different clients can share buffers,
so even deduplicated client counters do not sum to physical `GPUActive`.
Observed holders do not establish which process allocated a buffer.

Inspect `coverage.errors` and `coverage.limitsHit` before interpreting a sample.
Permission denials, vanished or oversized files, malformed fields, missing
identity and ambiguous devices are recorded separately. Absent counters are
unknown, not zero. Counts describe observed failures, not every hidden process.

Each sample permits at most 4096 processes, 65536 fd entries, 8192 fdinfo reads, 256 client rows
and 512 holder entries. Each read is capped at 16 KiB; each fd has at most 64
usage counters. The process-directory walk also stops after 8192 entries.
A cooperative 250 ms deadline is checked between reads; an individual slow
kernel read can exceed it. Samples restart directory traversal, so repeated
timeouts can repeatedly omit later entries. An error-free sample still cannot
establish coverage beyond the collector's procfs visibility and permissions.

Retained output is capped at four 4 MiB files, rotated before appending. The
directory must be caller-owned and private; log files are mode 0600. A directory
lock excludes a second writer. Arguments, executable paths, environments and
client-supplied names are excluded. Process names and identifiers remain local
diagnostic data. Exited clients, kernel-only allocations and events between
samples remain unobservable.

## Repeat the bounded cost measurement

The following is the actual paired command used on 2026-10-13 at 01:38:24 UTC.
It alternates baseline and candidate, taking ten read-only host samples of each
with default limits and no generated load. The baseline is main's saved,
uncommitted delivery copy. Preserve the measured versions to repeat the
comparison; the paths alone do not establish their contents.

Measured SHA-256 values:

- Baseline: `224820e1817f5d8c62b78137072d27b52c5536d9f6642c86cabdbeb41682de4d`.
- Candidate: `74f333301834186790a89269426f55f93b4a78a8f202c07500c737df7429ed6b`.

Only counts and coverage are printed; no process records are written. Timing
covers `snapshot()`, excluding import, JSON formatting, output and `RecordWriter`
costs. CPU time is this Python process's user plus system CPU time, not total
host CPU utilization. The fixed baseline-first order and changing host workload
limit causal comparisons; this is a bounded operational check.

```sh
python3 - <<'PY'
import datetime
import hashlib
import importlib.util
import json
import statistics
import time
from pathlib import Path

paths = {
    'baseline': Path('/var/tmp/gnoblin-incident-delivery-20261013/scripts/gnoblin-incident-resources.py'),
    'candidate': Path('scripts/gnoblin-incident-resources.py'),
}
modules = {}
for name, path in paths.items():
    spec = importlib.util.spec_from_file_location(name, path)
    modules[name] = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(modules[name])
print(json.dumps({'startedUtc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                  'sha256': {k: hashlib.sha256(p.read_bytes()).hexdigest() for k, p in paths.items()}}))
measurements = {name: [] for name in paths}
for index in range(10):
    for name, recorder in modules.items():
        wall, cpu = time.perf_counter(), time.process_time()
        row = recorder.snapshot()
        cpu_ms, wall_ms = (time.process_time()-cpu)*1000, (time.perf_counter()-wall)*1000
        result = {'sample': index+1, 'version': name, 'wallMs': round(wall_ms, 3),
                  'cpuMs': round(cpu_ms, 3), 'clientRows': len(row['gpuClients']),
                  'holderRows': sum(len(c['holders']) for c in row['gpuClients']),
                  'coverage': row['coverage']}
        measurements[name].append(result)
        print(json.dumps(result))
        del row
print(json.dumps({name: {key: {'min': min(r[key] for r in results),
                              'median': statistics.median(r[key] for r in results),
                              'max': max(r[key] for r in results)}
                        for key in ('wallMs','cpuMs')}
                  for name, results in measurements.items()}))
PY
```

| Sample | Baseline wall ms | Baseline CPU ms | Candidate wall ms | Candidate CPU ms |
| ------ | ---------------- | --------------- | ----------------- | ---------------- |
| 1      | 250.102          | 247.959         | 146.681           | 136.147          |
| 2      | 250.593          | 248.606         | 119.287           | 117.666          |
| 3      | 250.063          | 247.674         | 115.646           | 114.851          |
| 4      | 250.069          | 247.536         | 115.350           | 114.580          |
| 5      | 250.065          | 247.657         | 116.888           | 114.966          |
| 6      | 253.930          | 252.226         | 115.456           | 114.715          |
| 7      | 250.276          | 248.686         | 115.931           | 114.908          |
| 8      | 252.120          | 250.371         | 114.002           | 113.316          |
| 9      | 254.938          | 253.496         | 118.845           | 118.190          |
| 10     | 254.038          | 252.825         | 117.613           | 116.862          |

The baseline hit `time` in all ten samples, visiting 516–533 process entries and
reading 6126–6349 fdinfo files. It retained 26 client rows and 32 holder rows each
time. Each sample recorded 306 permission denials, 2–4 oversized files and 0–4
vanished files. Its median wall/CPU times were 250.435/248.646 ms.

The candidate had no limit hits and reported `traversalComplete: true` in all
ten samples. It visited 651–653 process entries, checked 9674–9708 fd entries,
skipped 9146–9180 non-device fds and read 527–532 fdinfo files. It retained 27
client rows and 33 holder rows each time. Each sample recorded 377–379 permission
denials, 0–4 unavailable fd targets and 0–4 vanished files; none had oversized
files. No ambiguous device identity was observed. Its median wall/CPU times
were 116.410/114.937 ms, about 54% less CPU per sample in this comparison.

The increased permission-denial count reflects reaching more process entries;
completed traversal is not full ownership coverage. Both scans are non-atomic.
The candidate still runs a short CPU burst, and this check excludes log-writing
cost and long-session changes. The default ten-second wait amortizes scanning.
If a larger visible fd population hits the time or entry limits, repeated
prefix omission is still possible: the implementation does not retain a cursor.
These ten completed walks justify the smaller filter change for review, not a
guarantee under arbitrary load or identification of the historical OOM owner.

## Focused validation

```sh
python3 -W error::ResourceWarning tests/incident-resources.test.py
similarity-py scripts/gnoblin-incident-resources.py tests/incident-resources.test.py
ruff check scripts/gnoblin-incident-resources.py tests/incident-resources.test.py
ruff format --check scripts/gnoblin-incident-resources.py tests/incident-resources.test.py
```

Twenty-one synthetic tests pass. They cover shared holders, identity ambiguity,
PID reuse, inaccessible and disappearing files, units and aliases, scan limits,
private bounded rotation, exclusive writers and SIGTERM cleanup. The new
missing-device fixture produced one merged row before the change and three
separate ambiguous observations afterward. Tests create private temporary
procfs fixtures and clean them up; they do not scan host `/proc`. New filter
tests cover a nonstandard device alias with no major-number restriction, skipped
non-device fdinfo, unavailable/denied target metadata with fdinfo fallback,
the separate entry budget, and a later process reached without spending the
read budget on earlier non-device fds. The first three filter checks failed on
the unfiltered reader before the implementation changed.

Prior replay of 187 recorded rows parsed 5111 client records and verified
163552 memory values without parser errors. Those existing records had PCI
device identifiers. The missing-device behavior is proven by synthetic data.

Scoped `similarity-py` reported five test-only pairs, scoring 85.47–95.83%:
the alias/invalid-unit test paired with the budget, missing-ID and oversized
input tests; the missing-ID test paired with the oversized-input test; the
unavailable-target fallback test paired with the counter-limit test. These
share fixture-and-assert structure but cover different failure modes. No
production match was reported, and no extraction was warranted. Ruff 0.16.7
lint and formatting checks passed using the existing pre-commit environment.

No host stress, VM/GUI work, privilege changes, service integration, installation
or commit was performed for this validation. Keep #170 open pending incident
ownership evidence; main reviews and integrates this scoped change.
