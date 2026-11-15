# GPU and large-host memory: attribution, the findings and what is proven

Lane GpuMemory (gnoblin-4z5, GitHub #170, kept open). The legacy Mutter session grew to about 31 GiB of `GPUActive`
(TTM pages) with an RSS under 1 GiB and the owner was never attributed. That was Mutter's session. This lane asks whether
the Rust compositor can grow GPU or host memory without bound, and makes every large allocation attributable.

## 1. Method

Attribution first, then a soak, then bounds only where a measurement showed growth.

- **Attribution.** Each renderer states the bytes of what it made (`gnoblin_render_frame::footprint::Footprint`:
  `target`, `scratch`, `staging`, `views`); the State adds the frame capture keeps; both are published per output as
  `gpu_memory_*_bytes` on the screen node (docs/gpu-memory.md). Per-process DRM client memory is read by
  `scripts/gnoblin-incident-resources.py` (`drm-memory-*`, read-only); the build instances have no DRM node, so it can
  only say "no DRM clients" there.
- **Soak.** `crates/gnoblin/tests/gpu_memory_soak.rs` runs the real compositor (renderer `software`, `gles` or
  `vulkan`), the stock JavaScript shell and a client that opens, resizes and closes windows, takes screenshots,
  cycles supersample 1, 2 and 4 and hotplugs a second output. One JSON line per interval: resident set, threads,
  descriptors and every screen's `gpu_memory_*`.

## 2. Findings

| Site                                                          | Measured or tested                                                                                                                                                                                                                                     | Result                                                                                                                                                                                                                      |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GLES textures of every frame (overlay, cursor, client buffer) | Soak on GLES/llvmpipe: RSS rose about 400 MB a second with two windows open (346 MiB to 7.6 GiB in 23 s). Test: 400 frames left 800 live textures                                                                                                      | **Leak, fixed.** Smithay queues a dropped texture and deletes it only on `cleanup_texture_cache`; the renderer never called it. It now does after each frame, and the shm texture is kept alive for the frame               |
| Vulkan staging and readback buffers                           | Test: 16,789,536 bytes held after one 16 MiB upload; readback of a larger past target stayed                                                                                                                                                           | **Ratchet, fixed.** Trimmed after each frame to a readback of the target and four output-sized uploads                                                                                                                      |
| Capture frame cache per output                                | Test: the entry of a removed output stayed (2 where 1 was left)                                                                                                                                                                                        | **Leak on hotplug, fixed.** Released with the output. Two images per output are otherwise held always, because the frame hook reads back every presented frame; that is bounded by the outputs and is reported as `capture` |
| Screenshot request queue                                      | Test: four timed-out requests left four entries; a full queue made the caller wait the whole deadline                                                                                                                                                  | **Stale entries, fixed.** A request that gives up withdraws itself, at most 8 wait. The control socket serves one request at a time, so growth was rate-limited in practice                                                 |
| Client buffers                                                | Test: a 16384 x 16384 shm buffer on a sparse gigabyte pool and a 16384 x 16384 dmabuf were accepted by Smithay                                                                                                                                         | **Refused now**: more than 16384 on an edge or 2^27 pixels is a protocol error before anything is mapped or imported                                                                                                        |
| Vulkan renderer process growth                                | 90 s soak on lavapipe: RSS 338 MiB to 2371 MiB while the counted bytes stayed under 240 MiB. Probe (`frames_with_no_upload_do_not_grow_the_process`, ignored): 200 solid-colour frames grew the process by 160 MiB, the same with a 16 MiB upload each | **Open.** About 0.8 MiB per frame whatever the frame holds, so it is per submit or per frame recorded (descriptor pool, command buffer, fence) or lavapipe itself; not attributed, not fixed                                |
| Soak result after the fixes                                   | 90 s on software: 124 to 362 MiB, plateau; GLES: 347 to 749 MiB, plateau (before the fix 346 MiB to 7.6 GiB in 23 s)                                                                                                                                   | Bounded over the run                                                                                                                                                                                                        |
| Software renderer client textures                             | Counted as `staging`                                                                                                                                                                                                                                   | Snapshot of every drawn layer each frame; bounded by the windows drawn, not capped                                                                                                                                          |

## 3. Proven and not proven

Proven on the Rust compositor, on the software renderer, GLES on llvmpipe and Vulkan on lavapipe in the gVisor
build instances: the red and green tests above (each failed with its fix disabled, passed with it), and the 90-second
soak series stored with the run. Not proven: a real GPU. llvmpipe and lavapipe report no video memory, the build
instances have no DRM device, and the GLES leak's size on a real driver is unmeasured (gnoblin-lubw tracks the real-GPU
dmabuf check). The 20-minute soak the plan asked for was not run (budget stop); only 90-second series, so slow growth is not excluded. The tiny-surface test maps 200 windows: 1000 took longer than a minute to map in a debug build, which is a cost question for another lane, not a memory one (the process grew 18 MB for 200 and kept 17 MB after they left). The session lock is not in the
churn. The DRM scan-out buffers are not counted. Nothing here attributes the legacy 31 GiB `GPUActive`: that was
Mutter; what this lane adds is the method and the finding that a Rust compositor using Smithay's GLES renderer
without its frame API leaks every imported texture, which is a plausible shape for the same symptom on a real GPU.
