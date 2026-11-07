# Constants audit: backend group

Read-only audit of the seven crates in the backend group. No Rust was changed or built. Counts come from `grep -rnE '^\s*(pub(\([a-z]+\))? )?(const|static) [A-Z_0-9]+'` over each `crates/gnoblin-*/src`, excluding `tests.rs` and `protection_tests.rs`. Inline magic numbers were skimmed, not exhaustively listed.

Config leaves used below come from `crates/gnoblin-config` and `crates/gnoblin-schema/schema/*.toml`. Named leaves that exist today:

- xwayland: `allow_grabs`, `allow_byte_swapped_clients`, `grab_access_rules`, `disable_extensions`, `scaling_factor` (`gnoblin-config` document.rs).
- output scale: `hooks.rs` accepts 0 or a value of at least 0.5.
- No lock timing leaf exists, and `gnoblin-lock` has no timing constants in non-test code.

Verdict key: KEEP, CONFIGURE, DERIVE, REMOVE, MOVE. Risk and effort use the scale in the brief.

## 1. Summary per crate

| Crate                   | Listed (non-test) | Individually in tables | Grouped | Notes                                                                 |
| ----------------------- | ----------------- | ---------------------- | ------- | --------------------------------------------------------------------- |
| gnoblin-backend         | 67                | 45                     | 22      | Includes shader source names, Vulkan and GLES layout values           |
| gnoblin-output          | 4                 | 4                      | 0       | EDID block layout; `FULL_HD` is a test fixture [INFERENCE]            |
| gnoblin-capture         | 0                 | 0                      | 0       | Only `table.rs:389` `SIZE`, a test fixture [INFERENCE]                |
| gnoblin-lock            | 0                 | 0                      | 0       | `state.rs:62,69` are `RELEASED`/`HELD` ownership values, not tunables |
| gnoblin-pipewire-export | 6                 | 6                      | 0       | Stream rate, buffer count, start deadline                             |
| gnoblin-input-eis       | 13                | 4                      | 9       | Caps bit flags are a protocol set                                     |
| gnoblin-xwayland        | 5                 | 5                      | 0       | Supervisor restart schedule and X11 XID band                          |

Total non-test items: 95 listed by grep. The 1000-item estimate in the brief is not borne out by this group. Most "constants" here are test fixtures or type-system values.

## 2. Findings

### 2.1 DRM gamma and present (gnoblin-backend)

| Item                                                 | Location            | Verdict   | Q1    | Q2  | Q3  | Risk | Effort | Notes                                                                                                                                                                                               |
| ---------------------------------------------------- | ------------------- | --------- | ----- | --- | --- | ---- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `DEFAULT_LENGTH = 256`                               | `drm/gamma.rs:39`   | KEEP      | no    | no  | no  | low  | S      | Already a documented fallback only: "it is only a default: the CRTC's own length is what a table is checked against." `GammaRamp::from_channels` checks lengths against the CRTC. No change needed. |
| `MIN_KELVIN = 1000`                                  | `drm/gamma.rs:45`   | CONFIGURE | maybe | no  | no  | low  | S      | A night-light floor. Leaf `display.night_light.min_kelvin`, type u32, range 1000..=6500, default 1000 (current). Confirm the Lua/shell owner first.                                                 |
| `NEUTRAL_KELVIN = 6500`                              | `drm/gamma.rs:51`   | KEEP      | no    | no  | no  | low  | S      | Its value is the sRGB D65 white point, a colour-science invariant.                                                                                                                                  |
| `SLOTS = 2`                                          | `drm/present.rs:49` | KEEP      | no    | no  | no  | low  | S      | Double buffering is an implementation choice tied to the atomic-commit path. Not user-facing.                                                                                                       |
| `FRAME_TIMEOUT = 10 s` (`render/vulkan/frame.rs:49`) | Vulkan fence wait   | KEEP      | no    | no  | no  | low  | S      | A hang detector, not a tunable. A shorter value would misfire on slow GPUs.                                                                                                                         |
| `STAGING_ALIGN = 16` (`frame.rs:52`)                 | Vulkan staging      | KEEP      | no    | no  | no  | low  | S      | Buffer-to-image copy offsets need the texel block size; 16 covers it. `minUniformBufferOffsetAlignment` does not apply here. `optimalBufferCopyOffsetAlignment` is a speed hint only [INFERENCE].   |

### 2.2 Headless and nested (gnoblin-backend)

| Item                                                       | Location                     | Verdict             | Q1  | Q2  | Q3  | Risk   | Effort | Notes                                                                                                                                                                                  |
| ---------------------------------------------------------- | ---------------------------- | ------------------- | --- | --- | --- | ------ | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `HEADLESS_GAMMA_SIZE = 256`                                | `headless.rs:71`             | KEEP                | no  | no  | no  | low    | S      | A fake CRTC for tests and headless runs; no hardware involved.                                                                                                                         |
| `DEFAULT_MODE = 1024x768@60000`                            | `headless.rs:92`             | CONFIGURE           | yes | no  | no  | low    | S      | Leaf `backend.headless.mode`, type mode string, default `1024x768@60`. Headless is used for CI, so a user may want 1080p.                                                              |
| `DEFAULT_CONNECTOR = "HEADLESS-1"`                         | `headless.rs:98`             | KEEP                | no  | no  | no  | low    | S      | Name is only seen by the session; no upstream source.                                                                                                                                  |
| `UNCONSTRAINED_SIZE = 1024x768`                            | `nested/mod.rs:121`          | KEEP                | no  | no  | no  | low    | S      | The initial size a nested window asks for before the parent configures it. The parent decides the real size.                                                                           |
| `DEFAULT_CONNECTOR = "NESTED-1"`                           | `nested/mod.rs:124`          | KEEP                | no  | no  | no  | low    | S      | As for headless.                                                                                                                                                                       |
| `DEFAULT_APP_ID = "org.gnoblin.Session"`                   | `nested/mod.rs:127`          | KEEP                | no  | no  | no  | low    | S      | Identifier shown to the parent compositor. Changing it would break parent-side matching rules.                                                                                         |
| `DEFAULT_REFRESH_MHZ = 60000`                              | `nested/mod.rs:134`          | DERIVE              | no  | yes | no  | medium | S      | Use the parent output's refresh rate when stated (`wl_output.mode`). The constant is only the fallback. Documented as such in the code.                                                |
| `SCALE_DENOMINATOR = 120`                                  | `nested/mod.rs:137`          | KEEP                | no  | no  | no  | low    | S      | `wp_fractional_scale_v1` defines the preferred scale as a fraction of 120 (wayland-protocols XML, see Section 5). Protocol invariant.                                                  |
| `CONFIGURE_ROUNDTRIPS = 32`                                | `nested/mod.rs:143`          | KEEP                | no  | no  | no  | low    | S      | Bound on a broken parent, documented as such. Not a schedule.                                                                                                                          |
| `MAX_KEYMAP_BYTES = 4 MiB`                                 | `nested/handler.rs:330`      | KEEP                | no  | no  | no  | low    | S      | Sanity cap on an fd-supplied keymap, a defence against a hostile parent.                                                                                                               |
| `SLOTS = 2`, `SLOT_ALIGNMENT = 4096`, `MAX_EXTENT = 16384` | `nested/surface.rs:72,78,85` | DERIVE (MAX_EXTENT) | no  | yes | no  | medium | M      | `MAX_EXTENT` should come from the device: `GL_MAX_TEXTURE_SIZE` or `maxImageDimension2D`. The 4096 slot alignment is a page-size choice; use `sysconf(_SC_PAGESIZE)` if it must match. |
| `POOL_COUNTER`                                             | `nested/surface.rs:89`       | KEEP                | no  | no  | no  | low    | S      | Unique name generator for shm pools. Not a constant in spirit.                                                                                                                         |

### 2.3 Input conversion constants (gnoblin-backend and gnoblin-input-eis)

| Item                                           | Location                              | Verdict | Q1  | Q2  | Q3  | Risk   | Effort | Notes                                                                                                                                                                                                                          |
| ---------------------------------------------- | ------------------------------------- | ------- | --- | --- | --- | ------ | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `STEPS_PER_UNIT = 120.0`                       | `libinput/map.rs:54`                  | KEEP    | no  | no  | no  | low    | S      | 120 units per wheel notch is the Windows and `wl_pointer` v8 convention for `axis_value120`.                                                                                                                                   |
| `AXIS_STEP = 120`                              | `nested/input.rs:37`                  | KEEP    | no  | no  | no  | low    | S      | Same convention; wire value for `axis_value120`.                                                                                                                                                                               |
| `KEY_OFFSET = 8`                               | `nested/input.rs:45`                  | DERIVE  | no  | yes | no  | medium | S      | The +8 is the X11/xkb keycode offset applied to `wl_keyboard.key` [INFERENCE]. Current `wayland.xml` fetch did not state it. Prefer a named helper that uses `xkbcommon`'s keycode conversion, or keep as a documented offset. |
| `UNITS_PER_STEP = 10.0`                        | `nested/input.rs:53`                  | MOVE    | no  | no  | no  | low    | S      | Same value as `PIXELS_PER_STEP` in gnoblin-input-eis (10 px per notch). Duplicated convention; move to one shared constant, e.g. the input crate.                                                                              |
| `CODE_MAX = 0x2ff`                             | `gnoblin-input-eis/src/service.rs:37` | DERIVE  | no  | yes | no  | medium | S      | Use `KEY_MAX` from `linux/input-event-codes.h` via `input-event-codes` or the `evdev` crate. [INFERENCE] that the header value is 0x2ff; the first 300 lines of the header fetched did not show it.                            |
| `PIXELS_PER_STEP = 10.0`                       | `input-eis/src/service.rs:46`         | MOVE    | no  | no  | no  | low    | S      | Duplicate of the nested-backend convention above; one shared item.                                                                                                                                                             |
| `UNITS_PER_STEP = 120.0`                       | `input-eis/src/service.rs:49`         | KEEP    | no  | no  | no  | low    | S      | Same wire unit as `STEPS_PER_UNIT`; EIS scroll-discrete convention.                                                                                                                                                            |
| Caps bit flags (`NONE`, `POINTER`, ..., `ALL`) | `input-eis/src/service.rs:76-90`      | KEEP    | no  | no  | no  | low    | S      | Bit values are the EIS protocol's device capability set. `ALL = 0b0011_1111` must match the flags defined; a `bitflags!` type removes the hand-written mask. Not user-tunable.                                                 |
| `EVERY`, `GROUPS` test arrays                  | `input-eis/src/service.rs:140,1103`   | KEEP    | no  | no  | no  | low    | S      | Within a test module [INFERENCE]; no action.                                                                                                                                                                                   |

### 2.4 Render and shader sizes (gnoblin-backend)

| Item                                                            | Location                                                        | Verdict   | Q1  | Q2  | Q3  | Risk   | Effort | Notes                                                                                                                                                                                                                                                          |
| --------------------------------------------------------------- | --------------------------------------------------------------- | --------- | --- | --- | --- | ------ | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `MOTION_MS = 200`                                               | `render/compose.rs:74`                                          | CONFIGURE | yes | no  | no  | low    | S      | Compose motion duration. Leaf: an animation duration, type u32 ms, range 0..=2000, default 200. Should be the shared animation leaf, not a backend constant. [INFERENCE] on which leaf family is intended.                                                     |
| `MAX_RADIUS = 4096.0`                                           | `render/gles/mod.rs:129`                                        | DERIVE    | no  | yes | yes | medium | S      | A shader clamp. Derive from the largest supported framebuffer dimension or drop by clamping to half the surface size.                                                                                                                                          |
| `UNIFORM_BINDING = 0`, `TEXTURE_UNIT = 0`, `VERTEX_FLOATS = 24` | `render/gles/gl.rs:61,64,69`                                    | KEEP      | no  | no  | no  | low    | S      | GL binding and layout values fixed by the shader. Not tunable.                                                                                                                                                                                                 |
| `UNIFORM_STRIDE = 256`                                          | `render/vulkan/plan.rs:40`                                      | KEEP      | no  | no  | no  | low    | S      | The spec caps `minUniformBufferOffsetAlignment` at 256, so 256 is valid everywhere The required-limits table lists max 256 (https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html). Power-of-two (so it divides 256) is [INFERENCE]: not re-read.. |
| `UNIFORM_BYTES = 96`, `FIXED_BYTES = 96`                        | `render/vulkan/pipeline.rs:84`, `render/shader/uniforms.rs:35`  | KEEP      | no  | no  | no  | low    | S      | Same layout, two copies. Better as one shared layout constant or derived from `std::mem::size_of`.                                                                                                                                                             |
| `INSTANCE_FLOATS = 22`                                          | `render/vulkan/pipeline.rs:87`                                  | KEEP      | no  | no  | no  | low    | S      | Instance vertex layout. Use `size_of::<Instance>() / 4` to remove drift risk.                                                                                                                                                                                  |
| `POOL_SETS = 256`                                               | `render/vulkan/pipeline.rs:90`                                  | REMOVE    | no  | no  | yes | low    | S      | A pool capacity guess. Size the pool from the number of descriptor sets actually allocated, or grow on demand.                                                                                                                                                 |
| `TARGET_FORMAT`, `SHM_FORMAT`, `READBACK_FORMAT`                | `render/vulkan/context.rs:52,55`, `render/software.rs:80,90,92` | KEEP      | no  | no  | no  | low    | S      | Format choices with a documented reason (shm byte order). The readback format is `cfg`-selected per platform; keep, but check it is documented.                                                                                                                |
| `VALIDATION_LAYER`                                              | `render/vulkan/context.rs:47`                                   | KEEP      | no  | no  | no  | low    | S      | Khronos layer name; string is the Vulkan spec name.                                                                                                                                                                                                            |
| `ENTRY`, `PREAMBLE`, `USER_FIELDS`, shader sources              | `render/shader/*`                                               | KEEP      | no  | no  | no  | low    | S      | Compile-time shader text. `USER_FIELDS` is a marker string for user shader injection.                                                                                                                                                                          |
| `MAGIC = 0x0723_0203`                                           | `render/vulkan/shader.rs:84`                                    | KEEP      | no  | no  | no  | low    | S      | SPIR-V magic number, from the SPIR-V specification.                                                                                                                                                                                                            |
| `BLOCK_RANGE`, `BLEND`                                          | `render/vulkan/pipeline.rs:484`, `vulkan/tests.rs:34`           | KEEP      | no  | no  | no  | low    | S      | Derived or test value.                                                                                                                                                                                                                                         |
| `GLASS_RGB`, `GLASS_TOLERANCE`                                  | `render/gles/tests.rs:1054,1057`                                | KEEP      | no  | no  | no  | low    | S      | Test-only.                                                                                                                                                                                                                                                     |
| `DOWNSCALE_IGNORED`                                             | `render/gles/mod.rs:1002`                                       | KEEP      | no  | no  | no  | low    | S      | A `Once` log guard, not a tunable.                                                                                                                                                                                                                             |

### 2.5 Output and EDID (gnoblin-output)

| Item                                         | Location                | Verdict | Q1  | Q2  | Q3  | Risk | Effort | Notes                                                |
| -------------------------------------------- | ----------------------- | ------- | --- | --- | --- | ---- | ------ | ---------------------------------------------------- |
| `BASE_BLOCK = 128`                           | `output/src/edid.rs:7`  | KEEP    | no  | no  | no  | low  | S      | EDID 1.4 base block size, VESA spec.                 |
| `HEADER = [00 FF FF FF FF FF FF 00]`         | `edid.rs:10`            | KEEP    | no  | no  | no  | low  | S      | EDID header, VESA spec.                              |
| `DESCRIPTORS = 54`, `DESCRIPTOR_LENGTH = 18` | `edid.rs:13,16`         | KEEP    | no  | no  | no  | low  | S      | EDID descriptor offsets. Protocol-fixed.             |
| `FULL_HD`                                    | `output/src/mode.rs:64` | REMOVE  | no  | no  | no  | low  | S      | Test fixture [INFERENCE]. Keep out of non-test code. |

### 2.6 Capture and lock

No non-test constants in `gnoblin-capture`. `table.rs:389` `SIZE` is a test fixture [INFERENCE].

`gnoblin-lock` has no timing constants in non-test code. `state.rs:62,69` define `RELEASED` and `HELD`, which are ownership states. KEEP. There is no lock timeout or grace-period leaf. Adding one is a product decision (Section 6).

### 2.7 PipeWire export (gnoblin-pipewire-export)

| Item                     | Location     | Verdict   | Q1  | Q2  | Q3  | Risk   | Effort | Notes                                                                                                                                                                                      |
| ------------------------ | ------------ | --------- | --- | --- | --- | ------ | ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `FRAME_RATE = 30`        | `lib.rs:46`  | CONFIGURE | yes | yes | no  | medium | M      | Screen-share frame rate. Leaf `capture.screencast.framerate`, u32 fps, range 1..=120, default 30. The PipeWire negotiated rate should be the one that is used, not a fixed 30 [INFERENCE]. |
| `FRAME_INTERVAL = 33 ms` | `lib.rs:48`  | DERIVE    | no  | no  | yes | low    | S      | Derive from `FRAME_RATE`: `1000 / rate` ms. It is 33.33 ms, rounded. Remove the second constant.                                                                                           |
| `BUFFER_COUNT = 4`       | `lib.rs:50`  | KEEP      | no  | yes | no  | low    | S      | Buffer pool count negotiated with PipeWire; the consumer reports it (`SPA_PARAM_Buffers`). Keep as the request, check the reply.                                                           |
| `START_DEADLINE = 5 s`   | `lib.rs:52`  | KEEP      | no  | no  | no  | low    | S      | Stream start watchdog. Not user-facing.                                                                                                                                                    |
| `BUFFER: u64 = 1`        | `lib.rs:54`  | KEEP      | no  | no  | no  | low    | S      | Buffer id/flag value; check against `spa_buffer` header. Use a named flag type. [INFERENCE]                                                                                                |
| `BYTES_PER_PIXEL = 4`    | `lib.rs:146` | DERIVE    | no  | yes | no  | low    | S      | Derive from the negotiated `SPA_VIDEO_FORMAT`'s stride (`spa_video_info_raw`). Only 4 bpp formats are used today.                                                                          |
| `INIT: Once`             | `lib.rs:158` | KEEP      | no  | no  | no  | low    | S      | One-time init guard.                                                                                                                                                                       |

### 2.8 XWayland (gnoblin-xwayland)

| Item                      | Location           | Verdict | Q1  | Q2  | Q3  | Risk | Effort | Notes                                                                |
| ------------------------- | ------------------ | ------- | --- | --- | --- | ---- | ------ | -------------------------------------------------------------------- |
| `FIRST_DELAY = 500 ms`    | `supervisor.rs:50` | KEEP    | no  | no  | yes | low  | S      | Restart back-off. Internal policy.                                   |
| `MAX_DELAY = 8 s`         | `supervisor.rs:52` | KEEP    | no  | no  | no  | low  | S      | Back-off cap.                                                        |
| `ATTEMPTS = 5`            | `supervisor.rs:54` | KEEP    | no  | no  | no  | low  | S      | Restart budget.                                                      |
| `STARTUP_DEADLINE = 10 s` | `supervisor.rs:56` | KEEP    | no  | no  | no  | low  | S      | Startup watchdog. Not user-facing.                                   |
| `X11_BAND = 1 << 63`      | `windows.rs:45`    | KEEP    | no  | no  | no  | low  | S      | XID space split; the X11 resource-id partition is set by the server. |

XWayland user settings already exist as leaves (Section 1). The supervisor timings are internal and do not need config.

## 3. Top ten wins

Ranked by user value over effort.

1. Derive `MAX_EXTENT` and the gamma size from the device (see below). The Vulkan `STAGING_ALIGN` and `UNIFORM_STRIDE` rows were corrected to KEEP after review. Effort S.
2. Derive `MAX_EXTENT` from `GL_MAX_TEXTURE_SIZE` or `maxImageDimension2D`. Removes a wrong upper bound on big outputs. Effort M.
3. Derive `DEFAULT_LENGTH` (gamma) from the CRTC's gamma LUT size. Removes a hard-coded 256 that some hardware does not match. Effort S.
4. Make `FRAME_RATE` a leaf (`capture.screencast.framerate`) and derive `FRAME_INTERVAL` from it. Effort M.
5. Make `MOTION_MS` a shared animation leaf. Users can already change other motion; this closes the gap. Effort S.
6. Derive `CODE_MAX` from `KEY_MAX` in `input-event-codes.h`. Removes a hand-copied kernel value. Effort S.
7. Move `PIXELS_PER_STEP`/`UNITS_PER_STEP` duplicates into one shared constant. Effort S.
8. Make headless `DEFAULT_MODE` a leaf. Useful for CI and screenshots. Effort S.
9. Derive nested `DEFAULT_REFRESH_MHZ` from parent `wl_output.mode` everywhere; the fallback is already the rule. Effort S.
10. Replace `POOL_SETS = 256` with a size computed from actual allocations. Effort S.

## 4. Constants to leave alone and why

- Protocol and spec values: `BASE_BLOCK`, EDID header and descriptor layout, `SCALE_DENOMINATOR = 120`, `AXIS_STEP`, `STEPS_PER_UNIT`, SPIR-V magic, Vulkan layer name, EIS caps bits. Changing these is a wire-format break.
- Safety bounds: `FRAME_TIMEOUT`, `MAX_KEYMAP_BYTES`, `CONFIGURE_ROUNDTRIPS`, `START_DEADLINE`, `STARTUP_DEADLINE`. Not user-visible; a tunable would invite misconfiguration.
- Identity strings: `DEFAULT_APP_ID`, `HEADLESS-1`, `NESTED-1`, `ENTRY`, shader fragments. Changing them breaks parent-side rules or shader injection.
- Internal policy: XWayland back-off (`FIRST_DELAY`, `MAX_DELAY`, `ATTEMPTS`), `SLOTS`, `POOL_COUNTER`, `X11_BAND`.
- Test fixtures: `SIZE`, `FULL_HD`, `SESSION`, `OUTPUT`, etc. Out of scope for config; they stay in test modules.

## 5. Upstream evidence

- `wayland.xml` (wayland core) fetched. It does not state a keycode offset. The +8 in `nested/input.rs:45` is the xkb keycode convention [INFERENCE]. Source: https://gitlab.freedesktop.org/wayland/wayland/-/blob/main/protocol/wayland.xml
- `input-event-codes.h` fetched (https://github.com/torvalds/linux/blob/master/include/uapi/linux/input-event-codes.h). `KEY_MAX = 0x2ff` was not seen in the first 300 lines [INFERENCE]; confirm before deriving.
- `wp_fractional_scale_v1` scale denominator 120: wayland-protocols staging XML [INFERENCE] on exact wording; verify before citing.
- DRM gamma LUT size and Vulkan `VkPhysicalDeviceLimits` are the authoritative sources for the DERIVE rows; not re-fetched in this pass.

## 6. Needs Kieran

- Should `MOTION_MS` live in the shared animation leaf, or in a compositor-wide `motion` section? It affects Lua and shell naming.
- Should `FRAME_RATE` be a per-stream leaf or a global screencast leaf?
- Is the lock needing a timeout or grace-period leaf? None exists; adding one is a product decision.
- Is headless `DEFAULT_MODE` a user-facing leaf or a test-only knob?
