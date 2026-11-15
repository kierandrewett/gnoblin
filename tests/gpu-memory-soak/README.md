# GPU memory soak series

The series of the three 25-minute soaks of lane GpuMemory (issue gnoblin-4z5), recorded by
`crates/gnoblin/tests/gpu_memory_soak.rs`. The method, what each run held in the churn and what the numbers say are in
`design/gpu-memory.md`, section 3.

| File             | What it is                                                                                       |
| ---------------- | ------------------------------------------------------------------------------------------------ |
| `software.jsonl` | The software renderer: one JSON line every 15 s, 1505 s, exit 0.                                 |
| `gles.jsonl`     | GLES on llvmpipe, same run, exit 0.                                                              |
| `vulkan.jsonl`   | Vulkan on lavapipe: stopped by the 6144 MiB limit after 187 s, exit 101 (the run's own failure). |
| `RUNS.txt`       | The command line, start and end time and exit code of each run, the host and the Mesa packages.  |
| `summarize.py`   | `python3 summarize.py *.jsonl` prints the table of the design note.                              |

Each line holds the time, the cycle, the windows mapped and in the scene, the supersample factor, the screenshots, the
lock cycles, the capture frames, refusals and sessions, the process's resident set, threads and descriptors, the sum of
`gpu_memory_bytes`, `_capture_bytes` and `_staging_bytes`, and every screen's `gpu_memory_*` properties.

## Limits

- The host is a gVisor build instance with no GPU and no DRM node: the renderers are llvmpipe (software GLES) and lavapipe
  (software Vulkan), they report no video memory, and the `drm-memory-*` counters of fdinfo are not measured.
- The test binary is a debug build, and the compositor and the stock shell share its process, so the resident set
  includes the shell.
- The Vulkan run had the validation layer removed from the instance and stopped at 6144 MiB on purpose.
- Twenty-five minutes shows a leak of kilobytes a cycle only when it adds up to more than the allocator's noise.

## The budget series (lane GpuBudget)

Four more 25-minute runs, on the compositor with the enforced memory budget (issue gnoblin-4z5 stays open: the legacy
31 GiB `GPUActive` is still unattributed). Each is the same churn as above; a run with `GNOBLIN_SOAK_BUDGET_MIB=384` adds
a client that leans on the budget every fifth cycle, in turn with a buffer that fits (16 MiB), one a little larger than
what is left, and one as large as the whole budget. The command lines, times and exit codes are in `RUNS-budget.txt`.

| File                        | What it is                                                                              |
| --------------------------- | --------------------------------------------------------------------------------------- |
| `software.budget-384.jsonl` | Software renderer, budget 384 MiB with the pressure client: exit 0.                     |
| `gles.budget-384.jsonl`     | GLES on llvmpipe, budget 384 MiB with the pressure client: exit 0.                      |
| `software.default.jsonl`    | Software renderer, the default budget (6144 MiB), no pressure client: exit 0.           |
| `gles.default.jsonl`        | GLES on llvmpipe, the default budget, no pressure client: exit 0.                       |
| `RUNS-budget.txt`           | Command line, start, end and exit code of each of the four, and the host.               |
| `RED.txt`                   | Each new test failing with the check it tests switched off, and the switches used.      |
| `FLAKE.txt`                 | The 30-run counts of the supersample test that failed once with `UnknownNode`, and why. |

`python3 summarize.py *.jsonl` adds the budget's numbers to its line for these series: refused, evicted, the largest and
last bytes the clients held, and what the pressure client was told.

The small-budget runs end with an assertion: with every client of the churn gone and the recorder closed, no byte stays
counted (`gpu_memory_clients_bytes` returns to 0), and the budget refused something. Both pass. A refused client ends with
`wl_display.error.no_memory` and nothing else of the session notices.

Limits of these runs, besides those above:

- The GLES run with the small budget evicted nothing: its capture recorder is open for the whole run, so the frame cache is
  in use and nothing else the renderer holds can be given back; every request that did not fit was refused. Eviction before
  refusal is what the software run shows (560 MiB given back, 73 refusals).
- The default-budget runs end with the recorder still open, so their last row still counts its 48 MiB buffer.
- The pressure client's buffers are sparse: what is measured is the compositor's accounting, not the machine's memory.
- A DRM backend, a GPU and the `drm-memory-*` counters are not here (no DRM node on the instances).
