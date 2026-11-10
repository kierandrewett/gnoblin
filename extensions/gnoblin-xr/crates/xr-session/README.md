# xr-session

The OpenXR session and frame loop for `gnoblin-xr`. It is a library with no socket code.

## Modules

- `runtime`: the `Runtime` trait and the session types. `SessionState` mirrors `XrSessionState`, `SessionEvent` is the
  state-change event, `FrameTiming` is what `wait_frame` returns, and `RuntimeError` carries the runtime's own message.
- `frame_loop`: `FrameLoop` is the state machine over a `Runtime`. `tick` drains the pending events, then runs one frame
  if the session is running. `run` calls `tick` until the session exits, sleeping when there is no frame to render.
    - `begin_session` is called once, on `Ready`.
    - Frames run in `Synchronized`, `Visible` and `Focused`. Each frame is `wait_frame`, `begin_frame`, `locate_head` and
      `end_frame`, in that order. `should_render: false` frames still call `begin_frame` and `end_frame`.
    - `end_session` is called once, on `Stopping`.
    - `Exiting` ends the loop. `LossPending` stops frames.
    - Runtime errors propagate to the caller.
- `openxr_runtime`: `OpenxrRuntime`, the OpenXR-backed `Runtime`. It loads the loader with `Entry::load`, checks for
  `XR_MND_headless`, creates the instance and a headless session, and reports `Ready` through `poll_event`. It is the only
  module with `unsafe` code. The unsafe calls are the loader load and `xrCreateSession`, and each has a `SAFETY` comment.
- `testing` (test only): a `FakeRuntime` that records the calls the loop makes.

## Scope

- The frame loop submits empty layer lists. There are no swapchains and no graphics binding.
- The session is headless. It is not wired to the Gnoblin owner socket.
- The OpenXR runtime's test is `#[ignore]`. It runs only with `GNOBLIN_XR_MONADO=1 cargo test -- --ignored`, against an
  installed Monado runtime that offers `XR_MND_headless`.

## Build and test

Run from `extensions/gnoblin-xr`:

```sh
cargo test -p xr-session
```
