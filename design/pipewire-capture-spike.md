# PipeWire capture spike

Status: the round trip does not yet deliver frames. The producer's process callback is never invoked, so no frame reaches the consumer. The spike is not proven.

## Environment

- Agent37 instance (Debian 12), libpipewire `0.3.65`, `pipewire` daemon starts and accepts clients.
- Rust binding: `pipewire` crate `0.10.1`, which wraps the system libpipewire dynamically. Acceptable for a dev-dependency only; the shipped binary would need the same libpipewire at run time, so it needs a packaging decision before a real producer lands.
- No session manager, no D-Bus session bus, no rtkit. The daemon logs these as warnings and runs anyway.

## What works

- Private daemon starts in a temp runtime dir and its socket appears.
- A client connects over the socket and creates an output stream (`Video/Source`) and an input stream.
- Without `StreamFlags::DRIVER` on the producer, streams stay `Paused` forever.
- With `StreamFlags::DRIVER` plus a `link-factory` link from producer output port to consumer input port, both streams reach `Streaming`.

## What does not work

- After both reach `Streaming`, the producer's process callback is never called (counted: 0 calls, 0 buffers, no errors). The consumer therefore receives no frames and reports no error.
- Tried and did not change the result: `node.always-process = true`; `node.want-driver = true` (added as a literal, the `pipewire::keys` constant is not exported in 0.10.1).
- Not tried: a Dummy-Driver or consumer-as-driver configuration, `pw-dump` of the graph while the test runs, and a `pw-cat` baseline to separate binding faults from daemon scheduling.

## Not yet checked

- Format negotiation: no extra buffer params or modifiers were needed to reach `Streaming`, but no frame was ever filled, so buffer layout is unproven.
- DMA-BUF: not possible on this instance (no GPU or DRM device). Shared memory (MemFd/MemPtr) is the only option to test here.

## Where the real producer belongs

`gnoblin-capture` is the foundation crate for capture policy and holds no mechanism; its `CaptureSource` trait is the seam. The PipeWire producer is mechanism, so it belongs in an engine-side crate that already owns the compositor's capture source, not in `gnoblin-capture`. The xtask deps rules ban `smithay*` and `wayland-*` externals in foundation crates and do not ban `pipewire`, and they exempt dev-dependencies from the engine edge.

## Next steps

1. Get frames flowing: capture `pw-dump` during the run, or run `pw-cat --record` against the producer as an independent consumer, to see whether the graph schedules the producer.
2. Once frames flow, check BGRx 64x48 size, stride and pixel values at the consumer.
