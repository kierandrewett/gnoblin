# gnoblin-xr

`gnoblin-xr` is the XR extension for Gnoblin. It is an out-of-process program on the owner socket. It attaches to the tree,
reads the `View` nodes under the node named `room`, and plans them into OpenXR layers on every frame. It is purely
translational: it does not decide what is drawn or where it sits. It copies each View's picture into a swapchain and submits
the layers. The design is in design/xr-extension.md (D34) and design/archive/node-api-refactor.md, section 9.

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
  submit, with the skipped Views and their reasons. A View whose `camera` names a stereo camera is a `Projection`
  layer whose two eyes `rig::eyes` states from the camera's pose, `ipd`, `metres` and frustum (the same eyes the
  compositor draws, `gnoblin-render-software` `stereo.rs`); a camera that cannot be drawn skips the View as
  `InvalidCamera`. `pick` tests a ray against the quads in reverse submission order and
  returns the View's pixel coordinates. It has no OpenXR or socket code, so its rules are tested without a headset. See
  `crates/xr-scene/README.md`.
- **`xr-layers`** (library, no OpenXR types): `compose` turns the planned layers and the ready swapchain of each View into
  the layers to submit: a quad per flat View, a projection per stereo View (with the eyes the plan carries, left eye in
  the left half of the picture), in plan order. Opacity below 1 becomes a
  colour scale when `XR_KHR_composition_layer_color_scale_bias` is enabled. Layers beyond `maxLayerCount` are dropped by
  priority (the View that shows a focused window is kept first, then the nearest to the head) and reported `layer-budget`. See `crates/xr-layers/README.md`.
- **`xr-input`** (library, no OpenXR types): the controller input model. A `ControllerState` (the pointing ray, the
  select and secondary values, the scroll stick) goes in once per frame; `Controller::update` returns the ordered
  `PointerEvent`s: `BTN_LEFT` for select and `BTN_RIGHT` for secondary, pressed at 0.75 and released at 0.25, and scroll
  amounts past a 0.15 deadzone at up to 400 axis units a second. `pointing_ray` picks the aim pose, else the head's
  gaze.
- **`xr-gpu`** (library): the `Presenter` trait copies one captured picture into one acquired swapchain image. `format`
  chooses the swapchain format (`R8G8B8A8_SRGB` or `B8G8R8A8_SRGB`, the runtime's first choice) and converts `wl_shm`
  Argb8888 or Xrgb8888 into it. `VulkanPresenter` does the copy through `ash` on a device made by
  `XR_KHR_vulkan_enable2`; `FakePresenter` (feature `testing`) records uploads. See `crates/xr-gpu/README.md`.
- **`xr-session`**: `OpenxrRuntime::connect_vulkan` opens a session on that Vulkan device, with swapchains and layer
  submission. `OpenxrRuntime::connect` is still the headless session. A runtime without `XR_KHR_vulkan_enable2` is
  refused with a message that names the extension.
- **`xr-tree`** (library): the owner-socket tree client. `TreeClient::connect_recording` also writes every frame it
  receives to a file, and `replay` serves such a file back to a client one change batch at a time. The recording in
  `crates/xr-tree/tests/recorded/` is what the owner sent while the stock shell built a room, moved the focus, closed a
  window and tore the room down; `tests/recorded.rs` replays it and checks the planner against it as the head moves.
- **`gnoblin-xr`** `surfaces.rs`: one swapchain per planned View. It is made when the View first has a picture, remade when
  the picture's size changes, filled only when the capture's `serial` changes, and destroyed when the View leaves the plan
  or the bridge ends. A swapchain is submitted only after a picture is in it. A swapchain whose runtime calls fail (one
  the runtime lost) is destroyed and made again on the next pass.
- **`gnoblin-xr`** pointer (`src/pointer.rs`): each frame the runtime's controller is read (`Runtime::locate_controller`) and the pointing ray is cast against the planned quads with `xr_scene::pick`. A hit sends `motion` to the owner over the window the View shows, on a seat named `xr`: the bridge reuses a seat of that name from the owner snapshot and creates one on first use. A miss sends `leave` for the window last hovered, unless a button is held. Select is `BTN_LEFT`, secondary is `BTN_RIGHT` and the stick scrolls. A press grabs the window under the ray until the last button is up; the coordinates keep following the ray on the grabbed window's plane. The owner refusing an op is logged, not fatal.
- **`gnoblin-xr`** (binary): connects to the owner socket (`$GNOBLIN_IPC_SOCKET`, else `$XDG_RUNTIME_DIR/gnoblin/ipc.sock`),
  attaches to the `tree` namespace, opens a Vulkan OpenXR session and runs `bridge::run`. When the runtime cannot give a
  Vulkan session it prints why and runs a headless session, which plans and drives the pointer and shows nothing. The
  bridge polls the owner socket and runs one frame per pass. Each frame plans the `room` subtree against the located head
  pose, pumps the captures, copies changed pictures into swapchains, and ends the frame with the composed layers. It logs
  the layer and skipped counts, the dropped or degraded Views and the failures when they change. A session that cannot
  start, fails or ends (`supervise.rs`) is followed by a new one after a wait of 250 ms that doubles to 4 s, over the same
  owner connection; the `xr` seat the pointer made is destroyed when a session ends, so the shell takes the room down. It
  exits only when the owner closes the socket (status 0) or fails it (status 1). `--help` and `--version` print and exit
  without connecting.

## What does not exist

- Head tracking and the layer budget have run only against the stub runtime, a fake owner and the compositor's tree
  socket, never on a headset. `src/head.rs` writes the head pose onto the camera named `head` (at most 60 a second, one in
  flight, never a stale or non-finite pose) and pairs each picture with the eyes it was drawn from; it also writes the
  runtime's layer limit to that camera's `layer_budget`. `xr-layers` drops Views over the budget, the focused one last,
  then the farthest from the head. See `docs/xr.md`, "Head tracking" and "Layer budget".
- No DMA-BUF path: pictures are `wl_shm` bytes, copied through a staging buffer.
- No environment, no layers of the extension's own.
- Nothing here has run on a headset. The null compositor of Monado accepts the layers and draws nothing.

## Starting

The guardian starts this program when the user sets `GNOBLIN_XR=1`; see `docs/xr.md`. It hands the program a Wayland connection with the
`view` role as `WAYLAND_SOCKET` and restarts it with backoff. `GNOBLIN_XR_PROGRAM` names a build that is not installed.

## Build and test

The extension is its own workspace, so build from this directory:

```sh
cd extensions/gnoblin-xr
CARGO_TARGET_DIR=/home/kieran/.cache/gnoblin-target-xr cargo test --workspace
```

The toolchain comes from the repository root's `rust-toolchain.toml`.

### Against Monado

The unit tests use a fake runtime, a fake presenter and a fake owner, and run anywhere. Two ignored tests use a real
runtime: `monado_vulkan_session_holds_the_uploaded_pixels_and_accepts_a_quad` (`xr-session`) and
`monado_bridge_submits_a_view_end_to_end` (`gnoblin-xr`). They need a running `monado-service` with a Vulkan driver, for
example lavapipe:

```sh
# the service, with the simulated headset and the null compositor (XDG_RUNTIME_DIR must match the client's)
SIMULATED_ENABLE=1 XRT_COMPOSITOR_NULL=1 VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json monado-service &
# the tests
GNOBLIN_XR_MONADO=1 XR_RUNTIME_JSON=<monado build>/openxr_monado-dev.json \
    VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json cargo test -- --ignored --test-threads=1
```

`monado_headless_runs_one_empty_frame` fails on Monado `7336a90` with `XR_ERROR_CALL_ORDER_INVALID` at `xrBeginFrame`, and
failed the same way before the Vulkan session was added. The null compositor draws nothing, so these tests check what the
runtime accepts and what the swapchain image holds, not what a headset shows. See the "Verified on Agent37" part of
`design/xr-extension.md`.
