# gnoblin-xr

`gnoblin-xr` is the XR extension for Gnoblin. It is an out-of-process program on the owner socket. It reads the `View`
nodes under the node named `room` and submits them as OpenXR layers. It is purely translational: it does not decide what
is drawn or where it sits. The design is in design/xr-extension.md (D34) and design/node-api-refactor.md, section 9.

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
- **`xr-session`** (library): the OpenXR session and frame loop. `Runtime` is the trait the loop drives (`poll_event`, `begin_session`, `end_session`, `wait_frame`, `begin_frame`, `end_frame`, `locate_head`). `FrameLoop` is the state machine over it: it begins the session on `Ready`, ends it on `Stopping`, runs one paired `wait_frame`, `begin_frame`, `locate_head` and `end_frame` per frame only while the session is running, and returns cleanly on `Exiting`. The loop is tested with a fake runtime. `openxr_runtime` is the one OpenXR-backed `Runtime`: it loads the loader, creates a headless (`XR_MND_headless`) instance and session, and is the only `unsafe` code in the extension. Its one test is `#[ignore]` and runs only with `GNOBLIN_XR_MONADO=1`. See `crates/xr-session/README.md`.
- **`gnoblin-xr`** (binary): a placeholder. It parses `--help` and `--version` and nothing else.

## What does not exist

- No swapchains and no layer submission: the frame loop passes empty layer lists to `end_frame`.
- No owner-socket client. The binary does not connect to Gnoblin, and `xr-session` is not wired to it yet.
- No rendering, no GPU code, no input mapping, no environment.
- The `[powers.process]` entry that starts this program is not written.

## Build and test

The extension is its own workspace, so build from this directory:

```sh
cd extensions/gnoblin-xr
CARGO_TARGET_DIR=/home/kieran/.cache/gnoblin-target-xr cargo test --workspace
```

The toolchain comes from the repository root's `rust-toolchain.toml`.
