# PipeWire capture spike

Status: the round trip is proven. A producer stream delivers five BGRx 64x48 frames to a consumer through a private daemon, and the test checks each frame's bytes.

## Environment

- Agent37 instance (Debian 12), libpipewire `0.3.65`, `pipewire` daemon starts and accepts clients.
- Rust binding: `pipewire` crate `0.10.1`, which wraps the system libpipewire dynamically. Acceptable for a dev-dependency only; the shipped binary would need the same libpipewire at run time, so it needs a packaging decision before a real producer lands.
- No session manager, no D-Bus session bus, no rtkit. The daemon logs these as warnings and runs anyway.

## Working recipe

- Daemon: private `pipewire` in a temp runtime dir; its socket appears and one client connects. No session manager needed.
- Producer: `StreamRc` output on `Video/Source` with `node.want-driver = true`; connect `Direction::Output` with `AUTOCONNECT | MAP_BUFFERS | DRIVER`.
- `DRIVER` makes the producer its own driver, so nothing else calls its process callback. Arm `Loop::add_timer` to call `trigger_process()` every 33 ms (30 fps). `trigger_process` needs the `pipewire` dev-dependency feature `v0_3_34`.
- Producer params: EnumFormat (BGRx, 64x48, 30 fps) and `SPA_PARAM_BUFFERS` (see below).
- Consumer: `StreamBox` input connected with `Some(producer node id)` and the same format.
- Link: create it with `link-factory` from the producer output port to the consumer input port, using port ids from the registry.
- Result: both streams reach `Streaming`; five BGRx 64x48 frames arrive with the expected blue value.

Failures met on the way: no timer means `process` never fires; no buffers param means the consumer maps zero bytes; a `StreamBox` dropped while the test holds its sink borrow aborts on the `state_changed` callback.

## Buffers and open items

- Buffer layout: the producer MUST offer `SPA_PARAM_BUFFERS` (4 buffers, 1 block, size = one frame, stride, align 16, dataType `1 << MemFd`). Without it the daemon has no buffers and the consumer maps a zero-sized plane.
- DMA-BUF: not possible on this instance (no GPU or DRM device). Shared memory (MemFd/MemPtr) is the only option to test here.

## Where the real producer belongs

`gnoblin-capture` is the foundation crate for capture policy and holds no mechanism; its `CaptureSource` trait is the seam. The PipeWire producer is mechanism, so it belongs in an engine-side crate that already owns the compositor's capture source, not in `gnoblin-capture`. The xtask deps rules ban `smithay*` and `wayland-*` externals in foundation crates and do not ban `pipewire`, and they exempt dev-dependencies from the engine edge.

## Next steps

1. Build the real producer in an engine-side crate, following the recipe above.
2. Decide how libpipewire is linked for shipped binaries (see Environment).
