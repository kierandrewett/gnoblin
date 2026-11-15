# Backend parity audit: headless, nested and DRM

**Status:** audit plus the first fixes. Items marked [INFERENCE] were read from source and not run. The Backend trait is
`crates/gnoblin-backend/src/lib.rs`, headless is `src/headless.rs`, nested is `src/nested/`, DRM is
`crates/gnoblin-backend-drm/src/lib.rs`. The tables describe the state before this lane; the last table says what
landed.

## What "the same" means here

The compositor above the `Backend` trait must behave identically on every backend wherever the host protocol or the
hardware can carry the behaviour. What a backend physically cannot do is refused with the typed `BackendError`
vocabulary (`Unsupported`, `Unpresentable`, `ModesetDisabled`), never ignored and never faked. A nested session does
not emulate gamma, power or arbitrary modes: a wlr-gamma-control forward would tint the host's whole monitor, not the
window. Headless is a virtual display and records gamma and power as virtual hardware, which is the one deliberate
asymmetry; the conformance suite states it per backend.

## Findings that decide the work

1. The `gnoblin` binary never selects a backend. `SessionConfig::default()` is headless and no config leaf, flag or
   variable changes it (`crates/gnoblin/src/main.rs:159`, `role.rs:56`). Only tests choose nested.
2. `Backend::build` refuses every renderer but `auto`/`software` for nested (`compositor/src/backend.rs:156`) and refuses
   `Drm` outright with a static string (`backend.rs:166`). The compositor crate does not depend on `gnoblin-backend-drm`.
3. Nested keeps a `Software` field instead of `AnyRenderer` (`nested/surface.rs`), so GLES and Vulkan cannot be
   created there, and every GPU-facing trait method falls to its permissive default.
4. `BackendEvent::Disconnected` is documented as "the session must end" but the server loop only recomputes the layout
   (`wayland/src/server.rs:1703`): a nested session whose host died, or whose last window was closed, runs forever with
   no output.
5. The Drm struct reports `target() == None` and `capture() == None` (`drm/src/lib.rs:452,525`): it is scan-out only.

## Capability matrix (before this lane)

Cells: same / differs / missing / refused / n/a. DRM cells say "fake" where only a fake kernel proves them.

### Outputs

| Behaviour                                   | Headless                                              | Nested                                                                                        | DRM                                                  | Result                                    |
| ------------------------------------------- | ----------------------------------------------------- | --------------------------------------------------------------------------------------------- | ---------------------------------------------------- | ----------------------------------------- |
| Connectors, hotplug add/remove              | handle queue (`headless.rs:470`)                      | one toplevel per `OutputSpec` at connect only; close = remove                                 | udev `sync_hardware` (fake)                          | nested runtime hotplug missing            |
| Mode list (`output_modes`)                  | the one mode (default)                                | missing: default, the host window is the only mode                                            | kernel list (fake)                                   | document: refused modes are `Unsupported` |
| Re-mode (`configure_output`)                | any non-degenerate mode                               | refused `Unsupported` (host decides)                                                          | kernel test+commit, off unless `GNOBLIN_DRM_MODESET` | same vocabulary                           |
| Preferred mode, EDID identity               | none                                                  | none                                                                                          | EDID                                                 | n/a for virtual outputs                   |
| Scale, fractional scale                     | settable                                              | follows `wp_fractional_scale`; `wl_output.scale` of the first host output only, never updated | settable                                             | nested follow incomplete                  |
| Supersample                                 | yes                                                   | yes, shm only at mode size                                                                    | yes                                                  | same                                      |
| Transform, layout position, mirror, enabled | compositor-level (`gnoblin-output`), not in the trait | same                                                                                          | same                                                 | same (n/a to backends)                    |
| Refresh rate                                | 60 Hz fixed                                           | first host `wl_output` mode, else 60 Hz                                                       | kernel mode                                          | differs by nature                         |
| Host death / window close                   | n/a                                                   | `Disconnected` ignored by the server                                                          | `Disconnected` ignored by the server                 | finding 4                                 |

### Renderers and presentation

| Behaviour                                        | Headless                    | Nested                                       | DRM                                 | Result               |
| ------------------------------------------------ | --------------------------- | -------------------------------------------- | ----------------------------------- | -------------------- |
| Software                                         | yes                         | yes                                          | no                                  | n/a on DRM           |
| GLES                                             | yes (surfaceless EGL)       | refused by `build`                           | yes (GBM/EGL scan-out)              | fix (a)              |
| Vulkan                                           | yes                         | refused by `build`                           | no                                  | fix (a)              |
| Damage tracking                                  | renderer                    | renderer, forwarded as `damage_buffer`       | renderer + flip                     | same                 |
| Frame pacing                                     | immediate                   | host frame callback, 2 shm slots             | page flip                           | differs by nature    |
| Presentation feedback                            | compositor clock after draw | same, not the host's `presented` [INFERENCE] | same, not the flip time [INFERENCE] | same stamp source    |
| `effect_kinds`                                   | per renderer                | software only, hard-coded (`mod.rs:705`)     | GLES (`lib.rs:530`)                 | derive from renderer |
| `gpu_views`, `read_view`, `release_views`        | GLES only                   | missing                                      | GLES                                | fix (b)              |
| `dmabuf`, `import_dmabuf`, linux-dmabuf feedback | GLES only                   | missing                                      | GLES scan-out device                | fix (b)              |
| `copy_to_dmabuf`, `dmabuf_capture_support`       | GLES only                   | missing                                      | picture only, no frame              | fix (b)              |
| `footprint`                                      | per renderer                | missing: all zeros                           | default zeros [INFERENCE]           | fix (b)              |
| `max_virtual_edge`                               | renderer limit              | `MAX_EXTENT`, not the renderer's             | GPU limit                           | fix with (a)         |
| Direct scanout, HDR, colour management           | none                        | none                                         | none [INFERENCE]                    | missing everywhere   |

### Input

| Behaviour                               | Headless          | Nested                                                               | DRM                                         | Result                    |
| --------------------------------------- | ----------------- | -------------------------------------------------------------------- | ------------------------------------------- | ------------------------- |
| Event variants                          | all, injected     | pointer, button, axis, key, touch; no `Touchpad`, no relative motion | all but tablet                              | tablet missing everywhere |
| `DeviceAdded`/`Removed`, caps           | injected by tests | never sent: `Seat.device` is empty                                   | libinput devices with caps                  | fix (f)                   |
| Keyboard focus loss                     | n/a               | `wl_keyboard.leave` ignored: held keys stick                         | libinput                                    | fix (f)                   |
| Keymap, repeat, modifiers               | seat's own        | host keymap text kept; clients repeat                                | libinput + seat                             | same                      |
| libinput settings (`mouse`, `touchpad`) | n/a               | n/a                                                                  | applied per device (`libinput/settings.rs`) | n/a                       |
| Relative pointer, pointer constraints   | none              | none: host lock never requested                                      | libinput relative                           | fix (f)                   |
| Shortcut inhibit (Super keys)           | n/a               | none: host eats its shortcuts                                        | n/a                                         | fix (f)                   |
| Gestures (swipe, pinch)                 | injected          | none from host                                                       | `Touchpad` samples                          | missing nested            |
| Switches (lid)                          | none              | none                                                                 | caps only [INFERENCE]                       | n/a                       |
| IME                                     | compositor-level  | none from host                                                       | none                                        | missing everywhere        |

### Capture, controls, host boundary

| Behaviour                                | Headless                        | Nested                                                      | DRM                                | Result                          |
| ---------------------------------------- | ------------------------------- | ----------------------------------------------------------- | ---------------------------------- | ------------------------------- |
| `capture`, screenshots, secure masking   | readback                        | readback (software)                                         | `None`: no readback                | differs, DRM refuses by `None`  |
| Screencopy, image-copy-capture, PipeWire | compositor level over `capture` | same                                                        | none (no readback)                 | follows `capture`               |
| Gamma, night light                       | applied (virtual)               | default `Unsupported`, flag false                           | kernel LUT                         | refuse nested, test it          |
| Power (DPMS)                             | applied (virtual)               | default `Unsupported`, flag false                           | kernel DPMS                        | refuse nested, test it          |
| Idle, session lock                       | compositor level                | same                                                        | same                               | same                            |
| Clipboard, primary selection             | clients only                    | not bridged with the host                                   | clients only                       | fix (f)                         |
| Cursor                                   | drawn into the frame            | drawn into the frame and the host cursor shows too          | drawn, hardware cursor [INFERENCE] | hide host cursor                |
| Window title, app id                     | n/a                             | set; no icon                                                | n/a                                | add icon if host offers it      |
| Fullscreen, resize, close                | n/a                             | fullscreen opt-in; resize follows; close removes the output | n/a                                | close of the last = session end |
| Multi-output                             | many virtual                    | many windows at connect                                     | many connectors                    | nested hotplug (c)              |
| Shutdown                                 | drop                            | drop closes the connection                                  | drop releases seat                 | same                            |
| Selection                                | `Backend::Headless` default     | `Backend::Nested`, test only                                | `Backend::Drm` refused             | wire all three                  |

## What landed, and what did not

| Item                                                                                                      | State                                                                                                                                                                                                                                                                                     | Proof                                                                                      |
| --------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| (a) nested renders with software, GLES or Vulkan                                                          | done: offscreen render, readback, `wl_shm`; no host dmabuf present                                                                                                                                                                                                                        | GLES and Vulkan tests in `gnoblin-wayland/tests/nested.rs` (llvmpipe, lavapipe)            |
| (b) dmabuf offers, GPU views, footprint, texture limit, effects on nested                                 | done, one shared code path with headless (`gnoblin-backend/src/gpu.rs`)                                                                                                                                                                                                                   | `assert_answers_like_headless` in `tests/nested.rs`                                        |
| (c) runtime hotplug of host windows                                                                       | done (`NestedHandle`); modes stay `Unsupported`: the host window is the only mode                                                                                                                                                                                                         | `a_handle_opens_and_closes_output_windows...`                                              |
| (d) host scale follow                                                                                     | done: fractional scale, and the integer scale of the `wl_output` a surface is on, with every host output bound                                                                                                                                                                            | `a_host_scale_change_reaches_the_nested_output`; the integer fallback has table tests only |
| (e) gamma and power on nested                                                                             | refused: default `Unsupported`, controls false, because a host monitor's gamma is shared with every other window                                                                                                                                                                          | not yet asserted by a shared suite                                                         |
| (f) devices, key release on focus loss, shortcut inhibit, host cursor hidden, swipe and pinch             | done                                                                                                                                                                                                                                                                                      | seat unit tests; `the_nested_window_asks_the_host_to_keep_its_shortcuts...`                |
| (f) relative pointer, pointer constraints, tablets, clipboard and primary selection bridging, window icon | not done: each needs a hook in the `Backend` contract                                                                                                                                                                                                                                     | open                                                                                       |
| (g) close and host death                                                                                  | done: the last window closed is `BackendEvent::SessionEnded` (a logout); a lost device is `Disconnected` and the loop fails with `LoopError::BackendLost`, for any backend                                                                                                                | `tests/backend_end.rs`, `closing_the_last_window...`                                       |
| conformance suite and status table                                                                        | not done                                                                                                                                                                                                                                                                                  | open                                                                                       |
| (h) `compositor.backend` and wiring `Backend::Drm`                                                        | not done: nothing in the shipped binary selects a backend, and `Backend::build` still refuses DRM. Plan: a `compositor.backend` leaf with `GNOBLIN_BACKEND` over it, `Drm::open` behind `Backend::Drm` with a hint per `DrmError`, the GLES renderer only, `GNOBLIN_DRM_MODESET` left off | open                                                                                       |

Proven where: software, GLES (llvmpipe) and Vulkan (lavapipe) under gVisor, with nested hosted by this repository's own
headless compositor. Not proven: a real Sway or Mutter host, real DRM hardware, a real keyboard focus change on a host.
