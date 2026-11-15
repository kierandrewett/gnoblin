# GPU memory

The compositor publishes, for each output, how many bytes its renderer and its capture path hold. It is an
owner-requested addition beyond the display specification, read-only, and it exists so that a process whose memory
grows can be attributed to an allocator instead of guessed at.

## Reading it

There are three ways to read it:

- In a script, `gnoblin.outputs.at(0).gpuMemory` is `{ target, scratch, staging, views, capture, total }`, in bytes.
- From a shell, `gnoblinctl output get ID gpu_memory_bytes` prints the total.
- On the tree, the properties are `gpu_memory_target_bytes`, `gpu_memory_scratch_bytes`, `gpu_memory_staging_bytes`,
  `gpu_memory_views_bytes`, `gpu_memory_capture_bytes` and `gpu_memory_bytes`.

They are published at most once a second, the first a second after the session starts, and announced only when a value changes.

| Field     | What holds the bytes                                                                                                  |
| --------- | --------------------------------------------------------------------------------------------------------------------- |
| `target`  | The output-sized colour target with its depth buffer, and the copy a supersampled output is averaged down into.       |
| `scratch` | The blur scratch targets and the pixels kept under the cursor.                                                        |
| `staging` | Host-visible upload and readback buffers (Vulkan), or the snapshot of each client buffer the software renderer draws. |
| `views`   | The pictures of the Views drawn on the GPU.                                                                           |
| `capture` | The last presented frame kept for screencopy, image copy and screen cast.                                             |
| `total`   | The sum.                                                                                                              |

## What the numbers mean

They are the sizes the compositor asked the driver for: width times height times the bytes per pixel of the format. A
driver that pads or tiles holds at least this much.

Not counted:

- A client's own buffers, which Smithay owns.
- The scan-out buffers of a DRM output.
- The driver's private memory.

On a real GPU, compare the total with the per-process `drm-memory-*` counters of `/proc/PID/fdinfo`
(`scripts/gnoblin-incident-resources.py` reads them). A gap is memory this page does not count.

## Bounds the compositor enforces

- A buffer of more than 16384 pixels on an edge or 2^27 pixels is refused when the client creates it
  (`wl_shm.invalid_stride`, `zwp_linux_buffer_params_v1.invalid_dimensions`).
- At most 8 screenshot requests wait for the loop; a request that gives up withdraws itself.
- The Vulkan renderer gives back its staging and readback buffers after each frame down to what one steady frame needs.
- The GLES renderer deletes the textures it imported for a frame when the frame is drawn.
- The frame capture keeps for an output is released when the output goes.
- At most 256 windows are managed at once. A window past that is left unmanaged, so a client that opens toplevels in a
  loop cannot make the compositor keep more; the limit is the session's, not one client's.

## Known limit

On a machine whose only Vulkan device is the software driver (lavapipe, Mesa 22.3.6), the Vulkan renderer makes the
process grow by about 0.8 MiB a frame. The driver does this, and the numbers above do not show it.

Do not choose the Vulkan renderer there for a long session. A real GPU's driver has not been measured. The measurements are
in `design/gpu-memory.md`.
