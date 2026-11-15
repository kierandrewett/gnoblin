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

| Site                                                          | Measured or tested                                                                                                                                      | Result                                                                                                                                                                                                                      |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| GLES textures of every frame (overlay, cursor, client buffer) | Soak on GLES/llvmpipe: RSS rose about 400 MB a second with two windows open (346 MiB to 7.6 GiB in 23 s). Test: 400 frames left 800 live textures       | **Leak, fixed.** Smithay queues a dropped texture and deletes it only on `cleanup_texture_cache`; the renderer never called it. It now does after each frame, and the shm texture is kept alive for the frame               |
| Vulkan staging and readback buffers                           | Test: 16,789,536 bytes held after one 16 MiB upload; readback of a larger past target stayed                                                            | **Ratchet, fixed.** Trimmed after each frame to a readback of the target and four output-sized uploads                                                                                                                      |
| Capture frame cache per output                                | Test: the entry of a removed output stayed (2 where 1 was left)                                                                                         | **Leak on hotplug, fixed.** Released with the output. Two images per output are otherwise held always, because the frame hook reads back every presented frame; that is bounded by the outputs and is reported as `capture` |
| Screenshot request queue                                      | Test: four timed-out requests left four entries; a full queue made the caller wait the whole deadline                                                   | **Stale entries, fixed.** A request that gives up withdraws itself, at most 8 wait. The control socket serves one request at a time, so growth was rate-limited in practice                                                 |
| Client buffers                                                | Test: a 16384 x 16384 shm buffer on a sparse gigabyte pool and a 16384 x 16384 dmabuf were accepted by Smithay                                          | **Refused now**: more than 16384 on an edge or 2^27 pixels is a protocol error before anything is mapped or imported                                                                                                        |
| Vulkan renderer process growth                                | Soak on lavapipe: RSS 333 MiB to 6125 MiB in 187 s while the counted bytes stayed under 362 MiB. A loop outside the renderer (section 4) grows the same | **Driver, not the renderer.** Mesa 22.3.6 lavapipe leaks about 0.8 MiB for every frame that draws the shared solid pass. Not fixable here; section 4 has the proof                                                          |
| Soak result after the fixes                                   | 25 minutes each on software and GLES (llvmpipe), with the session lock and a capture session in the churn (section 3)                                   | Bounded: software 475 MiB peak, GLES 851 MiB peak, no descriptor or thread growth; a slow creep in the software mean is not excluded                                                                                        |
| Software renderer client textures                             | Counted as `staging`                                                                                                                                    | Snapshot of every drawn layer each frame; bounded by the windows drawn, not capped                                                                                                                                          |

## 3. The 25-minute soaks

The soak (`crates/gnoblin/tests/gpu_memory_soak.rs`) was run for 1500 s on each renderer in the gVisor build instance
(4 CPUs, Mesa 22.3.6 llvmpipe and lavapipe, debug build), one after the other. Each run opens, resizes and closes windows
(six at a time, one connection per cycle that opens and closes at once), takes a screenshot every cycle, cycles
supersample 1, 2 and 4, plugs and unplugs a second output, takes and releases the session lock every 25 cycles with the
stock shell painting the lock screen, and keeps one `ext_image_copy_capture` session open on the first output for the
whole run, fed a frame whenever the last one ended. The series, the exact command lines, the start and end times and the
exit codes are in `tests/gpu-memory-soak/` (`RUNS.txt`, `*.jsonl`; `summarize.py` prints the table below).

| Renderer          | Samples and cycles           | Lock cycles | Capture frames    | RSS start, peak, end (MiB) | Descriptors, threads | `gpu_memory_bytes` max (MiB) | Exit |
| ----------------- | ---------------------------- | ----------- | ----------------- | -------------------------- | -------------------- | ---------------------------- | ---- |
| software          | 103 over 1505 s, 1266 cycles | 50          | 1165 (50 refused) | 114, 475, 306              | 40-59, 13-16         | 199                          | 0    |
| GLES (llvmpipe)   | 103 over 1505 s, 1194 cycles | 47          | 1099 (47 refused) | 340, 851, 584              | 40-59, 22-25         | 249                          | 0    |
| Vulkan (lavapipe) | 15 over 190 s, 159 cycles    | 6           | 146 (6 refused)   | 333, 6125, 5914            | 40-59, 23-36         | 362                          | 101  |

How to read it:

- **Descriptors and threads stay in a band** on all three: nothing is opened per cycle and kept, including by the lock
  and the capture session. A refused capture frame is a frame asked for while the session was locked; the recorder
  opens a new session after the lock, so `capture_sessions` grows by about one a lock.
- **GLES is flat.** The mean resident set by five-minute block is 638, 735, 708, 674 and 705 MiB, and the peak of the
  last block (812) is under the peak of the third (851). The textures of the cleanup fix are not accumulating.
- **Software creeps a little.** The mean by block is 278, 355, 362, 370 and 385 MiB and the peak 442, 458, 459, 459 and
  475 MiB, a second-half slope of about 3 MiB a minute that the end of the run does not keep (306 MiB with every client
  gone). Over 25 minutes that is bounded, and a leak of a few KiB a cycle is not excluded; the series cannot tell it
  from the allocator keeping freed pages.
- **The counted bytes (`gpu_memory_*`) return to the floor** (9 MiB software, 60 MiB GLES at the end), so nothing the
  renderers count is held after the churn.
- **Vulkan on lavapipe did not finish.** The soak stops and fails when the process passes 6144 MiB
  (`GNOBLIN_SOAK_RSS_LIMIT_MIB`), which it did after 187 s, with the counted bytes at 29 MiB. The validation layer was
  removed from the instance for this run. Section 4 attributes it.

The soak does not show: a real GPU, `drm-memory-*` in fdinfo (the instances have no DRM node, so
`scripts/gnoblin-incident-resources.py` finds no DRM client), the DRM scan-out buffers, or the stock shell's own heap
(it is in the same process here, so it is part of the resident set above).

## 4. Vulkan on lavapipe: whose growth it is

The probe that was ignored since the first soak (`frames_with_no_upload_do_not_grow_the_process`) measured 200
solid-colour frames growing the process by 160 MiB. The attribution, in the order it was done:

1. **Bisect the renderer's stages** (a temporary switch in `frame.rs`): with the draw calls removed and everything else
   left in (fence, command buffer, descriptor pool reset, render pass, clear), the growth was 0; with the draws kept it
   was 329 MiB for 400 frames. Validation layer on or off, blending on or off, the instance buffer's `first_instance`
   and the scissor made no difference.
2. **Allocation stacks.** heaptrack on the probe showed 260 MiB leaked over 131,000 calls, and every stack of a leaked
   allocation is inside `libvulkan_lvp.so`, on lavapipe's own queue thread (`start_thread` at the bottom); no frame of
   `gnoblin-render-vulkan` is in any of them. The largest are blocks of 32,752 bytes.
3. **A loop outside the renderer** (`crates/gnoblin-render-vulkan/src/driver_probe.rs`): its own instance, device, image,
   render pass, pipeline, command buffer and fence, borrowing only the shader compiler. 400 frames of it: **0 KiB with
   no draw, 329,000 KiB with the renderer's solid pass, 0 KiB with the solid pass whose `gnoblin_corner(local)` coverage
   is replaced by `1.0`**. A simpler shader (a flat colour, a smoothstep, a length, a branch over a uniform) does not
   grow; the trigger is that one function of the shared fragment stage as lavapipe compiles or runs it.
   `a_vulkan_loop_without_the_renderer_grows_only_where_the_solid_pass_draws` asserts the two that do not grow and
   prints the third.

So the growth is lavapipe's, set off by the shared solid pass, and not a handle, descriptor set, command buffer, fence or
image of the renderer. It is not fixed here, because there is nothing in the renderer to fix; a lavapipe release newer
than 22.3.6 has not been tried. A real GPU's driver is unmeasured. The consequence is that `renderer = "vulkan"` on a
machine whose only Vulkan device is lavapipe is not fit for a long session (about 0.8 MiB a frame), which is a reason to
leave it off there and not a reason to change the renderer.

`frames_with_no_upload_do_not_grow_the_process` runs now, in a process of its own (the resident set is the process's, and
the tests of a binary run beside each other), and skips itself on a CPU driver; a machine with a GPU is where it can fail.

## 5. Proven and not proven

Proven on the Rust compositor, on the software renderer, GLES on llvmpipe and Vulkan on lavapipe in the gVisor build
instances: the red and green tests of section 2 (each failed with its fix disabled, passed with it), the three 25-minute
series of section 3 with the lock and an open capture session in the churn, and the attribution of section 4.

A thousand 1x1 windows from one client (`a_thousand_tiny_windows_stop_at_the_cap_and_end_with_their_client`, 4 s in a debug
build, two round trips instead of two thousand) leave exactly 256 managed, which is `MAX_WINDOWS` of the window record
(`fallback.rs`): a window past it is left unmanaged. That is a cap on the session, not on a client: when this was measured
no per-client cap on toplevels, surfaces or buffers existed (section 6 adds them), and one client could use all 256. The
session answers a second client meanwhile,
the process grows by 26 MB and keeps all of it after the client leaves (the allocator holds it), and the next client's
window is managed again.

The screen memory numbers are published at most once a period (`gpu_memory_period`, a second) and only when a value
differs. With the production period a quiet stock shell runs no frame and the tree's serial stands still for four
periods (`a_stock_shell_at_the_production_memory_period_does_no_work_when_nothing_happens`); the tests that count the
serial or the shell's frames at a time they choose run with the period at zero.

Not proven: a real GPU (llvmpipe and lavapipe report no video memory, the build instances have no DRM device, and the
GLES leak's size on a real driver is unmeasured; gnoblin-lubw tracks the real-GPU dmabuf check), slow growth beyond 25
minutes, or growth in the software renderer's resident set smaller than the allocator's noise. Nothing here attributes the
legacy 31 GiB `GPUActive`: that was Mutter; what this lane adds is the
method and the finding that a Rust compositor using Smithay's GLES renderer without its frame API leaks every imported
texture, which is a plausible shape for the same symptom on a real GPU. The issue stays open.

## 6. The enforced budget (lane GpuBudget)

Sections 1 to 5 made memory attributable. This section makes it bounded: the compositor refuses or evicts where it used to
grow. `crates/gnoblin-wayland/src/gpu_memory/ledger.rs` is the one place a request is decided; `gpu_memory.rs` measures,
evicts and refuses; the bounds are `compositor.gpu-memory-budget-mib`, `compositor.client-memory-limit-mib` and
`compositor.client-surface-limit` (a zero turns one off; they are read at start).

**What is counted.** Per client: the live buffers and the live surfaces. A shared-memory buffer is its `stride * height`,
the part of its pool it exposes, not the pool: a pool is a sparse mapping the client controls and costs address space,
while the texture a renderer makes from a buffer is the same pixels, counted once. A GPU buffer is `width * height` times
the bytes per pixel of its format (an estimate: tiling and padding add, a planar format is its average; four when the
format is unknown). A toplevel is a surface, so the surface limit is the per-client window limit; `MAX_WINDOWS` (256)
stays the session's managed cap. The aggregate is the clients' bytes plus what section 1 already counts: every renderer's
`target`, `scratch`, `staging` and `views`, and the frame capture keeps. The aggregate is kept apart from the per-client
maps and the tests compare them after churn.

**Defaults.** Budget 6 GiB: well under the 31 GiB that ran a session out, above a desktop of a few 4K clients (32 MiB a
buffer, about 100 MiB of output targets). Per client 2 GiB: a third of the budget, so one client cannot take the session
and two cannot either, and still several of the largest buffer the compositor accepts (512 MiB). Surfaces 4096: a browser
with a hundred tabs holds a few thousand, and each costs a record the bytes do not show (a 1x1 surface is one pixel).

**Where it is decided.** A shared-memory buffer and a surface at the request (`request_guard.rs`, before Smithay makes the
object; what was admitted is recorded once the object exists, so a request Smithay rejects leaves nothing behind, and
`Dispatch::destroyed` gives it back, which a disconnect runs for every object). A GPU buffer where the loop imports it
(`apply_dmabuf_imports`), because the backend is held there. A mode or supersample change in `apply_output_changes`, as the
growth of the output's target. The footprint is measured once a frame, after the frame.

**At the bound.** A client past its own limit is refused. For the aggregate, the ledger first counts what could be given
back (`Footprint::releasable`, View pictures, the capture frame) and admits the request if that covers the excess; the loop
then gives back, cheapest to remake first and only as far as needed: the capture frame, View pictures
(`Backend::release_views`), blur scratch and the software renderer's per-layer snapshots (`Backend::release_scratch`).
Eviction is demand-driven, never because the session is merely over budget, so a session sitting at its budget does not
repaint in a loop. A request that does not fit even then is refused with `wl_display.error.no_memory`, which ends that
client's connection (the compositor and every other client go on) and is logged with the client's pid.
Never evicted: the output target and the staging a frame in flight reads (they are not in `releasable`), a capture frame
that an open session or a pending screencopy frame reads, and any View picture while a lock is up. A capture frame that was
given back is made again by the next session or screencopy frame, or when the budget has room for it again; the first copy
asked right after an eviction can fail (`failed`, `unknown`) and the next is served.

**Targets and captures.** The 8192 edge limit stays; a mode or factor whose target would pass the budget is refused by name
("memory: ...") after caches are given back, and the output keeps what it had. A capture is never refused by the budget: its
frame cache is refilled for a reader even over the budget, and the client's own buffers are what is refused.

**Published** (every screen node, the same value on each, since the tree has no session node): `gpu_memory_clients_bytes`,
`gpu_memory_budget_bytes`, `gpu_memory_refused_total`, `gpu_memory_evicted_total`; `Output.gpuMemory` gains `clients`,
`budget`, `refused`, `evicted`. They are announced only when they change.

**The DRM backend.** `Drm::footprint(output)` reports what the output's `Buffers` hold: its GLES renderer's footprint (the
composed target, blur scratch, View pictures) plus the scan-out slots allocated so far, `slots * width * height * 4` bytes
counted as `target` because the kernel scans one of them out and the budget must never take it back. The probe context
the device keeps for reading View pictures and importing clients' GPU buffers serves every output and belongs to none, so
it is added once, to the first output. `release_views` and `release_scratch` reach each output's renderer and the probe, as
they do on the headless backend, so what the ledger counts as reclaimable on DRM can be reclaimed. Tested on fakes only:
the slot ring's count (a failed allocation holds nothing, a second ensure makes no second slot), the byte arithmetic
(`scanout_bytes`, saturating) and `Footprint::plus`. Nothing here has a DRM node, so the real GBM allocation, the
renderer's footprint on a scan-out context and the comparison with `drm-memory-*` are unmeasured.

**GPU buffer size.** The ledger counts a dmabuf as `width * height * bits_per_pixel / 8`, with the bits from Smithay's
format table (`get_bpp`; 32 for a format it does not know). The driver's tiling, row padding, compression metadata and the
extra planes of a multi-planar format are not seen, so it is a floor. It is taken where the loop imports the buffer, after
the budget is asked, and recorded only if the importer accepted it: a buffer the importer or the budget refuses is never
in the account, so giving it up gives back nothing. `tests/dmabuf.rs` shows a refused `create` and a refused `create_immed`
leaving another client's bytes and the same client's earlier buffers exactly as they were.

**A client that was ended.** `post_error` marks the connection dead; the objects of a dead client are destroyed (and so given
back to the ledger) when the display next cleans up, which wayland-backend does when a client's connection has an event to
dispatch, so when the client closes it. A client that has heard `no_memory` exits and closes. One that holds the connection
open and silent keeps what it already held counted until some client's event runs the cleanup, which is the memory it
still holds; its refused request was never counted.

**Not covered.** The driver's own memory, a driver-reported number to check the estimates against, the `wl_shm_pool`
objects themselves (each holds a descriptor and a sparse mapping, so a client that makes thousands of pools is bounded by
the descriptor limit, not by the budget), and a client that holds nothing (an idle connection), which costs nothing here.
