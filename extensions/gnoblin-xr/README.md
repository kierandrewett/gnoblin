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
- **`gnoblin-xr`** (binary): a placeholder. It parses `--help` and `--version` and nothing else.

## What does not exist

- No OpenXR dependency. No session, frame loop, swapchain or layer submission.
- No owner-socket client. The binary does not connect to Gnoblin.
- No rendering, no GPU code, no input mapping, no environment.
- The `[powers.process]` entry that starts this program is not written.

## Build and test

The extension is its own workspace, so build from this directory:

```sh
cd extensions/gnoblin-xr
CARGO_TARGET_DIR=/home/kieran/.cache/gnoblin-target-xr cargo test --workspace
```

The toolchain comes from the repository root's `rust-toolchain.toml`.
