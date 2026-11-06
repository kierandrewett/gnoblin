# Vulkan render backend

This note records how the renderers are wired today, the options for a Vulkan
binding, the choice for the first slice and what that slice leaves out. Facts are
read from the tree at `origin/rust-compositor`. Judgement is marked as such.

## How a renderer is selected and driven today

There is no renderer trait. Each renderer is a struct with the same four inherent
methods, and the callers dispatch on an enum.

- `Software` (`render/software.rs`) is Smithay's Pixman renderer drawing into an
  offscreen buffer. `Gles` (`render/gles/`) is a surfaceless EGL context driven
  through `glow`.
- Both expose `new(physical: Size)`, `draw(&OutputFrame, Scale)`,
  `capture(with_cursor) -> Image` and `resize(Size)`. Errors are the small `Copy`
  `RenderError` (`Unavailable`, `Degenerate`, `Failed`).
- `headless.rs` holds the only dispatch: `HeadlessRenderer { Software, Gles }`
  builds a `TargetRenderer` enum whose `draw`, `capture` and `resize` match on the
  variant. `Headless::new()` is `Software`, and `HeadlessConfig::gles()` selects GLES.
- The DRM scan-out path (`drm/scanout.rs`) builds a `Gles` directly on the device's
  EGL display and calls `Gles::draw_scanout`. The nested backend builds a `Software`
  directly (`nested/surface.rs`).
- The compositor never names a renderer. `gnoblin-compositor/src/backend.rs` builds
  `Headless::new()` or `Nested::connect`, and `Drm` is refused by name. The wayland
  crate drives a `Backend` (`render`, `capture`, `screenshot`) and each output's
  `Target::draw(&OutputFrame)` and `Target::present(damage)`.
- No configuration key chooses a renderer. `compositor` has `debug`,
  `locate-pointer` and a few others, and `frame-renderers` is unrelated (it names
  window-frame commands).

## The frame description

`render/frame.rs` is the whole contract. An `OutputFrame` is a background colour,
damage rectangles, layers bottom first, an optional portal `Overlay` and an optional
cursor `Overlay`. Rectangles are output-local logical pixels, and a renderer
multiplies by the scale using `physical_rect` (both edges rounded).

- `Layer::Surface` holds a `wl_buffer`, a fractional source crop, a destination, the
  buffer transform, opacity, tint, corner radius, an `Effects` chain and an
  optional `Model`.
- `Layer::Solid` holds a colour, tint, corner radius, effects and an optional
  `Model`.
- `Model` is a 4x4 matrix plus an `opaque` flag. The GLES path draws these through a
  depth buffer (the D18 pass). The flat path skips them.
- Software ignores corner radius and effects. GLES draws both.

## Effect passes

`gnoblin-render/src/effect.rs` defines `Pass::{Blur, Shadow, Shader}`, ordered per
node. `Blur` has a radius in logical pixels. `Shadow` has an offset, blur, opacity
and colour. `Shader` is a user WGSL function with typed uniforms.

The GLES passes are WGSL files under `render/gles/shaders/`: a shared vertex stage
(`quad.wgsl`, written in Vulkan's y-down clip convention), a preamble with one
96-byte uniform block and the rounded-box SDF, and the bodies `solid`, `surface`,
`shadow` and `blur`. `naga` turns them into GLSL ES 3.10. Every pass blends
premultiplied (`ONE`, `ONE_MINUS_SRC_ALPHA`). Blur copies the backdrop into a
scratch target and runs a separable nine-tap Gaussian over it, horizontal then
vertical. D24 (landed) puts a node's effects inside its opacity group.

## Capture and readback

`Backend::capture(output, with_cursor)` returns an `Image`: RGBA8, rows top first.
Software and GLES draw the overlay and cursor into the same target. Before each one
they save the pixels underneath, and a capture pastes those back so the overlay is
always left out and the cursor only when asked. GLES flips the `glReadPixels` rows.
A Vulkan target needs no flip because the shared vertex stage already maps row 0 to
the top.

## Options for the Vulkan binding

### A: `ash`

Raw, generated bindings. Facts: the `loaded` feature opens `libvulkan` with
`dlopen`, so a compositor starts without a Vulkan loader. Every call is `unsafe`.
All extension loaders exist, including the ones a compositor needs for
`VK_EXT_external_memory_dma_buf`, `VK_EXT_image_drm_format_modifier` and
`VK_KHR_external_semaphore_fd`.

- Pros: full control of memory, queues, layouts and synchronisation. Dmabuf import
  and explicit sync are ordinary Vulkan calls. No second abstraction between us and
  a driver bug. `wgpu-hal`'s Vulkan backend is itself built on `ash`.
- Cons: all resource lifetimes, barriers and layouts are ours. Much `unsafe`,
  which the crate must confine to named modules. More code for the same frame.
- Risk (judgement): sync mistakes show up only under validation or on one driver.
  Mitigated by running the validation layer in debug builds and on lavapipe.
- Shaders: reuse the existing WGSL and compile it to SPIR-V with `naga` (`spv-out`).
- Depth pass: a depth attachment and the `Model` matrix path already in the shared
  vertex stage, so it is a pipeline state change.
- Blur: render passes with intermediate images, or compute. Our choice of formats.
- Damage: scissor per rectangle, as GLES does, and `VK_KHR_incremental_present`
  later for presentation.
- Dmabuf: `VK_EXT_external_memory_dma_buf` and the modifier extension, directly.

### B: `wgpu`

Portable API over Vulkan, Metal, DX12 and GL. Facts: shaders are WGSL natively and
`naga` is its compiler. Dmabuf import and external semaphores are not in the public
API, so they go through `wgpu-hal` (`Device::as_hal`, `Texture::from_raw`), which is
`unsafe` and tied to the `wgpu-hal` version.

- Pros: safe resource model, WGSL needs no step to SPIR-V, depth and render targets
  are first class, far less code for the offscreen slice.
- Cons: the compositor does not need portability. The two hard parts (dmabuf in,
  explicit sync and KMS presentation out) are the ones it hides. D12 already
  rejected an own `wgpu` renderer for this reason.
- Risk (judgement): lock-in to `wgpu-hal`'s internals and release cadence, and
  `wgpu` choosing the device and queues, which a DRM path wants to control.
- Depth, blur, damage: all expressible. Per-frame scissors need no hal access.

### C: `vulkano`

Safe wrapper with its own resource and sync tracking. Facts: it has a safe-ish
device, buffer and image model, and its shader macro compiles GLSL with `shaderc`
(C++), though a raw SPIR-V `ShaderModule` can be loaded instead. [INFERENCE: its
external-memory fd import exists but is `unsafe`, and I have not exercised it.]

- Pros: fewer lifetime and layout mistakes than `ash`, and a reasonable API.
- Cons: heavier dependency, its own tracking model between us and the driver,
  and the same `unsafe` escape for dmabuf. Smaller user base than `ash` or
  `wgpu` (judgement).
- Risk (judgement): API churn between releases, and the tracked-resource model
  fights a compositor that must import buffers owned by clients.

### Shader production

| Route                   | Dependency    | Notes                                              |
| ----------------------- | ------------- | -------------------------------------------------- |
| `naga` WGSL to SPIR-V   | pure Rust     | Already a workspace dependency, needs `spv-out`.   |
| `shaderc` or `glslang`  | C++ build     | Needs a C++ toolchain, which the workspace avoids. |
| Checked-in `.spv` files | none at build | Stale against the WGSL and unreviewable as diff.   |

Judgement: compile from WGSL at startup with `naga`. D12 says every shader is
written once in WGSL and compiled per backend, user shaders must be compiled at
runtime whichever route is taken, and the same sources then feed both renderers.
Startup compilation also makes a shader failure a named startup error. Nothing
generated is checked in.

## Choice for slice 1: A, `ash` with `naga`

Reasons, in order:

1. The first features after this slice are dmabuf import, explicit sync and KMS
   presentation. They are the reason to have a Vulkan renderer at all and `ash`
   exposes them without an escape hatch (fact for A and B, `unsafe` for C).
2. The shaders already exist as WGSL written for Vulkan's clip convention, so the
   step to SPIR-V is a dependency feature, not a new language.
3. The dependency cost is small and pure Rust: `ash` plus the `spirv` crate that
   `naga` pulls for `spv-out`. `ash` is the only crate that sees the driver.
4. `unsafe` is concentrated in `render/vulkan/` behind RAII wrappers.

The cost is more code than `wgpu` would need. Judgement: acceptable, because the
code that would be saved is the code the later slices have to take back.

### Containing a change of mind

Nothing outside `render/vulkan/` names an `ash` type. The renderer's public surface
is the same four methods as the other two (`new`, `draw`, `capture`, `resize`) over
an `OutputFrame`, and callers dispatch on the existing `TargetRenderer` enum. A
`wgpu` or `vulkano` renderer would replace the module and keep that surface. The
WGSL sources, the uniform packing and the parity tests stay as they are.

### Default

The `vulkan` cargo feature of `gnoblin-backend` is on by default because `ash` is
loaded at run time: a machine with no Vulkan loader builds and runs, and every
Vulkan test prints a skip message instead of failing. `compositor.renderer = auto`
keeps today's behaviour and does not choose Vulkan.

## Out of scope for slice 1

In priority order:

1. Dmabuf import of client buffers, and dmabuf scan-out buffers.
2. KMS presentation, explicit sync and `VK_KHR_incremental_present`.
3. The D18 depth pass: a layer with a `Model` is skipped with a warning.
4. Blur, shadow and user-shader passes (slice 2 takes blur).
5. Buffer transforms other than `Normal`, which GLES does not draw either.
6. A texture cache across frames. Slice 1 uploads each surface every frame.
7. Making Vulkan the `auto` choice.
