# gnoblin-xr

`gnoblin-xr` is the XR extension for Gnoblin. It is an out-of-process program on the owner socket. It attaches to the tree,
reads the `View` nodes under the node named `room`, and plans them into OpenXR layers on every frame. It is purely
translational: it does not decide what is drawn or where it sits. It does not yet submit those layers, so the session
runs with empty layer lists. The design is in design/xr-extension.md (D34) and design/node-api-refactor.md, section 9.

## What exists

- **`xr-runtime`** (library)
    - `lookup`: finds the active OpenXR runtime manifest. The order is `$XR_RUNTIME_JSON` (exclusive when set),
      `$XDG_CONFIG_HOME/openxr/1/active_runtime.json`, each directory in `$XDG_CONFIG_DIRS`, then
      `/etc/xdg/openxr/1/active_runtime.json`. Each directory is searched for the architecture file
      `active_runtime.<arch>.json` first. The rules follow the Khronos OpenXR SDK loader specification (`runtime.adoc`).
      The manifest must parse as JSON with `runtime.library_path`; a relative `library_path` is resolved against the
      manifest's directory. The result is `Runtime { name, library_path, source }`.
    - `service`: detects the runtime service binaries `monado-service` and `wivrn-server` on `PATH`.
    - `supervisor`: `RuntimeManager` starts, polls, stops and reports on a runtime service child. It restarts on failure, at
      most 3 times. It kills the child on drop. It works through the `Spawner` and `ChildProcess` traits, so tests use a fake
      child and no real process is started. `status` reports `no-runtime` when no manifest is found.
- **`xr-scene`** (library, no dependencies): the pure translation from the `room` subtree to layer placements, and the
  ray pick against them. `plan` walks the `room` Views in tree order and returns the `Quad` and `Projection` layers to
  submit, with the skipped Views and their reasons. `pick` tests a ray against the quads in reverse submission order and
  returns the View's pixel coordinates. It has no OpenXR or socket code, so its rules are tested without a headset. See
  `crates/xr-scene/README.md`.
- **`gnoblin-xr`** pointer (`src/pointer.rs`): each frame the runtime's pointing ray is cast against the planned quads with `xr_scene::pick`. A hit sends `motion` to the owner over the window the View shows, on a seat named `xr`: the bridge reuses a seat of that name from the owner snapshot and creates one on first use. A miss sends `leave` for the window last hovered.
- **`gnoblin-xr`** (binary): connects to the owner socket (`$GNOBLIN_IPC_SOCKET`, else `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`),
  attaches to the `tree` namespace, opens the headless OpenXR session and runs `bridge::run`. The bridge polls the owner
  socket and runs one frame-loop tick per pass. Each frame plans the `room` subtree against the located head pose and
  logs the layer and skipped counts when they change. It exits when the runtime asks the session to exit or the owner
  closes the socket. `--help` and `--version` print and exit without connecting.

## What does not exist

- No swapchains and no layer submission: the frame loop passes empty layer lists to `end_frame`, and the plan is only logged.
- No rendering, no GPU code, no environment. Buttons and scroll are not wired: the runtime has no button source yet.
- The `[powers.process]` entry that starts this program is not written.

## Build and test

The extension is its own workspace, so build from this directory:

```sh
cd extensions/gnoblin-xr
CARGO_TARGET_DIR=/home/kieran/.cache/gnoblin-target-xr cargo test --workspace
```

The toolchain comes from the repository root's `rust-toolchain.toml`.
