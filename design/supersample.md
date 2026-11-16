# Supersampling: compose at `mode * s`, show the real mode

This is an owner-requested addition beyond `design/archive/js-spec-native.md` part 2: the spec's
`OutputProfile` has `scale` and no `supersample`. User page: `docs/shell-api/outputs.md`.

## What was verified before building

- **`wl_output`** (`/usr/share/wayland/wayland.xml`): `mode` is the panel's pixels; `scale` is an integer the client
  uses for buffer scale. Nothing ties `mode / scale` to the desktop's logical size.
- **`xdg_output`** (`xdg-output-unstable-v1.xml`): `logical_size` "describes the size of the output in the global
  compositor space" and its text names compositors whose logical size is not `mode / wl_output.scale` (the fractional
  1.5 example: mode 3840x2160, logical 2560x1440). So a real mode, `wl_output.scale = ceil(u*s)` and a logical size of
  `mode / u` is a legal combination, and it is the one this feature needs.
- **`wp_fractional_scale_v1`**: `preferred_scale` is the numerator over 120. At `u*s = 2` that is 240.
- **Smithay `65aa86f`** (`src/output.rs`, `src/wayland/output/xdg.rs`):
    - `Scale::Fractional(f)` sends `ceil(f)` on `wl_output.scale` and `f` to fractional-scale.
    - The xdg-output handler derives `logical_size` from `current_mode.size / scale.fractional_scale()` with no override.
    - With a real mode and scale `u*s` it would send `mode / (u*s)`, which breaks the invariant.
    - So the stock `new_with_xdg_output` global is replaced by our own `zxdg_output_manager_v1`
      (`crates/gnoblin-wayland/src/protocols/zxdg_output_v1.rs`), which sends the layout rectangle's size.
    - `OutputManagerState::new()` still serves `wl_output`.
- **The tree.**
    - A frame's rectangles are logical, and every renderer maps them with the target's `scale` into a target of
      `physical` pixels (`physical_rect`).
    - Blur radius, shadow blur, spread and offset, corner radius and border width are logical and already multiplied
      by `scale` (GLES `Painter`, software `shape`).
    - Capture sizes are `logical rect * Output::current_scale()`; the cursor shape lookup and the fractional-scale
      preferred scale read the same value.
    - So feeding `u*s` as the compositing scale and `mode*s` as the physical size is the whole renderer change.
- **X11.**
    - The tree set Smithay's per-client `client_scale` once at Xwayland start from `xwayland.scaling-factor` (automatic =
      1.0) and published `Xft/DPI` only from the desktop settings. It never followed an output's scale.
    - Smithay's window manager already converts every geometry between X pixels and logical pixels with that scale
      (`xwm/mod.rs`: `to_logical(client_scale)` on the way in, `configure` multiplies on the way out).
    - So the compositor must not divide or multiply again: a first version did, and the real Xwayland test showed 300 X
      pixels as 75 logical ones.
- **DRM.** `Drm::render` draws a frame straight into the GBM scan-out buffer with `Gles::draw_scanout`; there is no
  client-buffer direct scan-out or overlay plane anywhere in the crate. `Backend::capture` is `None` there, before and
  after this feature: a hardware output has no screenshot at any factor. That is left unsupported on purpose (it is a
  separate feature: a readback of the scan-out buffer), and `docs/shell-api/outputs.md` says so.

## The invariant

Logical size never changes. `u` is the existing `scale` (default 1), `s` the new supersample factor (>= 1).

| Quantity                                                              | Value                      | Source of truth             |
| --------------------------------------------------------------------- | -------------------------- | --------------------------- |
| `Output::mode`                                                        | the real hardware mode     | backend                     |
| `Output::scale`                                                       | `u`                        | profile / backend           |
| `Output::supersample`                                                 | `s`                        | profile / backend (new)     |
| logical size, layout rectangle, pointer, `screen.screen_width/height` | `mode / u`                 | unchanged                   |
| virtual size                                                          | `mode * s` (must be whole) | `Output::virtual_size`      |
| compositing scale (renderer, Smithay, wp_fractional_scale)            | `u * s`                    | `Output::effective_scale`   |
| `wl_output.mode`, wlr-output-management heads                         | real mode, head scale `u`  | unchanged                   |
| `wl_output.scale` / preferred scale                                   | `ceil(u*s)` / exact `u*s`  | Smithay `Scale::Fractional` |
| `xdg_output.logical_size`                                             | `mode / u`                 | layout rectangle            |
| capture buffer (screencopy, image-copy, PipeWire, `capture.screen`)   | `mode * s`                 | `rect * effective_scale`    |

`screen.output_scale` stays `u`; `screen.supersample` is new and read-only. JS derives `virtualSize` and the effective
scale from the two.

## Architecture

1. **Model** (`gnoblin-output`, `gnoblin-types`). `Scale::product`; `supersample` module: parse (`2`, `1.5`, `default`),
   `virtual_size(mode, s)` with the whole-size and 8192 limits, the `is_recommended` rule, `ProfileChange`/`OutputProfile`
   key `supersample`. The layout engine is untouched: logical size does not depend on `s`.
2. **Backend contract** (`gnoblin-backend`). `Output.supersample`; `Output::virtual_size`, `effective_scale`.
   `Backend::configure_output` / `test_output` take the factor; `BackendEvent::OutputResized` carries it. A target's
   `scale()` is the effective scale and its buffer is `virtual_size`. The nested backend takes the factor only when
   its buffer is the mode and refuses it by name otherwise; it refuses a mode or scale change, as it always did.
3. **Downsample.** `Image::downsample(size)` in `gnoblin-render-frame`: exact box filter (average of s x s texels, rounded
   half up per channel) for an integer ratio, bilinear for any other. `Backend::capture_display` is the downsampled
   readback; `Backend::capture` is the internal hi-res frame, which is what every capture path uses.
4. **GLES scan-out** (`gnoblin-render-gles`, `gnoblin-backend-drm`).
    - The DRM output keeps one persistent virtual-size target and draws damage into it with scale `u*s`.
    - A box-filter pass (`downsample` fragment shader, texel-centre taps) then fills the real-size scan-out buffer
      whole, because a flip buffer holds an old frame.
    - A refused target allocation leaves the old buffers, as a modeset does.
    - No direct scan-out exists, so none has to be disabled; `Drm::render` always composes.
5. **Protocols** (`gnoblin-wayland`). `refresh_output` hands Smithay `Fractional(u*s)` (`Integer` when whole); the
   composer's canvas scale takes `u*s` so canvases paint sharp. Own xdg-output (above). Mirrors fit the display-size
   frame (`capture_display`), not the hi-res one.
6. **Captures.** Everything reading `Backend::capture` or the output frame is hi-res by default. The compositor's own
   `capture.screen` still takes a `display` flag in its payload for the downsampled frame; portal Screenshot and
   `gnoblinctl capture` pass it through. Secure treatment is applied before any resampling.
7. **X11.** The Xwayland client scale becomes `scaling-factor * s_max` where `s_max` is the largest supersample among
   outputs (a per-client value in Smithay), re-applied live, and XSETTINGS gets `Gdk/WindowScalingFactor = s` and
   `Xft/DPI * s` so toolkits draw at s times the pixels at the same size. Smithay's `set_xsettings` only adds, so a list
   that loses a setting (the scale back to one) clears what was published first.
8. **Limit.** `supersample.max-virtual` is a constant (8192 per edge) in `gnoblin-output`; GLES also checks
   `GL_MAX_TEXTURE_SIZE`. A refusal names the factor, the mode, the virtual size and the limit.
9. **JS.** `Output.apply({ supersample })`, `OutputProfile({ supersample })`, `output.supersample`,
   `output.virtualSize`, `output.effectiveScale`. A factor other than 1, 2, 4 is accepted when `mode*s` is whole and
   logs one `supersample N is not recommended: use 1, 2 or 4`; NaN, < 1 and too-big values throw with the property path.

## Failure modes first

1. **Odd factor with a fractional virtual size** (1.5 on 1367 px): refused naming the axis.
2. **Virtual size over the limit**: refused, nothing changed (`test_output` is run before any output is driven).
3. **A change on a live output** resizes the target and re-sends the scale; a refused resize keeps the old buffers and
   the profile is not stored (`apply_in_order` rolls back).
4. **A hotplugged output** with a stored profile gets `s` back through `settle_outputs`.
5. **Secure content** is masked in the internal frame before it is downsampled or captured.
6. **A buffer-scale-2 client at `u*s = 2`** is drawn 1:1 (logical size = buffer / 2, physical = logical * 2).

## Status and where it is proven

Every row was run; "fake" and "real" say what the test stood on. Tests named are the ones to read.

- **Software, headless.** Model, policy, rollback, hotplug, the Smithay scale, our own xdg-output, hi-res captures and the
  `display` flag. Real `wayland-client` connections (`tests/supersample.rs`). Not proven: a GPU.
- **GLES on llvmpipe.** The box filter equals the software renderer's (checker, one-pixel line), the scan-out buffer,
  damage, and the texture-limit refusal (`downsample_tests.rs`). Not proven: a real GPU's filtering.
- **DRM.** The virtual target and the downscale into the scan-out buffer, and a refused allocation keeping the old
  buffers. A fake kernel with the modeset opt-in enabled (`modeset.rs`, `plan.rs`). Not proven: any real modeset, flip or
  display.
- **Nested.** The factor is accepted, the capture is hi-res, the parent shows the averaged frame, and refusals name the
  factor. A real host compositor (`tests/nested.rs`). Not proven: a parent that rounds a fractional scale.
- **Xwayland.** The client scale is `u * s`, the logical geometry is unchanged, the X client is configured in X pixels,
  and XSETTINGS follows the outputs and goes back when they do. Real Xwayland 22.1.9 (`tests/real_xwayland.rs`). Not
  proven: pairing through `xwayland-shell-v1`, which needs 23.1 (those tests skip here).
- **Vulkan.** Draws at `u * s` into `mode * s`, the capture is hi-res, and the displayed frame is its average. Lavapipe
  (`a_vulkan_headless_output_can_be_supersampled`). Not proven: a real GPU.
- **IPC.** `output.apply` takes `supersample`, `output.supersample` is published, and a refusal names the setting. The
  real server and `gnoblinctl` (`tests/output_apply.rs`).
- **JS.** The setting, the one-time notice and a profile re-applying a changed factor, on the fake compositor
  (`tests/displays.rs`).

Decisions that are not gaps:

- **DRM capture.** `Drm::capture` is `None` and stays so: no output is screenshot-able on hardware at any factor, which
  is older than this feature. Implementing it would be a readback of the scan-out buffer and a separate change.
- **Vulkan.** The renderer takes any compositing scale and any image the device allows, so there is nothing to refuse. Its
  `maxImageDimension2D` is reported by `max_texture_size` like GLES's limit, so a factor past it is refused with the numbers
  before an image is made.
- **X11 client scale.** Smithay's window manager converts X pixels and logical pixels itself; the compositor only sets
  `client_scale`. The pairing tests that need Xwayland 23.1 skip on 22.1.9 and are not claimed here.
