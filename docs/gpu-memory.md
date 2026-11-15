# GPU memory

The compositor publishes, for each output, how many bytes its renderer and its capture path hold. It also holds the
session to a memory budget: what one client may hold, and what everything together may hold.

The numbers are read-only. They exist so that a process whose memory grows can be attributed to an allocator instead of
guessed at. The budget exists so that memory cannot grow without bound.

## Reading it

There are three ways to read it:

- In a script, `gnoblin.outputs.at(0).gpuMemory` is
  `{ target, scratch, staging, views, capture, total, clients, budget, refused, evicted }`, in bytes except `refused`.
- From a shell, `gnoblinctl output get ID gpu_memory_bytes` prints the total.
- On the tree, the properties are `gpu_memory_target_bytes`, `gpu_memory_scratch_bytes`, `gpu_memory_staging_bytes`,
  `gpu_memory_views_bytes`, `gpu_memory_capture_bytes` and `gpu_memory_bytes`, and for the session as a whole
  `gpu_memory_clients_bytes`, `gpu_memory_budget_bytes`, `gpu_memory_refused_total` and `gpu_memory_evicted_total`.

They are published at most once a second, the first a second after the session starts, and announced only when a value changes.

| Field     | What holds the bytes                                                                                                  |
| --------- | --------------------------------------------------------------------------------------------------------------------- |
| `target`  | The output-sized colour target with its depth buffer, and the copy a supersampled output is averaged down into.       |
| `scratch` | The blur scratch targets and the pixels kept under the cursor.                                                        |
| `staging` | Host-visible upload and readback buffers (Vulkan), or the snapshot of each client buffer the software renderer draws. |
| `views`   | The pictures of the Views drawn on the GPU.                                                                           |
| `capture` | The last presented frame kept for screencopy, image copy and screen cast.                                             |
| `total`   | The sum of the five above, for this output.                                                                           |

The last four fields are about the whole session, so they are the same on every output; read them from the first.

| Field     | What it says                                                                                            |
| --------- | ------------------------------------------------------------------------------------------------------- |
| `clients` | The bytes of every client's live buffers, as the budget counts them (see below).                        |
| `budget`  | The budget, in bytes; 0 when it is off.                                                                 |
| `refused` | The requests refused for passing a bound since the session began: buffers, surfaces and output changes. |
| `evicted` | The bytes given back from caches to make room for a client's buffer since the session began.            |

## What the numbers mean

The first six are the sizes the compositor asked the driver for: width times height times the bytes per pixel of the
format. A driver that pads or tiles holds at least this much.

What each renderer counts:

- The software and GLES renderers (the headless backend): their targets, blur scratch, snapshots and View pictures.
- The DRM backend: the same for each output's GLES renderer, plus the scan-out buffers it allocated for the output (two
  slots of the mode's size, four bytes a pixel, counted as `target` because the kernel is scanning one of them out), and
  the renderer it keeps for reading View pictures and importing clients' GPU buffers, which serves every output and is
  counted once, on the first.

Not counted:

- The driver's private memory, and the padding or tiling a driver adds.
- A client's own memory: a shared-memory pool is a mapping the client controls, and a GPU buffer lives in the client's
  allocation.

On a real GPU, compare the total with the per-process `drm-memory-*` counters of `/proc/PID/fdinfo`
(`scripts/gnoblin-incident-resources.py` reads them). A gap is memory this page does not count. The DRM numbers have not
been compared with those counters yet: the machines this was built and tested on have no DRM device.

## The budget

Three bounds, each a setting of the `compositor` section, in mebibytes except the surface count. A value of 0 turns that
bound off. They are read when the compositor starts.

| Setting                              | Default | What it bounds                                                                                 |
| ------------------------------------ | ------- | ---------------------------------------------------------------------------------------------- |
| `compositor.gpu-memory-budget-mib`   | 6144    | Everything: the clients' live buffers, what every renderer holds and what capture keeps.       |
| `compositor.client-memory-limit-mib` | 2048    | The live buffers of one client.                                                                |
| `compositor.client-surface-limit`    | 4096    | The live surfaces of one client (toplevels, popups, subsurfaces and cursors are all surfaces). |

What a client is counted for:

- A shared-memory buffer is `stride * height`, the part of its pool the buffer exposes. The pool is not counted: it is a
  sparse mapping the client controls, and a texture made from a buffer is the same pixels.
- A GPU (dmabuf) buffer is `width * height` times the bits per pixel of its format, divided by eight. This is an
  estimate, because the compositor cannot ask the driver what the buffer took. Padding and tiling add to it, a
  multi-planar format is counted at its average bits per pixel, and an unknown format counts four bytes a pixel.
- A GPU buffer is counted when the loop imports it, not when the client creates it. A buffer the importer refuses is
  never counted and gives nothing back.
- A buffer counts until the client destroys it or disconnects. A client that makes and destroys a buffer every frame
  holds one at a time.

The defaults come from the incident this was built for, a session that reached 31 GiB of GPU memory and ran out. The
budget is a fifth of that and above what a desktop of several 4K clients holds. A client may use a third of it, so one
client cannot take the session and two cannot either.

### What a client that passes a bound sees

The compositor ends that client's connection with `wl_display.error.no_memory` (code 2), and says why in the message, for
example `the client holds 8388608 bytes of buffers and a buffer of 1048576 more passes its limit of 8388608`. A toolkit
reports this as "out of memory" or "Protocol error 2 on wl_display". The compositor and every other client go on. The
compositor logs the refusal (`area = "memory"`, with the client's pid) at warn level.

The ended client's buffers and surfaces stay counted until its connection is closed, because that is when its objects are
destroyed; a client that has heard the error exits and closes it. A client that holds the connection open without sending
anything keeps what it already had counted, which is what it still holds.

Before it refuses a buffer for the budget, the compositor gives back what it can make again, in this order and only as far
as the buffer needs:

1. The frame capture keeps for screencopy and screen cast, while no capture session and no screencopy frame reads it. A
   session or frame opened afterwards makes it again; the first copy asked right after an eviction can fail with `failed`
   (`unknown` for image copy capture), and the next is served.
2. The pictures of the Views drawn on the GPU. They are drawn again from the scene. None are given back while the session
   is locked.
3. The blur scratch targets and the software renderer's per-layer snapshots, which the next frame makes again.

The output target, what a frame in flight reads and what an open capture reads are never given back. A mode or supersample
change whose target would pass the budget is refused the same way (`memory: ...`) and the output keeps what it had.

### Reading the telemetry

- `refused` rising means clients are being ended or output changes refused: look for the `no_memory` warning in the log,
  which names the client's pid.
- `evicted` rising with `refused` at 0 means the budget is doing its job: caches were given back for clients that fit.
- `clients` near `budget` means the clients are the memory. Compare it with the per-output numbers: if `total` is large
  too, the renderers and capture are.
- `clients` far below the process's resident set means the growth is not in what the budget counts. Compare with
  `drm-memory-*` and the heap.

## Other bounds the compositor enforces

- A buffer of more than 16384 pixels on an edge or 2^27 pixels is refused when the client creates it
  (`wl_shm.invalid_stride`, `zwp_linux_buffer_params_v1.invalid_dimensions`).
- At most 8 screenshot requests wait for the loop; a request that gives up withdraws itself.
- The Vulkan renderer gives back its staging and readback buffers after each frame down to what one steady frame needs.
- The GLES renderer deletes the textures it imported for a frame when the frame is drawn.
- The frame capture keeps is released when the output goes.
- At most 256 windows are managed at once. A window past that is left unmanaged. That is the session's limit; the
  per-client surface limit above is what stops one client from making them without end.

## Known limit

On a machine whose only Vulkan device is the software driver (lavapipe, Mesa 22.3.6), the Vulkan renderer makes the
process grow by about 0.8 MiB a frame. The driver does this, and the numbers above do not show it.

Do not choose the Vulkan renderer there for a long session. A real GPU's driver has not been measured. The measurements are
in `tests/gpu-memory-soak/`.
