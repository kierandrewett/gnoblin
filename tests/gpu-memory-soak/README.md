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
