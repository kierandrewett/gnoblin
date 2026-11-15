# GPU and large-host memory: the inventory, the budget and the telemetry

Lane GpuMemory (gnoblin-4z5, GitHub #170). The legacy Mutter session grew to about
31 GiB of `GPUActive` (TTM pages) with an RSS under 1 GiB and the owner was never
attributed. This lane's question: can the Rust compositor grow GPU or host memory
without bound, and can every large allocation be attributed when it happens?

Status: inventory complete and read from source on 2026-11-15 (commit `3d714c1dd`).
The accounting layer, the leak tests and the soak are designed below and **not yet
built**; the Agent37 box pool was at its instance limit for this session, so no
build or test has run on the box. Nothing here claims the legacy 31 GiB mystery is
resolved — that was Mutter's session, not this code.

## 1. Every GPU or large-host allocation site

Sizes are the formula each site grows by. "Capped" names the thing that already
bounds it; "cap proposed" names what lane GpuMemory adds. [INFERENCE] marks a
value or lifetime not verified from source this session.

| # | Site | Owner | Key / lifetime | Size | Cap today | Eviction |
| - | ---- | ----- | -------------- | ---- | --------- | -------- |
| 1 | Client shm import (`Gles::import` → Smithay `import_shm_buffer`, `crates/gnoblin-render-gles/src/lib.rs`) | Smithay's buffer cache, per `WlBuffer` | the buffer object; released when the buffer is destroyed | `width*height*4` per buffer, plus driver storage | none beyond driver limits | buffer drop |
| 2 | Client dmabuf import (`dmabuf_texture` → Smithay `import_dmabuf`) | Smithay's cache on the buffer | per buffer, kept across frames | the dmabuf's own size (client-allocated) | driver import check only | buffer drop |
| 3 | Output target (`GlContext::target`, one per renderer) | `Gles.target` | per output; replaced by `resize`, old released (`Gles::resize`) | `mode*s` (supersample) *4 B, plus depth renderbuffer | `GL_MAX_TEXTURE_SIZE` and `MAX_VIRTUAL_EDGE` 8192 (`gnoblin-output/src/supersample.rs`) | old target released on resize |
| 4 | Blur scratch pair (`Gles::ensure_scratch`, two targets the output's size) | `Gles.scratch` | per output, rebuilt after resize | `2 * mode*s *4` | as site 3 | released on resize |
| 5 | Downsample target `shown` (`Gles::downsampled`) | `Gles.shown` | per output, kept across captures, replaced on size change | mode *4 B | as site 3 | replaced on size change |
| 6 | View subtree targets (`GlContext::subtree_target`, `subtree::Subtrees.slots`) | `Gles.subtrees` | key `(view_id, capture)`; retired when the view leaves `live_views` (`retire_views`) | `RGBA8` + `STENCIL_INDEX8` per View, ≤ `MAX_PIXELS` 33,554,432 px (`view_scene::size_fits`) | the View cap, not a byte cap | `release` on retire |
| 7 | Software renderer buffer + tint scratch (`Software.buffer`, `scratch`) | `Software` | per output; replaced by `resize` | mode *4 B, scratch up to one layer copy | host memory only | replaced on resize |
| 8 | Vulkan target (`RenderTarget`, `VkImage` device-local) | `Vulkan.target` | per output; replaced by `resize`, Drop destroys | mode *4 B | `maxImageDimension2D` | Drop on replace |
| 9 | Vulkan per-frame staging / uniforms / instances / readback (`FrameRecorder::ensure`) | `Vulkan.recorder` | reused, grown to `next_power_of_two` of the need, never shrunk | up to one frame's layer bytes (host-visible) + readback copy | grows to largest frame seen; **no shrink and no absolute cap** | none (stays at high-water mark) [INFERENCE: high-water only, by `ensure`] |
| 10 | DRM scan-out slots (`scanout::Buffers.slots`, `SLOTS = 2` GBM buffers + one GLES target `composed`) | `Drm.buffers` map per output | per output; dropped on output removal, mode change, resume (`buffers.remove`) | `2 * mode*4` + `mode*s *4` | `max_composed_edge` (`GL_MAX_TEXTURE_SIZE` ∩ `MAX_VIRTUAL_EDGE`) | whole `Buffers` dropped on modeset |
| 11 | Capture frame cache (`CaptureTables.pixels: BTreeMap<OutputId, Cached>`) | `State.capture` | per output, overwritten every presented frame, kept while any capture session is open | up to 2 *Image*s (plain + cursored) of `mode*s` *4 B per output | number of outputs; **no session-count pressure and no byte cap** | overwritten per frame; never freed while a session is open |
| 12 | View readbacks (`CaptureTables.view_readbacks`) | `State.capture` | key `(NodeId, revision)`; dropped when the scene revision changes or the view goes (`sync_view_readbacks`) | one `Image` per View on the GPU path | View count and the View pixel cap | revision change / view gone |
| 13 | View pictures on the CPU (`Composer.views` → `ViewPicture.image/redacted`) | `Composer.views` | per live View; retained against `view_nodes` every frame | up to two `Image`s per View (display + redacted), ≤ `MAX_PIXELS` | View pixel cap | `views.retain` |
| 14 | Canvas pictures (`Composer.canvases`) | `Composer.canvases` | per canvas node, replaced on content/size/scale change, retained against canvas nodes | size*scale *4 B | none beyond node count [INFERENCE] | `canvases.retain` |
| 15 | Window close snapshots (`WindowNode.closing` → `SurfaceSource::Snapshot`) | `Composer.windows` | one per closing window; lives for the 200 ms fade (`MOTION_MS`), freed by `finish_fade` | one `Image` copy of the buffer | fade duration | end of fade |
| 16 | Screenshot requests (`frame::Screenshots.queue` of `Shot`, `Image` served into `ShotSlot`) | `State.screenshots` | queued per control call; served once, waiter takes the image | one `Image` per pending shot | queue unbounded **[INFERENCE: the queue is a `VecDeque` with no cap found]** | served/timed out |
| 17 | PipeWire stream buffers (`PipewireExportStream.buffers`, `BUFFER_COUNT = 4` daemon-side planes) | the daemon's stream | per session; dropped with the stream (`reap`) | `4 * format bytes` in the daemon, plane written through the mapped buffer | session count; stream format | `reap` on session end |
| 18 | Dmabuf-capture pending queue (`DmabufCapture.pending: Vec<Pending>`) | `State.capture.dmabuf` | queued per request, answered next turn (`apply_dmabuf_captures`) or cancelled (`abort_image_copy`) | plane descriptors only, not pixels | loop drains each turn | applied/aborted |
| 19 | Cursor cache (`gnoblin-cursor/src/chain.rs::Cache`) | `CursorChain` | LRU of rasterised cursor images | sum of image bytes | `CURSOR_CACHE_BYTES` = 8 MiB, LRU evict | byte-cap LRU |
| 20 | Smithay's own surface state, popups, `UserDataMap` GPU handles | Smithay | per live surface | driver-dependent | none of ours [INFERENCE] | with the surface |

### What is already bounded

Targets and per-output intermediates (sites 3–8, 10) are all bounded by the
8192-edge virtual-surface limit and the driver's texture limit, and every one is
released on resize or replacement — an output that toggles supersample s=1/2/4
does not accumulate targets (`Gles::resize` releases the old target, scratch and
`shown` are size-matched and replaced). View textures (6) are retired with the
View. The closing snapshot (15) lives 200 ms. The cursor cache (19) has an
8 MiB LRU cap.

### Where growth is not yet attributed or capped

- **Site 11, the per-output capture cache**, holds up to two full-size `Image`s
  per output for as long as any capture session is open, with no byte accounting
  at all. It is the closest analogue of the legacy `GPUActive` growth: a client
  that opens sessions and never releases keeps the compositor's frame copies
  alive, and nothing in the log says who.
- **Site 9, the Vulkan recorder's host-visible buffers**, ratchets to the largest
  frame ever drawn and never shrinks or reports.
- **Site 16, the screenshot queue**, has no cap found this session [INFERENCE].
- **Sites 1–2** are the big real-GPU ones and are attributed to buffers only
  inside Smithay; the compositor's own accounting must sum them per client.

## 2. The accounting layer (designed; not built)

One module, `crates/gnoblin-backend/src/render/memory.rs`, in the engine set so
`gnoblin-wayland`, the backends and the renderers can all name it.

- **Categories** mirror the table: `client_shm`, `client_dmabuf`, `output_target`,
  `effect_scratch`, `view_subtree`, `capture_cache`, `staging`, `scanout`,
  `snapshot`, `cursor`, `other`. One `u64` counter per category, per output and
  per client where the site has one.
- **Allocation hook sites**: `GlContext::target`/`subtree_target`/`release`
  (sites 3–6), `Software::new`/`resize` (7), `VkImage`/`VkBuffer` create-destroy
  (8–9), `scanout::Buffers` allocate/drop (10), `CaptureTables::pixels` insert and
  `sync_view_readbacks` (11–12), `Composer::snapshot`/`finish_fade` (15), the
  Smithay import paths behind sites 1–2 via the renderer wrappers.
- **Budget**: a `GPU_BUDGET_BYTES` constant (default 2 GiB GPU-visible total) and
  a per-client cap (default 512 MiB). A `reserve(category, bytes, key)` that
  would exceed a budget fails the allocation with a log line naming the category,
  the owner and both numbers — the renderer paths already degrade one layer or
  one target per frame, so refusal is survivable everywhere the sites live.
- **Telemetry**: published as read-only tree properties on the `screen` node
  (`gpu_memory_output_bytes`, `gpu_memory_total_bytes`, …, spec-style camelCase
  `gpuMemory*` in JS, marked beyond-spec in `docs/shell-api/outputs.md`, schema in
  `crates/gnoblin-schema/schema/tree/screen.toml`, golden d.ts regenerated), a
  `gnoblinctl status` section with per-category bytes, and per-DRM-client
  `fdinfo` (`drm-memory-*` from `/proc/<pid>/fdinfo`, read-only, where the kernel
  exposes it) attributed to the client the connection belongs to.

## 3. The proof plan (designed; not run)

Leak tests first, on software, GLES llvmpipe and Vulkan lavapipe (a37 box;
`a37-gate.sh run/in`, scripts uncommitted under the worktree):

a. 500 open/close windows of different sizes, shm and dmabuf: counters return to
   baseline. b. resize storms. c. capture/screencopy/image-copy sessions created
   and dropped, including a client that never releases and one that disconnects
   mid-capture (the suspects are sites 9, 11, 16). d. supersample 1/2/4 toggled
   100 times on a live output; mode changes; hotplug. e. View create/destroy.
   f. blur/shadow on/off. g. lock screen on/off. h. a 16384×16384 buffer refused
   with a protocol error and no allocation; thousands of tiny surfaces against
   the per-client cap.

Then a 20-minute soak on the box with the stock JS shell and a churning client,
recording RSS, counters and texture counts at intervals, stored in `tests/` with
command lines and exit codes. Known limits to state in the report: llvmpipe and
gVisor, not a real GPU; the real-GPU dmabuf check is tracked in gnoblin-lubw.

## 4. What the legacy issue still needs

The 31 GiB `GPUActive` of 2026-10-11 was the Mutter session's; nothing here
attributes it. What this lane gives the legacy investigation is the *method*:
per-category counters plus per-client fdinfo, which would have named the owner in
minutes. Mutter's own accounting is out of scope for this lane.
