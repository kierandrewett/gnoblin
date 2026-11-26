# The Vulkan renderer: production design

Status: design, not implemented. Written against `origin/main` at `46bb3a397` (the `InfraFix` lane). This file becomes
`crates/gnoblin-render-vulkan/DESIGN.md` when the first lane lands.

Owner goal: "put some real effort into the Vulkan renderer. it needs to be prod ready", and "Vulkan is the bare
minimum". Every window effect must draw on Vulkan. The software renderer may lack some.

## 1. Decisions in one page

| #   | Decision                                                                                                                        | Section |
| --- | ------------------------------------------------------------------------------------------------------------------------------- | ------- |
| 1   | Nested sessions present through the host's `zwp_linux_dmabuf_v1` with the existing GBM presenter. No Vulkan WSI swapchain.      | 4.1     |
| 2   | DRM scan-out keeps `gnoblin-backend-drm`'s own GBM ring and flip state machine. Vulkan imports the buffers by modifier.         | 4.2     |
| 3   | A finished frame is handed on as a `sync_file`: `IN_FENCE_FD` on KMS, implicit fence on a dmabuf, CPU wait as the last resort.  | 4.3     |
| 4   | One `VulkanDevice` per physical GPU, shared by every output on it. Each output owns only its target and frame ring.             | 8.1     |
| 5   | Two frames in flight per output, tracked with one timeline semaphore per device. `draw` no longer waits for the GPU.            | 8.5     |
| 6   | Client textures persist across frames, keyed by surface and commit, and only the buffer damage is uploaded.                     | 5.1     |
| 7   | Effects follow one fixed paint order and CSS semantics, written once in the shared WGSL. Colour-only grades run on the CPU too. | 6       |
| 8   | Golden pictures move to `gnoblin-render-frame`. The first renderer to draw a case writes it; the other must match it.           | 10.1    |
| 9   | `auto` becomes a chain: vulkan, then gles, then software. A CPU Vulkan device (lavapipe) is never chosen by `auto`.             | 9       |
| 10  | A lost device is rebuilt by the backend, with bounded retries. `auto` moves down the chain after repeated loss.                 | 8.6     |
| 11  | Explicit sync is `linux-drm-syncobj-v1` through Smithay's module, with the release point signalled when the frame retires.      | 5.4     |
| 12  | A device without `VK_EXT_image_drm_format_modifier` (lavapipe) reads and writes LINEAR dmabufs as buffers, with a copy.         | 5.2     |

## 2. Where the renderer stands

The crate draws an `OutputFrame` offscreen through `ash`, compiles the shared WGSL to SPIR-V with `naga`, clips rounded
corners, runs shell shader passes and reads the picture back (`crates/gnoblin-render-vulkan/src/lib.rs:1-110`).

### 2.1 What is missing for production

| Gap                                                                                   | Evidence                                                                                              |
| ------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| No backdrop blur and no filter blur (shadow, border, background, grades landed in V2) | `RendererKind::effect_kinds` has no `blur`, `crates/gnoblin-backend/src/render/renderer.rs:88-94`     |
| No present, no dmabuf                                                                 | `SurfaceSource::Dmabuf` is skipped, `gnoblin-render-frame/src/frame.rs:173-184`                       |
| Not selectable on DRM                                                                 | `crates/gnoblin-compositor/src/backend.rs:236-246` refuses everything but `auto` and `gles`           |
| Every frame waits for the GPU                                                         | `FrameRecorder::submit` waits the fence, `src/frame.rs:298-313`                                       |
| Every client buffer is uploaded whole, every frame                                    | a new `VkImage` per surface per frame, `src/frame.rs:620-680`                                         |
| One `vkAllocateMemory` per image                                                      | `VkContext::allocate`, `src/context.rs:315-329`; no sub-allocation                                    |
| One `VkDevice` per output                                                             | `Vulkan::with_sources` creates its own context, `src/lib.rs`                                          |
| No 3D layers, no View pictures, no magnifier or colour matrix, no supersampling       | `plan.rs:16-20`, `renderer.rs:96-101`, `frame.rs:692-696`                                             |
| Device choice ignores the display                                                     | `choose` ranks by device type only, `src/context.rs:492-553`                                          |
| Nobody rebuilds a lost renderer                                                       | `RenderError::Lost` is only recorded, `renderer.rs:263-266`, `gnoblin-backend-drm/src/scanout.rs:366` |
| No pipeline cache on disk                                                             | the `GraphicsContext` cache lives in memory only, `src/pipeline.rs:281-291`                           |

### 2.2 Gaps the GLES reference has too

The design fixes these for both GPU renderers, because "parity with GLES" means nothing where GLES is wrong.

- The blur ignores `saturation` and `noise`: `blur.wgsl` grades brightness, contrast and tint only.
- The blur ignores `downscale` and says so once in the log (`gnoblin-render-gles/src/lib.rs:1625-1634`).
- The blur golden was written at full resolution with the default `downscale` of 0.5 ignored, so it changes when
  either renderer honours it (`gnoblin-render-gles/src/golden.rs:106-113`).
- Inset shadows, `background` and non-backdrop `filter` entries are not drawn anywhere (`cascade.rs:660-760`).
- A client `wl_shm` buffer is uploaded whole every frame (`import_shm_buffer(buffer, None, &[])`, `gles/src/lib.rs:1042-1045`).
- On DRM the frame is never read back, so screenshots and screen casts of a hardware output are refused
  (`docs/backends.md`, row "A frame read back as pixels").
- `wl_surface.set_buffer_transform` is not drawn by any renderer. It stays out of scope here.

### 2.3 Stale docs found on the way

- `docs/backends.md` and `crates/gnoblin-backend/parity.md` say dmabuf import is "supported: GLES and Vulkan". Vulkan
  imports nothing.
- `crates/gnoblin-schema/schema/tree/seat.toml:163-166` says Vulkan draws no kind, then says `shader` is drawn by a
  renderer that draws it. The code lists `shader` for Vulkan.
- `crates/gnoblin-backend/parity.md:103` says nested keeps a `Software` field. Nested uses `AnyRenderer`.

## 3. Shape of the crate after this design

```text
VulkanDevice (one per physical GPU, Rc)
├── VkContext          instance, device, queue, extensions, limits, DRM node
├── Allocator          sub-allocated device memory, dedicated imports
├── Pipelines          shipped passes, user-shader worker, on-disk pipeline cache
├── TextureCache       client textures by content key, shared by all outputs
├── Timeline           one timeline semaphore; every submit signals the next value
└── RetireQueue        objects released when the timeline passes their value

VulkanOutput (one per output, owned by the backend's target)
├── RenderTarget       persistent RGBA8 image the frame is composed in
├── FrameRing[2]       command buffer, descriptor arena, uniform ring per frame in flight
└── Saved              pixels under the cursor, for captures that leave it out
```

`Vulkan` stays the public name of the per-output object so callers change little. `Vulkan::new(physical)` remains as a
convenience that opens a private device, for tests and the headless default.

### 3.1 File ownership inside the crate

| File                                                   | Owns                                                        | Lane                          |
| ------------------------------------------------------ | ----------------------------------------------------------- | ----------------------------- |
| `context.rs`                                           | instance, device choice, extension and feature negotiation  | V1                            |
| `memory.rs`                                            | the allocator and the footprint it reports                  | V1                            |
| `sync.rs`                                              | timeline, retire queue, frame ring                          | V1                            |
| `textures.rs`                                          | the texture cache and damaged uploads                       | V1                            |
| `frame.rs`                                             | recording one frame from a plan                             | V1, hooks for V2 to V4        |
| `plan.rs`                                              | pure planning of draws and jobs                             | V2, then V3                   |
| `pipeline.rs`                                          | pipelines, samplers, descriptor arenas, pipeline cache file | V1, entries added by V2 to V4 |
| `blur.rs`, `scratch.rs`                                | backdrop and filter blur, scratch image pool                | V3                            |
| `subtree.rs`, `spatial.rs`, `view.rs`                  | View pictures, 3D layers, magnifier and colour matrix       | V4                            |
| `dmabuf.rs`, `formats.rs`, `external.rs`, `capture.rs` | import, export, sync files, copies                          | V5                            |

## 4. Presentation

### 4.1 Nested on a Wayland host

The nested backend already presents two ways (`crates/gnoblin-backend/src/nested/present_dmabuf.rs:1-40`):

- read the frame back and write it into a `wl_shm` slot;
- allocate a buffer with GBM on the host's main device, have the renderer copy its frame into it
  (`AnyRenderer::copy_frame_into_dmabuf`), and attach that buffer.

Vulkan joins the second path by implementing `copy_frame_into_dmabuf` (lane V5) and answering `dmabuf_support`
(lane B1). `GbmPresenter::can_fill` then accepts it with no change of its own. The shared-memory path stays as the
fallback the presenter already falls back to.

How the copy works:

1. The GBM buffer is imported once per slot as a `VkImage` with its explicit modifier and plane layouts
   (`VkImageDrmFormatModifierExplicitCreateInfoEXT`), dedicated memory, `COLOR_ATTACHMENT` usage.
2. Each present records a full-buffer draw from the target into the slot image. It is a draw, not `vkCmdBlitImage`,
   because a modifier image is not guaranteed the blit feature, while GBM's `RENDERING` flag means it can be rendered to.
3. The image is released to `VK_QUEUE_FAMILY_FOREIGN_EXT`, the submit signals a binary semaphore exported as a
   `sync_file`, and the file is attached to the dmabuf with `DMA_BUF_IOCTL_IMPORT_SYNC_FILE`. The host's renderer then
   waits for it implicitly.
4. The CPU never waits. The slot is reused only after the host releases its `wl_buffer`, which the presenter tracks.

Why not `VK_KHR_wayland_surface`:

- A WSI swapchain takes buffer attachment, release and pacing away from the nested loop, which today drives both
  paths from the host's frame callbacks.
- The dmabuf route is the same mechanism as DRM scan-out and dmabuf capture. One import path, tested once, serves
  three uses.
- It works for every renderer that can fill a dmabuf, so the presenter stays renderer-neutral.

### 4.2 DRM and KMS scan-out

`gnoblin-backend-drm` does not use Smithay's `DrmCompositor`. It allocates two XRGB8888 GBM buffers per output with the
plane's modifiers, imports them into a per-output GLES renderer, composes into a persistent target and copies it over
the buffer being flipped (`scanout.rs:1-17`, `scanout.rs:257-442`). The flip state machine is pure and tested
(`present.rs`). Vulkan slots into that structure; nothing above it changes.

| Step           | Today (GLES)                                 | With Vulkan                                                      |
| -------------- | -------------------------------------------- | ---------------------------------------------------------------- |
| Allocate       | GBM, `RENDERING \| SCANOUT`, plane modifiers | the same, modifiers intersected with what Vulkan can render into |
| Import         | `Gles::import_scanout` (EGLImage)            | `Vulkan::import_scanout`: explicit-modifier `VkImage`            |
| Compose        | `Gles::draw_scanout`, damage into the target | `Vulkan::draw_scanout`, damage into the target                   |
| Copy to buffer | blit with rows reversed (GL is bottom-up)    | the present draw, no flip (Vulkan is top-down)                   |
| Supersampling  | `draw_scanout_downsampled`                   | the present draw runs `downsample.wgsl` with the flip flag off   |
| Hand to KMS    | flip, implicit sync                          | flip with `IN_FENCE_FD`; see 4.3                                 |
| Damage hint    | none (`damage_clips: None`, `gpu.rs:962`)    | `FB_DAMAGE_CLIPS` from the slot's accumulated damage             |

`scanout.rs` grows a renderer enum, `ScanRenderer { Gles(Gles), Vulkan(Vulkan) }`, behind the crate's new `vulkan`
feature. `Device` keeps the GBM device and gains an optional `Rc<VulkanDevice>`; the GLES probe context becomes a probe
of whichever renderer the session uses.

Per-slot damage: the target is persistent, so the copy only needs what changed since this slot was last written.

- Each slot keeps the union of the damage of the frames it missed. With two slots, that is the last frame's damage
  plus this one's.
- The draw covers that union, and the same rectangles go to `FB_DAMAGE_CLIPS`.
- A pure `SlotDamage` type holds the rule and is tested on its own, like `present.rs`.

Formats: XRGB8888 maps to `VK_FORMAT_B8G8R8A8_UNORM` with the alpha write masked off. A plane whose modifiers Vulkan
cannot render to leaves the output to GLES under `auto`, and refuses an explicit `vulkan` naming the modifiers.

### 4.3 Handing a finished frame on

| Consumer                             | First choice                                       | Fallback                                                |
| ------------------------------------ | -------------------------------------------------- | ------------------------------------------------------- |
| KMS, atomic, plane has `IN_FENCE_FD` | `PlaneConfig.fence` = exported `sync_file`         | none needed                                             |
| KMS, legacy or no `IN_FENCE_FD`      | wait the frame's timeline value before `page_flip` | the same; this is the only CPU wait on the present path |
| Host compositor (nested)             | `DMA_BUF_IOCTL_IMPORT_SYNC_FILE` on the dmabuf     | CPU wait before `attach` on kernels before 6.0          |
| Capture client (dmabuf)              | `IMPORT_SYNC_FILE` on the client's dmabuf          | CPU wait before the frame's `ready` event               |

Smithay's DRM surface already writes `IN_FENCE_FD` when a fence is passed (`smithay/src/backend/drm/surface/atomic.rs:1254`).
Smithay's own compositor skips `IN_FENCE_FD` on NVIDIA because commits fail there; D1 copies that rule.

## 5. Client buffers

### 5.1 Shared memory with damage

The frame does not say which part of a buffer changed, so both GPU renderers upload whole buffers every frame. Lane C3
adds one field to `Surface`:

```rust
/// Identifies the surface's current content so a renderer can keep its texture across frames.
pub content: Option<ContentKey>,

pub struct ContentKey {
    pub surface: u64,           // stable per wl_surface (and per snapshot)
    pub commit: u64,            // increases on every commit that attaches or damages
    pub damage: Arc<[Rect]>,    // buffer-coordinate damage of that commit, since the previous one
}
```

The texture cache is keyed by `surface`, as wlroots does, because surface damage is relative to the surface's previous
content and not to the buffer's. The rules:

- Same `commit` as cached: no upload.
- Next `commit`, same size: upload the damage rectangles only, through the staging ring.
- Gap in commits, a new size or no key: upload the whole buffer.
- A surface not drawn for 120 frames, or destroyed, gives its texture back through the retire queue.

A snapshot (closing window) is keyed by its `Arc<Image>` pointer and uploaded once. The cache is per device, so a
window spanning two outputs is uploaded once.

Formats: the session's `wl_shm` advertises `argb8888` and `xrgb8888` only (`crates/protocols/wl-shm/src/lib.rs:39`). The
renderer keeps `B8G8R8A8_UNORM` and uses an image view swizzle `A = ONE` for `xrgb8888` instead of a separate pipeline.

### 5.2 dmabuf import

What Vulkan advertises, built once per device in `formats.rs`:

| DRM fourcc                       | `VkFormat`                        | Note                                             |
| -------------------------------- | --------------------------------- | ------------------------------------------------ |
| `ARGB8888`                       | `B8G8R8A8_UNORM`                  |                                                  |
| `XRGB8888`                       | `B8G8R8A8_UNORM`, swizzle `A=ONE` |                                                  |
| `ABGR8888`                       | `R8G8B8A8_UNORM`                  |                                                  |
| `XBGR8888`                       | `R8G8B8A8_UNORM`, swizzle `A=ONE` |                                                  |
| `ARGB2101010`, `XRGB2101010`     | `A2R10G10B10_UNORM_PACK32`        |                                                  |
| `ABGR2101010`, `XBGR2101010`     | `A2B10G10R10_UNORM_PACK32`        |                                                  |
| `ABGR16161616F`, `XBGR16161616F` | `R16G16B16A16_SFLOAT`             | drawn as stored values; no colour management yet |
| `RGB565`                         | `R5G6B5_UNORM_PACK16`             |                                                  |

For each pair the device lists in `VkDrmFormatModifierPropertiesListEXT`, a modifier is offered when:

- its features include `SAMPLED_IMAGE` and `SAMPLED_IMAGE_FILTER_LINEAR`;
- `vkGetPhysicalDeviceImageFormatProperties2` with the modifier and `DMA_BUF_BIT` external handle answers `IMPORTABLE`;
- the maximum extent covers `MAX_VIRTUAL_EDGE`, or the extent is recorded and checked per buffer.

`DRM_FORMAT_MOD_INVALID` (an implicit modifier) is never offered, because Vulkan cannot import one. Multi-plane
modifiers (Intel CCS, AMD DCC with metadata planes) are offered when the driver lists them; each plane's fd, offset and
pitch go into the explicit plane layouts, and disjoint planes get `VK_IMAGE_CREATE_DISJOINT_BIT` with one memory each.

Feedback:

- `main_device` is the device's render node, read from `VK_EXT_physical_device_drm` as a `dev_t`.
- There is one tranche, the main device's, because Gnoblin has no direct scan-out of client buffers.
- The table feeds the existing `DmabufSupport` (`crates/gnoblin-backend/src/dmabuf.rs:34-130`), so the protocol layer
  is unchanged.

Import, in `dmabuf.rs`:

1. `DmabufSupport::check` (size, plane count, offered pair) runs first, as for GLES.
2. Create the image with the explicit modifier, import each plane's fd with `VkImportMemoryFdInfoKHR`, dedicated
   allocation, bind.
3. Cache the image on the dmabuf's identity, as GLES caches its texture on the buffer. A refused import is
   `DmabufImportError::Refused` before the client may use the buffer, never a black window.
4. `y_inverted` sets a negative v scale, as `gles/src/dmabuf.rs:99-112` does.
5. Each use acquires the image from `VK_QUEUE_FAMILY_FOREIGN_EXT` and releases it back after the frame.

Devices without `VK_EXT_image_drm_format_modifier`: lavapipe (Mesa 25.3.6, measured on this machine with `vulkaninfo`)
has `VK_EXT_external_memory_dma_buf` but not the modifier extension. Such a device offers LINEAR only.

It imports the fd as a `VkBuffer` and copies it into a cached texture, with `bufferRowLength` set from the plane's
pitch. A writable LINEAR target (scan-out, capture) is written the same way in reverse. This keeps lavapipe testable in
CI without pretending it can sample a tiled buffer.

### 5.3 Multi-plane YUV

GLES offers no YUV: formats importable only as `GL_TEXTURE_EXTERNAL_OES` are left out (`gles/src/dmabuf.rs:8-12`).
Production parity therefore needs none. Lane Y1 adds `NV12` and `P010` through `VkSamplerYcbcrConversion` with
immutable samplers, one pipeline per conversion, as an improvement over GLES that video players benefit from. It does
not block the production gate.

### 5.4 Synchronisation with clients

Implicit sync is the default and must work, because most clients use it:

- Before sampling a client dmabuf, export its write fence with `DMA_BUF_IOCTL_EXPORT_SYNC_FILE` (read access), import
  it as a temporary binary semaphore (`VK_EXTERNAL_SEMAPHORE_HANDLE_TYPE_SYNC_FD_BIT`) and wait on it at the fragment
  stage of the frame's submit.
- On a kernel before 6.0 (no export ioctl), `poll()` the dmabuf fd for `POLLIN` before recording. That is a CPU stall
  per busy buffer, logged once.
- After writing a dmabuf others read (scan-out, nested present, capture), import the frame's `sync_file` into it, as 4.3
  says.

Explicit sync is `linux-drm-syncobj-v1`, using Smithay's `wayland::drm_syncobj` module (present in the pinned revision):

- Served only when the backend's device has a DRM node and `supports_syncobj_eventfd` is true. Lavapipe sessions do not
  serve it.
- Acquire points use Smithay's `DrmSyncPointBlocker`: the commit waits on an eventfd until the point signals, so the
  renderer never waits on an acquire point itself.
- Release points are signalled from the CPU when the timeline passes the value of the last frame that sampled the
  buffer. The backend reports that through a new `Backend::buffers_retired` hook fed by the retire queue.
- An explicit-sync surface skips the implicit export ioctl for its buffers.

Explicit sync matters most for NVIDIA's proprietary driver, whose implicit sync support is limited `[INFERENCE]`.

## 6. Effects

Every effect below is written once in `gnoblin-render-frame/src/shader/wgsl/` and compiled by `naga` to GLSL ES 3.10
for GLES and to SPIR-V for Vulkan. Colour-only operations also exist as one Rust function in `gnoblin-render-frame`, so a
solid colour is graded on the CPU and the software renderer has a reference.

### 6.1 Paint order of one layer

The order is fixed by kind. Within one kind, list order is paint order (later on top), which is what `Effects` already
promises (`gnoblin-render/src/effect.rs:9-11`). That differs from CSS `box-shadow`, where the first shadow is on top; the
goldens depend on the current rule, so it stays.

1. Outer shadows.
2. Backdrop blur, with its grade, tint and grain, clipped to the layer's rounded shape.
3. Background.
4. Inset shadows.
5. Content: the client buffer, or the solid colour, or the shader passes that replace it. Then the filter grade.
6. Borders.
7. The layer's tint and opacity, and its corner clip, over everything from step 3 on. Steps 1 to 6 sit in one group
   when the layer has a backdrop blur or a filter blur; otherwise each draw carries the opacity itself. An outer shadow
   carries the opacity but not the tint, so a faded window fades its shadow, as in CSS.

### 6.2 What each effect means

| Effect             | Shape it follows                        | Meaning                                                                                    |
| ------------------ | --------------------------------------- | ------------------------------------------------------------------------------------------ |
| Outer shadow       | silhouette (`frame: true`) or the layer | as today: full strength at the edge, fading outwards over `blur`; `spread` grows the shape |
| Inset shadow (new) | inside the layer's rounded shape        | the outer falloff reflected: full strength at the edge, fading inwards over `blur`         |
| Border             | silhouette                              | as today: a band of signed distance, inside or outside                                     |
| Backdrop blur      | the layer's rounded shape               | Gaussian of what was drawn below, then saturate, brightness, contrast, tint, grain         |
| Background (new)   | the layer's rounded shape               | one colour filling the window rectangle under the content                                  |
| Filter grade (new) | the layer's own pixels                  | saturate, brightness, contrast of everything the layer paints                              |
| Filter blur (new)  | the layer's own pixels and margin       | Gaussian of everything the layer paints, spilling past its edge                            |
| Shader             | the layer, clipped by the wrapper       | as today                                                                                   |

Corner radius applies everywhere: the shape of shadows (outer and inset), borders, the backdrop blur clip, the
background, the group composite and the shader wrapper all use the layer's `corner_radius` and `silhouette`.

#### Inset shadow

- Hole: the layer's shape moved by `offset`, shrunk by `spread` on every side, with radius `max(r - spread, 0)`.
- Strength at a point inside the shape: `smoothstep(-blur, 0, sdf_hole(p))`. It is 1 on and outside the hole's edge and
  falls to 0 at `blur` pixels inside it.
- Drawn as `colour * opacity * strength`, multiplied by the shape's antialiased coverage, so nothing leaks past a
  rounded corner.
- New WGSL file `inset_shadow.wgsl`. `params` = offset x, offset y, blur, spread, all physical.

CSS paints an inset shadow below the content. A window whose client is opaque therefore hides it, except where the
client is translucent. That follows the owner's "as CSS does" rule; a shell that wants a glow over the content uses a
border or a shader.

Neither shadow is a CSS Gaussian (CSS reaches half strength at the edge with sigma `blur / 2`). Changing that would
redraw every shadow golden and every shell's look, so the existing falloff stays and the inset one mirrors it.

#### Background

`Pass::Background { colour }` fills the layer's shape (silhouette, else `dst`) with its corner radius, below the
content and above the backdrop blur. A translucent background therefore tints a blurred backdrop, as in CSS. It uses the
existing solid pass; no new shader.

#### Grades

`Pass::Filter { blur, saturate, brightness, contrast }` grades the layer. The backdrop blur's grade uses the same
function. The order is fixed: saturate, then brightness, then contrast (the style already multiplies like kinds
together, `cascade.rs:674-707`, so list order is not kept).

On straight sRGB values, clamped to 0..1 after each step, then premultiplied again:

| Step         | Formula                                                               |
| ------------ | --------------------------------------------------------------------- |
| saturate s   | CSS `feColorMatrix type="saturate"`: luma weights 0.213, 0.715, 0.072 |
| brightness b | `c * b`                                                               |
| contrast k   | `(c - 0.5) * k + 0.5`                                                 |

No transfer function is applied, as for every other pass (`gles/src/lib.rs:68-74`); browsers also grade CSS filter
functions on sRGB values.

Where a grade runs:

- Solid colours (background, borders, shadows, solid layers) are graded on the CPU by
  `gnoblin_render_frame::grade::apply`, so they are exact and the software renderer gets them for free.
- Content pixels are graded in the fragment stage by `gnoblin_grade` in the preamble.
- A node whose subtree is several layers (a window with a titlebar child) passes its grade down to each layer, the way
  opacity is passed down today. That is exact unless translucent children overlap, the same limit opacity has.

#### Grain

`noise` adds `(h(x, y) - 0.5) * noise * 0.0625` to each colour channel after the grade, before premultiplying. `h` is a
PCG integer hash of the pixel's physical position relative to the layer's top-left, so the grain moves with the window
instead of swimming under it, and every renderer and every frame draw the same grain.

#### Backdrop blur

As GLES does today, plus the two things it skips:

- `downscale` d: the copied region is reduced to `ceil(size * d)` by successive halvings with a bilinear tap, then
  blurred at radius `radius * d`, then sampled back up with linear filtering when composed.
- Grade, tint and grain apply once, in the final write at the layer's full resolution, never on the small image. With
  `downscale` 1 that write is the last vertical sweep; the earlier sweeps stay ungraded, as now. The grain's origin is the
  layer's top-left.

The blurred backdrop is clipped by the layer's rounded shape. When the layer's opacity is 1, Vulkan draws the blur
straight onto the target and the content over it; that is pixel-equal to GLES's group and saves a full-size image.

#### Filter blur

CSS blurs everything the element paints. Vulkan and GLES do the same for one layer:

1. Steps 1 to 6 of 6.1 are drawn into a group image the size of the layer's painted extent, grown by the blur's reach.
2. The group is blurred with the same sweeps as the backdrop blur: the radius means what a backdrop blur's does (sigma is
   half of it), in one pass. A backdrop blur on the same layer stays outside the filtered picture.
3. The result is composed with the layer's opacity. It is not clipped by the corner radius; it spills past the edge,
   as in CSS.

A node with several layers needs one picture of all of them. The composer makes it with the window-picture machinery of
the window-node design (`crates/gnoblin-render/DESIGN.md` on `js-WindowNode`, section 3.6 step 4).

The subtree is drawn as a private picture and shown as one `SurfaceSource::View` layer, which then takes the filter. The
picture is redrawn only when the subtree is dirty, so a still blurred window costs one blur per change.

### 6.3 Shared WGSL and uniform changes

| Change                                                                                                    | Files                                                           |
| --------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| `grade: vec4<f32>` (saturate, brightness, contrast, noise) in `Gnoblin`; block grows from 96 to 112 bytes | `preamble.wgsl`, `uniforms.rs`, GLES and Vulkan `UNIFORM_BYTES` |
| `gnoblin_grade(c)` and `gnoblin_grain(local)` in the preamble                                             | `preamble.wgsl`                                                 |
| Blur's grade moves from `params.zw` into `grade`; saturation and grain added                              | `blur.wgsl`                                                     |
| Surface and solid passes call `gnoblin_grade`; identity grade costs nothing visible                       | `surface.wgsl`, `solid.wgsl`                                    |
| New inset shadow pass                                                                                     | `inset_shadow.wgsl`, `sources.rs`                               |
| A flip flag in `taps.z` so Vulkan (top-down) and GLES (bottom-up) share the downsample pass               | `downsample.wgsl`                                               |
| CPU twins of grade and grain                                                                              | new `gnoblin-render-frame/src/grade.rs`                         |

The uniform stride stays 256 bytes in Vulkan, so the larger block costs nothing there.

### 6.4 The frame and tree contract

| Change                                                                                            | Where                                                                 |
| ------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------- |
| `Shadow.inset: bool`; `Pass::name` is `inset-shadow` when set                                     | `gnoblin-render/src/effect.rs`                                        |
| `Pass::Background { colour }`, `Pass::Filter(FilterParams)`; `NAMES` gains `background`, `filter` | `gnoblin-render/src/effect.rs`                                        |
| Effect kinds `background` and `filter`; `inset` on `shadow` reuses the existing field             | `gnoblin-schema/schema/tree/effect.toml`, `gnoblin-core/src/timed.rs` |
| The tree accepts the new kinds                                                                    | `gnoblin-compositor/src/ipc/tree_effects.rs`                          |
| The style maps inset shadows, `background` and `filter` instead of reporting them `unapplied`     | `gnoblin-std-impl/src/style/cascade.rs`                               |
| `abi.lock`, the catalog hash and the SDKs are regenerated                                         | as the window-node design's core lane does                            |

A renderer that does not draw a new kind names it in the node's `skipped` as `<kind>: unsupported-by-renderer`, through
the existing table (`compose.rs:3358`). The software renderer keeps reporting `filter` blur and backdrop blur; it gains
background, inset shadow and grades cheaply, but that is the software lanes' choice, not this design's.

Out of scope: four different corner radii. The style keeps four (`Radii`, `cascade.rs:225`), but the node has one
`corner_radius`. Supporting four is a node schema change first; the shaders would then take a `vec4` of radii.

## 7. Capture, readback, Views and the window picture

### 7.1 What Vulkan must answer

| Call (`AnyRenderer`)                                       | Today on Vulkan | After                                                           | Lane   |
| ---------------------------------------------------------- | --------------- | --------------------------------------------------------------- | ------ |
| `capture(with_cursor)`                                     | readback, waits | waits only for the frame being read                             | V1     |
| `copy_frame_into_dmabuf`                                   | `Unsupported`   | draw into the client's imported buffer, `sync_file` attached    | V5     |
| `copy_image_into_dmabuf`                                   | `Unsupported`   | upload, then the same draw                                      | V5     |
| `read_dmabuf`                                              | `Unsupported`   | import, copy to a host buffer, wait                             | V5     |
| `dmabuf_capture_support`, `dmabuf_picture_support`         | `None`          | the pairs Vulkan can render into (`COLOR_ATTACHMENT` modifiers) | V5, B1 |
| `view_pixels`, `read_view`, `release_views`, `draws_views` | none            | View pictures on the GPU, as `gles/src/subtree.rs` does         | V4     |
| `downsampled(shown)`                                       | none            | the downsample pass into a shown-size image, read back          | V4     |

### 7.2 Capture on DRM

The Vulkan target on DRM is the composed frame before the scan-out copy, so it can be read. Lane D1 implements
`Backend::capture` and `dmabuf_capture_support` for a Vulkan DRM output from the target. Secure layers are redacted
through the existing `capture_redacted` (`renderer.rs:514-536`).

Screenshots and screen casts then work on real hardware for the first time. The GLES DRM path keeps refusing; making it
read back is a separate GLES change.

### 7.3 Views and the window picture

`gles/src/subtree.rs` draws a `ViewScene` into a texture per View, keeps it until the scene's revision, size or
liveness changes, and draws a capture variant on demand. Vulkan V4 mirrors it:

- One `RGBA8` colour image and one `D24S8` (or `D32S8`) depth-stencil image per View, from the allocator.
- The stencil nests clipping ancestors, as in GLES. The depth buffer serves perspective eyes and model layers.
- `SurfaceSource::View` layers sample the image directly. Nothing is read back for the frame that shows it.
- A View's picture is redrawn only when its revision changes (`an_unchanged_view_is_not_drawn_again` is the test GLES
  has; Vulkan gets the same).

The window picture of the window-node design is this same machinery with an explicit source and rig, so it needs
nothing more from the renderer. Window captures, portal window casts and XR all read it.

### 7.4 XR

XR reads View and window pictures through `ext-image-copy-capture` into `wl_shm` (`extensions/gnoblin-xr/crates/xr-gpu`).
That keeps working with Vulkan unchanged. A dmabuf capture path for XR (the compositor exports, `xr-gpu` imports into the
OpenXR device) is possible once V5 lands, but it is XR's lane and not needed for the renderer to be production-ready.

## 8. Robustness

### 8.1 Device selection

| Backend  | The device is                                                                              | When none matches                                                                               |
| -------- | ------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------- |
| DRM      | the one whose `VK_EXT_physical_device_drm` primary node is the KMS card the backend opened | `auto`: GLES on that card. `vulkan`: see below                                                  |
| Nested   | the one whose primary or render node is the host's dmabuf `main_device`                    | the best device; the host may refuse its buffers, and the presenter falls back to shared memory |
| Headless | the best by type (discrete, integrated, virtual, CPU)                                      | `NoDevice`                                                                                      |

An explicit `vulkan` on DRM with no matching device may use a CPU device (lavapipe) when the LINEAR buffer path of 5.2
can write the scan-out buffer. That is what the vkms CI check runs on. Otherwise it is refused naming the card and every
device's reason.

Multiple GPUs: the DRM backend drives one primary GPU (`gnoblin-backend-drm/src/udev.rs:5-35`), for every renderer.
Outputs on a second GPU are a backend change. The Vulkan side is ready for it, because devices are per GPU (section 3).

### 8.2 Required features

Vulkan 1.2 is the floor, for timeline semaphores, `driverID` and sampler Y′CbCr conversion in core.

| Extension                          | Needed for                          | Missing means                                       |
| ---------------------------------- | ----------------------------------- | --------------------------------------------------- |
| `VK_KHR_external_memory_fd`        | dmabuf import and export            | ineligible for DRM and nested                       |
| `VK_EXT_external_memory_dma_buf`   | the same                            | the same                                            |
| `VK_EXT_image_drm_format_modifier` | tiled and multi-plane buffers       | LINEAR only (5.2); not chosen by `auto`             |
| `VK_KHR_external_semaphore_fd`     | `sync_file` import and export       | CPU waits; not chosen by `auto`                     |
| `VK_EXT_queue_family_foreign`      | ownership transfer of shared images | not chosen by `auto`                                |
| `VK_EXT_physical_device_drm`       | matching the display's device       | cannot match; not chosen by `auto` on DRM or nested |
| `VK_EXT_memory_budget`             | the footprint and the budget        | allocation counts only                              |
| `VK_EXT_device_fault`              | logging why a device was lost       | the loss is logged without detail                   |

### 8.3 Memory

- Device-local images and buffers are sub-allocated with the `gpu-allocator` crate (Vulkan backend, `ash` 0.38). It is
  a new workspace dependency; V1 confirms with `cargo tree` that it resolves to the same `ash` as Smithay.
- Imported and exported dmabuf memory is always a dedicated allocation, outside the allocator.
- Host-visible memory is a staging ring and a readback buffer, trimmed after each frame to what one steady frame needs,
  as today (`docs/gpu-memory.md:129`).
- The footprint adds the allocator's used bytes and, with `VK_EXT_memory_budget`, the heap usage, so the session's GPU
  budget (`crates/gnoblin-wayland/src/gpu_memory.rs`) sees real numbers.
- `release_scratch` frees the blur and filter scratch pool, as GLES does.

A steady session makes no `vkAllocateMemory` call. The test counts them.

### 8.4 Pipelines and descriptors

- Pipeline cache file: `$XDG_CACHE_HOME/gnoblin/vulkan/<pipelineCacheUUID hex>.bin`, loaded at device creation,
  checked against the device's header (vendor, device, UUID) and a Gnoblin header (format version, length, CRC32).
- A file that fails a check is deleted and ignored. It is written atomically (temporary file, then rename) when the
  device is dropped, and after every 16 new pipelines from the worker thread, which owns the cache.
- The file is capped at 32 MiB. `VulkanOptions::pipeline_cache: Option<PathBuf>` lets tests use a temporary directory
  or none.
- User shader pipelines stay keyed by source hash in the 64-entry cache. A shell reload that writes the same source
  finds the pipeline built.
- Descriptor sets: one arena per frame in flight, reset when that frame retires. A cached texture owns a long-lived set
  from a pool with `FREE_DESCRIPTOR_SET`, so a steady frame writes no texture descriptors.

### 8.5 Frame pacing

- Each output has two frame slots. `draw` records, submits with a signal of `timeline = N`, and returns.
- Before a slot is reused, the CPU waits for its value, with the 10-second hang timeout that exists today
  (`frame.rs:68`). In steady state that value has long passed and the wait returns at once.
- `capture` waits only for the value of the frame it reads.
- The retire queue destroys pipelines, textures and images when the timeline passes the value of the last frame that
  used them. That replaces the per-frame `wait_idle` rules in `pipeline.rs:680-686`.
- Present timing stays the backend's: the DRM state machine allows one flip in flight; nested follows the host's
  frame callbacks.

### 8.6 A lost device

`VK_ERROR_DEVICE_LOST` from a submit or a wait marks the device lost; every output on it returns `RenderError::Lost`.
Recovery belongs to whoever owns the renderer: the headless target, the nested target and the DRM `Buffers`.

1. Record the loss for the frame's shaders, which already strikes them (`renderer.rs:263-266`).
2. Log `VK_EXT_device_fault` details when the device has them.
3. Drop the outputs and the device. GBM scan-out buffers and KMS framebuffers survive and are imported again.
4. Create a new device with the same choice, then new outputs. Damage every output whole.
5. Retry after 0 ms, 100 ms and 1 s. Three losses within ten minutes end the retries.
6. After that, `auto` moves to the next renderer in the chain (section 9) and updates `seat.effect_kinds` and
   `seat.renderer`. An explicit choice keeps retrying every 5 s and logs each failure, because an explicit value is
   never second-guessed (`gnoblin-compositor/src/backend_select.rs:20-24`).

The front buffer stays on screen during recovery, so the user sees a frozen frame, not black.

### 8.7 Validation

- Debug builds enable `VK_LAYER_KHRONOS_validation` when installed (exists). Tests also turn on synchronisation
  validation through `VkValidationFeaturesEXT`, because frames in flight and foreign queues are where races hide.
- Every drawing test fails on any message the layer reports (exists).
- The CI renderer job installs the layer and fails when it was not enabled, the way it fails today when no device
  exists (`justfile:127-139`).

### 8.8 Shell reloads

The renderer belongs to the backend and outlives every shell. A reload removes nodes; the caches that hang off them
release on the existing signals:

- View and window pictures leave with the frame's `live_views` list.
- Client textures stay, because clients outlive a reload, and leave when their surface does.
- User shader pipelines stay in the cache, so the first frame after a reload does not recompile.
- The redraw waker is installed by the backend, not the shell, so it survives.

The test reloads the shell ten times and asserts the footprint and allocation count return to their first value.

## 9. Choosing the renderer

### 9.1 The rule

`RendererKind::AUTO` (`renderer.rs:45-54`) is replaced by a pure function in a new `render/select.rs`, in the style of
`backend_select.rs`: inputs are the setting, the backend and a probe report; the output is the renderer and the reason.

| Setting    | Tries, in order        | When all fail                               |
| ---------- | ---------------------- | ------------------------------------------- |
| `auto`     | vulkan, gles, software | software cannot fail where a session starts |
| `vulkan`   | vulkan                 | the session is refused with every reason    |
| `gles`     | gles                   | the session is refused                      |
| `software` | software               | refused only on DRM, as today               |

On DRM, software is not in the `auto` chain, because it cannot scan out.

### 9.2 When Vulkan is eligible for `auto`

All of these must hold:

1. The loader opens and the instance is Vulkan 1.2 or newer.
2. The device matches the backend's device (8.1) and is not a CPU implementation. Lavapipe leaks about 0.8 MiB a frame
   on Mesa 22.3.6 (`tests/gpu-memory-soak/DESIGN.md:71-95`) and is far slower than llvmpipe GLES at blur.
3. Every extension in 8.2 marked "not chosen by `auto`" is present, with `timelineSemaphore` enabled.
4. On DRM, at least one XRGB8888 modifier of the primary plane can be imported as a colour attachment.
5. The driver is not on the deny list: a table of `driverID` and minimum version in `context.rs`, empty at first.
6. The start-up self-test passes: every shipped pipeline builds, and a 16×16 frame with a shadow, a border and a blur
   reads back within tolerance of its expected probes, in under 50 ms.

### 9.3 Reporting

- One `info` log line per decision: `renderer: auto chose gles: vulkan is not eligible: <reason>`.
- Two new read-only seat properties, appended to `seat.toml`: `renderer` (`vulkan`, `gles` or `software`) and
  `renderer_reason` (why `auto` chose it; empty for an explicit choice). `gnoblinctl` shows them with the seat.
- `seat.effect_kinds` is set again whenever the renderer changes, and the composer's table with it
  (`gnoblin-wayland/src/server.rs:491`). The schema comment changes from "set once at startup" to say so.
- A refused explicit choice stays a start error naming the setting, as today (`backend.rs:205-246`).

### 9.4 When Vulkan enters `auto`

Lane S1 ships the chain as gles, then software, with Vulkan present but gated off. Lane F1 puts Vulkan first when all
of these are true:

- Every lane in section 11 except Y1 has landed.
- Every golden case passes on Vulkan (lavapipe) and GLES (llvmpipe) in CI, validation and sync validation clean.
- The DRM VM checks pass with `renderer = "vulkan"` on all four distributions.
- The performance budgets of 10.5 hold in CI.
- A one-hour soak on each owner-named hardware machine (question 2) shows no growth and no device loss.

## 10. Tests

### 10.1 Goldens shared by both GPU renderers

- The golden scenes move from `gnoblin-render-gles/src/golden.rs` to `gnoblin-render-frame/src/golden.rs` behind a new
  `test-support` feature, and the files move to `gnoblin-render-frame/tests/golden/`.
- Each renderer runs `golden::check_all(name, draw)` in its own tests. A machine without the device skips loudly, as
  now.
- `GNOBLIN_UPDATE_GOLDEN=gles` or `=vulkan` writes the files from that renderer. The first renderer to draw a new case
  writes it; the second must match. A disagreement is settled by section 6, not by whichever is newer.
- Tolerance stays 3 per channel for shapes and becomes 4 for blur cases, because the two drivers round bilinear taps
  differently. Each case states its own.

New cases, each a 32×24 scene like the existing five:

| Case                         | Proves                                                    |
| ---------------------------- | --------------------------------------------------------- |
| `border`                     | inside and outside rings, offset                          |
| `corners`                    | shadow, border, background and blur all follow radius 4   |
| `inset_shadow`               | the inward falloff and the corner clip                    |
| `background`                 | the fill under a translucent window, under the content    |
| `filter_grade`               | saturate, brightness, contrast on content and solid parts |
| `filter_blur`                | the spill past the edge                                   |
| `backdrop_grade`             | saturation and grain on a backdrop blur                   |
| `blur_downscale`             | `downscale` 0.5 and 0.25                                  |
| `colour_matrix`, `magnifier` | the frame's View and matrix pass                          |
| `supersample`                | the downsample pass, both modes                           |

Each case also carries renderer-neutral probes in the same module: for example the inset shadow's edge pixel equals
the shadow colour over the background, and `filter_grade` of a known colour equals `grade::apply` of it. A wrong golden
written by the first renderer is then caught.

A GPU test in each renderer draws 256 colours through `gnoblin_grade` and compares with `grade::apply` within 1 level.

### 10.2 Vulkan-only tests

| Test                                                                                                                      | Lane   |
| ------------------------------------------------------------------------------------------------------------------------- | ------ |
| `an_idle_frame_submits_nothing`, `a_steady_frame_uploads_nothing`, `a_steady_session_allocates_no_device_memory`          | V1     |
| `damage_uploads_only_the_damaged_rows`                                                                                    | V1     |
| `two_outputs_share_one_texture_for_one_surface`                                                                           | V1     |
| `a_frame_returns_before_the_gpu_finishes` (a slow fault, then a capture that waits)                                       | V1     |
| `a_corrupt_pipeline_cache_is_ignored_and_replaced`                                                                        | V1     |
| `a_texture_is_destroyed_only_after_its_last_frame_retires`                                                                | V1     |
| `a_shadow_does_not_leave_the_damage`, `a_blur_at_the_edge_repeats_the_edge` (ported from GLES)                            | V2, V3 |
| `a_tilted_layer_matches_gles`, `a_view_is_drawn_once_per_revision`                                                        | V4     |
| `a_linear_dmabuf_imports_and_draws`, `a_refused_dmabuf_is_an_error_not_a_crash`, `an_inverted_dmabuf_reverses_the_v_axis` | V5     |
| `a_writer_fence_is_waited_before_sampling` (a second queue writes late; the readback never shows the old colour)          | V5     |
| `a_frame_copied_into_a_dmabuf_reads_back_equal`                                                                           | V5     |

Where a dmabuf comes from in a test, tried in order, and the CI recipe fails if none worked:

1. `/dev/udmabuf` through Smithay's `allocator::udmabuf` (LINEAR, memfd-backed).
2. A LINEAR dmabuf exported by a second Vulkan device of the same driver.
3. GBM on a render node, where one exists (the VM and real hardware).

### 10.3 Backend and session tests

| Test                                                                           | Where                                                 | Lane |
| ------------------------------------------------------------------------------ | ----------------------------------------------------- | ---- |
| the nested session presents through the host's dmabuf with Vulkan              | `gnoblin-wayland/tests/nested_dmabuf.rs`              | B1   |
| a GPU capture of a Vulkan output equals the shared-memory capture              | `gnoblin-wayland/tests/dmabuf_capture_gpu.rs`         | B1   |
| a lost device is rebuilt and the next frame is whole (fault injection)         | `gnoblin-backend/src/headless.rs` tests               | S1   |
| three losses move `auto` to GLES and update `effect_kinds`                     | same                                                  | S1   |
| `auto` explains every refusal (pure `select.rs` table tests)                   | `gnoblin-backend/src/render/select.rs`                | S1   |
| ten shell reloads leave the footprint where it started                         | new `gnoblin-wayland/tests/render_reload.rs`          | V1   |
| `SlotDamage` accumulates what a slot missed                                    | `gnoblin-backend-drm/src/scanout.rs` tests            | D1   |
| the conformance table's Vulkan rows hold                                       | `gnoblin-backend-drm/src/conformance.rs`, `parity.md` | D1   |
| explicit sync: a commit waits for its acquire point; release follows the frame | new `gnoblin-wayland/tests/syncobj.rs`                | X1   |

### 10.4 The DRM VM checks

The `drm-vm` job boots four distributions under KVM with a 2D `virtio-vga` (`scripts/ci/vm/run.sh:12-16`). Lane CI1
adds checks; each is a new row in `run.sh`, listed in `EXPECTED_FAILURES` until D1 lands, as that script's convention
requires.

| Check         | Setup                                                      | Passes when                                                                                     |
| ------------- | ---------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `vulkan-foot` | `compositor.renderer = "vulkan"`, lavapipe in the guest    | the session serves, foot runs, the QMP screendump shows the expected background and window      |
| `auto-reason` | `auto`, lavapipe only                                      | the log says `auto chose gles: vulkan is not eligible: the only device is a CPU implementation` |
| `vkms-vulkan` | `modprobe vkms`, `GNOBLIN_DRM_DEVICE` set to the vkms card | 120 flips complete; a `grim` screenshot (capture on DRM, 7.2) matches the scene                 |

Guest packages: `mesa-vulkan-drivers` (Debian, Fedora), `vulkan-swrast` (Arch), `libvulkan_lvp` (openSUSE). Whether
each distribution's kernel ships `vkms` is checked by the lane with `modinfo vkms`; a distribution without it skips
`vkms-vulkan` with a logged reason.

### 10.5 Performance budgets

Counters (`Vulkan::stats`: submits, bytes uploaded, device allocations, pipelines built) are always on and cheap.

| Budget                                                                                                         | Where checked                             |
| -------------------------------------------------------------------------------------------------------------- | ----------------------------------------- |
| Idle frame: 0 submits, 0 uploads, 0 allocations                                                                | unit tests, every CI run                  |
| Steady 10-window frame: 0 uploads, 0 allocations                                                               | unit tests                                |
| CPU time to record and submit a 1920×1080 frame of 10 windows with shadows and borders: under 2 ms on lavapipe | `just renderer-perf`, CI                  |
| Vulkan wall time per frame at most 1.25× GLES (llvmpipe) on the same scene, median of 20                       | `just renderer-perf`, CI                  |
| GPU time, same scene plus one default blur on a 1000×700 window: under 1.5 ms on an integrated GPU             | owner hardware, timestamp queries         |
| Frames in flight per output: at most 2                                                                         | `a_frame_returns_before_the_gpu_finishes` |
| Footprint within 10% of GLES for the same scene                                                                | `gpu_memory` tests                        |

### 10.6 CI changes

All on GitHub-hosted runners. No a37 box runs in CI.

| File                                       | Change                                                                                                                |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------- |
| `scripts/ci-install-system-libraries.sh`   | add the validation layers (`vulkan-validationlayers`, `vulkan-validation-layers`) and `vulkan-tools` per distribution |
| `justfile` `renderer-test`                 | fail when Vulkan validation was not enabled or a dmabuf test found no source; run the shared goldens                  |
| `justfile`                                 | new `renderer-perf`: both GPU renderers on one runner, the budgets of 10.5                                            |
| `.github/workflows/matrix.yml` `renderer`  | `sudo modprobe udmabuf` before the tests; a `renderer-perf` job                                                       |
| `scripts/ci/vm/run.sh`, `guest-session.sh` | the checks of 10.4, guest packages, the renderer setting in the guest config                                          |

If Ubuntu 24.04's lavapipe lacks something a test needs, the `renderer` job for `vulkan` moves into the Fedora 43
container the `distro` job already builds, instead of the test being narrowed.

## 11. Lanes

Sizes are working days for one agent: S is 1 to 2, M is 3 to 5, L is 6 to 10.

### 11.1 Table

| Lane | Work                                                                                                        | Owns (exact files)                                                                                                                                                                                                                          | Depends on                                    | Size |
| ---- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------- | ---- |
| C1   | Effects contract: inset, background, filter kinds                                                           | `gnoblin-render/src/effect.rs`; `gnoblin-schema/schema/tree/effect.toml`; `gnoblin-core/src/timed.rs`; `gnoblin-compositor/src/ipc/tree_effects.rs`; `gnoblin-std-impl/src/style/cascade.rs`; abi and SDK outputs                           | none                                          | M    |
| C2   | Shared WGSL, uniforms, grade twins, golden module move                                                      | `gnoblin-render-frame/src/shader/wgsl/*`, `shader/uniforms.rs`, `shader/sources.rs`, new `grade.rs`, new `golden.rs`, `tests/golden/*`; the `UNIFORM_BYTES` constants in both GPU crates; delete `gnoblin-render-gles/src/golden.rs`        | none                                          | M    |
| C3   | Content key and buffer damage on `Surface`                                                                  | `gnoblin-render-frame/src/frame.rs` (`Surface`); `gnoblin-backend/src/render/compose/layers.rs` and the compose code that builds surfaces; struct literals in tests                                                                         | none                                          | S    |
| V1   | Device and output split, frames in flight, allocator, texture cache, pipeline cache, stats                  | `gnoblin-render-vulkan/src/{context,memory,sync,textures,frame,lib,error}.rs`; `pipeline.rs` (cache and arenas only); `Cargo.toml`; workspace `Cargo.toml` for `gpu-allocator`                                                              | C3 for damage uploads                         | L    |
| V2   | Shadow, inset shadow, border, background, filter grades                                                     | `gnoblin-render-vulkan/src/plan.rs`; new pipeline entries in `pipeline.rs`; golden cases `border`, `corners`, `inset_shadow`, `background`, `filter_grade`                                                                                  | C1, C2                                        | M    |
| G1   | GLES completion: saturation, grain, downscale, inset, background, filter grades and blur                    | `gnoblin-render-gles/src/lib.rs`, `program.rs`, `downsample.rs`                                                                                                                                                                             | C1, C2                                        | L    |
| V3   | Backdrop blur with downscale, grades, tint, grain; filter blur; scratch pool                                | new `gnoblin-render-vulkan/src/{blur,scratch}.rs`; blur jobs in `plan.rs` and `frame.rs`; golden cases `blur_downscale`, `backdrop_grade`, `filter_blur`                                                                                    | V1, V2                                        | L    |
| V4   | Views on the GPU, 3D layers, magnifier and colour matrix, supersampling                                     | new `gnoblin-render-vulkan/src/{subtree,spatial,view,downsample}.rs`; the model path in `plan.rs` (after V3 merges)                                                                                                                         | V1                                            | L    |
| V5   | dmabuf import and export, format table, sync files, capture copies                                          | new `gnoblin-render-vulkan/src/{dmabuf,formats,external,capture}.rs`; the extension list constant in `context.rs`                                                                                                                           | V1                                            | L    |
| B1   | Backend wiring: shared device, Vulkan arms for dmabuf, capture and Views, nested presenter, device matching | `gnoblin-backend/src/render/renderer.rs` (`AnyRenderer` impls); `nested/present_dmabuf.rs`; `nested/surface.rs`; `headless.rs`; `gnoblin-backend/Cargo.toml`                                                                                | V5; V4 for Views                              | M    |
| D1   | DRM scan-out, `IN_FENCE_FD`, damage clips, capture on DRM, DRM loss recovery                                | `gnoblin-backend-drm/src/{scanout,gpu,lib,conformance}.rs`, `Cargo.toml`; the refusal in `gnoblin-compositor/src/backend.rs:236-246`; `crates/gnoblin-backend/parity.md`                                                                    | V5, B1                                        | L    |
| S1   | Selection chain, eligibility, reasons, recovery, seat properties                                            | new `gnoblin-backend/src/render/select.rs`; `RendererKind` in `renderer.rs`; `gnoblin-compositor/src/backend.rs` (build); `gnoblin-schema/schema/tree/seat.toml`; `gnoblin-wayland/src/server.rs`; `gnoblin-config/schema/config.toml` docs | B1; D1 for DRM                                | M    |
| X1   | `linux-drm-syncobj-v1`                                                                                      | new `gnoblin-wayland/src/glue/wp_linux_drm_syncobj_v1.rs`; protocol list and access entries; `Backend::buffers_retired` in `gnoblin-backend/src/lib.rs`                                                                                     | V5, B1                                        | M    |
| CI1  | CI and VM checks                                                                                            | `.github/workflows/matrix.yml`, `justfile`, `scripts/ci-install-system-libraries.sh`, `scripts/ci/vm/*`                                                                                                                                     | none to start; D1 to remove expected failures | M    |
| P1   | Performance budgets and the soak                                                                            | new `gnoblin-render-vulkan/tests/perf.rs`; `justfile` `renderer-perf` (coordinated with CI1); `tests/gpu-memory-soak/DESIGN.md`                                                                                                             | V1, V3                                        | S    |
| Y1   | YUV dmabufs (optional, beyond GLES)                                                                         | new `gnoblin-render-vulkan/src/ycbcr.rs`; format rows in `formats.rs`                                                                                                                                                                       | V5                                            | M    |
| F1   | Gate, Vulkan first in `auto`, docs sweep, move this file                                                    | `select.rs` (one line); `docs/backends.md`, `docs/guides/window_effects.md`, `docs/gpu-memory.md`, `docs/custom-shaders.md`, this design                                                                                                    | all but Y1                                    | S    |

### 11.2 Order

1. Wave 1, in parallel: C1, C2, C3, V1, CI1 (infrastructure rows only).
2. Wave 2: V2 and G1 (after C1 and C2), V4 and V5 (after V1).
3. Wave 3: V3 (after V2 merges, because both edit `plan.rs`), B1 (after V5), X1 (after B1).
4. Wave 4: D1, then S1 and P1.
5. Wave 5: F1, after the gate of 9.4.

Shared files and who goes first: `plan.rs` is V2, then V3, then V4's model path. `pipeline.rs` is V1's cache first;
V2 to V4 only add entries. `renderer.rs` is B1's `AnyRenderer` impls, then S1's `RendererKind`. Each later lane rebases.

### 11.3 Acceptance per lane

- **C1**: `tree_ipc.rs` accepts `background`, `filter` and `shadow` with `inset`; style tests map inset shadows,
  `background` and `filter` grades and blur to effects; no `unapplied` text for them; GLES, software and Vulkan list
  the new kinds in `skipped` as `unsupported-by-renderer`; `abi.lock` and `gen-sdks --check` pass.
- **C2**: the five existing goldens pass on GLES from the new location; the grade twin test passes on GLES; the
  downsample pass with the flag on is pixel-equal to today.
- **C3**: every surface layer the composer emits for a client buffer has a key; a commit with damage carries it; a
  snapshot has a stable key; `compose/tests.rs` covers all three.
- **V1**: every test in 10.2 marked V1; the existing 38 Vulkan tests still pass with sync validation on; `Vulkan::new`
  still works for callers.
- **V2**: the V2 golden cases pass on Vulkan (and on GLES where G1 has landed); `effect_kinds` for Vulkan lists
  `shadow`, `border`, `background`, `inset-shadow`, `filter` (grade only until V3), `shader`.
- **G1**: every golden case of section 10.1 except the View cases passes on GLES; the downscale warning is gone.
- **V3**: blur goldens pass on Vulkan within 4; `blur` joins `effect_kinds`; the ported edge and damage tests pass.
- **V4**: `draws_views` is true for Vulkan; `view_gpu_tests.rs` and `view_secure_tests.rs` pass with Vulkan;
  `unsupported-3d` is no longer logged for Vulkan.
- **V5**: the V5 tests of 10.2 pass in CI with a dmabuf source; the format table test lists only importable pairs.
- **B1**: the 10.3 rows marked B1 pass; `headless.rs:1614` (Vulkan has no dmabuf capture) is turned around.
- **D1**: `drm-vm` `vulkan-foot` and `vkms-vulkan` pass on every distribution; conformance rows `render.vulkan` and
  "a frame read back as pixels" say supported for Vulkan.
- **S1**: the 10.3 rows marked S1 pass; `seat.renderer` and `seat.renderer_reason` are readable through `gnoblinctl`.
- **X1**: `syncobj.rs` passes in the VM; the protocol is not offered on lavapipe.
- **CI1**: the renderer job fails on a machine without validation or without a dmabuf source; the new VM checks run.
- **P1**: the CI budgets of 10.5 pass; the measurements on owner hardware are recorded in this file.
- **Y1**: an NV12 buffer draws within 2 levels of a CPU conversion.
- **F1**: the gate list of 9.4 is ticked with links to runs; `auto` picks Vulkan on a matching GPU.

### 11.4 Working on a box

Lanes build and test on an a37 box: 12 GB disk, 4 GB RAM, 2 CPUs, no `/dev/dri` (a gVisor sandbox, as
`gnoblin-wayland/tests/dmabuf_gpu.rs:12-16` records).

Check the box for lavapipe before starting:

```sh
ls /usr/share/vulkan/icd.d/ | grep lvp          # the lavapipe ICD file
vulkaninfo --summary | grep -E 'deviceName|driverInfo'
ls /usr/share/vulkan/explicit_layer.d/ | grep -i validation
```

If any is missing, install `mesa-vulkan-drivers libvulkan1 vulkan-tools vulkan-validationlayers` (Debian or Ubuntu)
or `mesa-vulkan-drivers vulkan-loader vulkan-tools vulkan-validation-layers` (Fedora).

Build one crate at a time with the limits the box can hold:

```sh
export CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 CARGO_PROFILE_TEST_DEBUG=line-tables-only
cargo test --locked -p gnoblin-render-vulkan -- --nocapture 2>&1 | tee /tmp/vk.log
grep -E 'no Vulkan device|no Vulkan loader' /tmp/vk.log && echo 'lavapipe missing: tests skipped'
```

What a box cannot prove: DRM, GBM, render-node dmabufs and udmabuf. Lanes V5, B1, D1, X1 and CI1 get that evidence by
pushing their branch; `matrix.yml` runs on a push to any branch that touches `crates/**`. A lane reports the run URL in
its result.

### 11.5 Risks

| Risk                                                                         | Lane   | Mitigation                                                                                                    |
| ---------------------------------------------------------------------------- | ------ | ------------------------------------------------------------------------------------------------------------- |
| Lavapipe cannot import a dmabuf it did not export `[INFERENCE]`              | V5, D1 | probe first; the second dmabuf source of 10.2; the VM uses GBM dumb buffers                                   |
| Ubuntu 24.04's Mesa lavapipe lacks an extension the tests need               | CI1    | move the Vulkan renderer job into the Fedora 43 container                                                     |
| A distribution kernel lacks `vkms`                                           | CI1    | `modinfo` check; skip with a logged reason on that distribution only                                          |
| llvmpipe and lavapipe round blur taps differently past tolerance 4           | V3, G1 | per-case tolerance; probes carry the meaning                                                                  |
| Blur reads outside the damage and picks up last frame's glass `[INFERENCE]`  | V3, G1 | a test with a changing layer under glass; fix in compose's damage if it fails                                 |
| NVIDIA rejects `IN_FENCE_FD` commits                                         | D1     | Smithay's rule: no `IN_FENCE_FD` on NVIDIA, CPU wait instead                                                  |
| `gpu-allocator` pulls a second `ash`                                         | V1     | `cargo tree -d`; a small block allocator in `memory.rs` if it does                                            |
| The uniform block growing to 112 bytes breaks a user shader's expectations   | C2     | user shaders read `GnoblinUser`, a separate block; the preamble is private                                    |
| The window-picture machinery is late, so multi-layer filter blur waits on it | C1     | single-layer filter blur ships first; the multi-layer path is the window-node lane's dependency, listed there |

## 12. Decisions taken and the one open input

1. Once the gate of 9.4 passes, `auto` on real hardware chooses Vulkan before GLES (decided: the owner set Vulkan as the
   bare minimum and asked for it to be production-ready). Until the gate passes, `auto` keeps its current order.
2. NVIDIA with the proprietary driver is best effort: it gets the CPU-wait path of D1 and its failures are fixed when
   reported, but it is not part of the release gate.
3. Open input from the owner: which machines make up the hardware gate (at least one AMD and one Intel GPU). It is needed
   only when lane F1 runs; nothing before F1 waits for it.
