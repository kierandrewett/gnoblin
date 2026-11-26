#![allow(unsafe_code)]
//! Recording a frame, submitting it and reading the target back.
//!
//! A frame is recorded into one [`InFlightFrame`](super::sync::InFlightFrame) of the output, submitted once, and not waited for:
//! `draw` returns when the frame is on the queue. The output has two slots, so a second frame can be recorded while the
//! first runs, and the slot a frame needs is waited for only when its previous frame has not finished, which in a steady
//! session it has. What a frame in flight reads is kept alive until the timeline passes its value (`sync.rs`).
//!
//! The exception is a frame that draws a shell's shader: its cost is what the watchdog needs to see, so that frame is waited
//! for and timed. A capture waits for its own submission, and so for every frame before it.
//!
//! # A frame, in order
//!
//! 1. **A slot.** The frame's slot is waited for (its previous frame has finished), reset, and begun.
//! 2. **Shader pipelines.** Every shader pass of the frame gets its pipeline (`ShaderPipelines`), made
//!    now if its source has not been drawn before. A source that cannot be drawn is reported on the
//!    frame's timings and its passes are left out.
//! 3. **Textures.** The texture cache (`textures.rs`) makes every surface layer's texture ready: the one it holds when the
//!    surface's commit is the one it has, the damaged rectangles of a new commit, the whole buffer otherwise. The overlay and
//!    the cursor are made the same way, and a solid layer with a shader pass has a one-pixel texture of its colour; those have
//!    no key and are made for the frame alone. A buffer that cannot be read, or an image the device cannot allocate, is
//!    logged and its layer skipped: the rest of the frame is drawn.
//! 4. **Images of the shader layers.** A layer whose pass reads the backdrop gets an image of its
//!    size to draw the backdrop into, and a layer with several passes gets two scratch images; a
//!    layer whose images cannot be made is drawn without its passes.
//! 5. **Plan.** With the textures known, the frame, each backdrop and each pass of a chain
//!    become lists of draws (`plan.rs`), joined into one set of buffers.
//! 6. **Uploads and shader jobs**, from the bottom layer up: the copies that update textures, then a layer's backdrop is
//!    drawn (the layers below it, secure ones black, over its rectangle) and the passes of its chain, each into its own render
//!    pass and then made readable. A layer reads the backdrops of those below it, so the order is the layers'.
//! 7. **Pass A.** The damage is cleared to the background and the layers are drawn, each
//!    clipped to the damage rectangle it belongs to.
//! 8. **Overlay.** The overlay is drawn over the layers. The cursor's pixels are copied into
//!    a host buffer first so a capture can leave the cursor out, then the cursor is drawn.
//! 9. **Submit**, which signals the next timeline value. The frame's own images go to the retire queue with that value.
//!
//! # Layouts
//!
//! The target lives in `COLOR_ATTACHMENT_OPTIMAL` between frames. A copy out of it moves it to
//! `TRANSFER_SRC_OPTIMAL` and back with explicit barriers; the render pass has the matching
//! subpass dependencies. A texture goes `UNDEFINED`, `TRANSFER_DST_OPTIMAL`,
//! `SHADER_READ_ONLY_OPTIMAL` and stays there; a cached texture that is updated goes `SHADER_READ_ONLY_OPTIMAL`,
//! `TRANSFER_DST_OPTIMAL` and back. A backdrop or scratch
//! image goes `UNDEFINED`, `COLOR_ATTACHMENT_OPTIMAL` (the render pass draws into it) and
//! `SHADER_READ_ONLY_OPTIMAL` by a barrier after the pass; a scratch image a later pass of the same
//! chain draws into again is moved back by a barrier that waits for the reads of the pass before.
//! Every frame begins with a memory barrier from all earlier work to all later work, so the frames in flight never race on
//! the target, a texture or the pixels saved under the cursor.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ash::vk;
use gnoblin_render::{Pass, ShaderKey};
use gnoblin_types::{Color, Rect, Scale, Size};

use super::Vulkan;
use super::context::{TARGET_FORMAT, VkContext};
use super::error::classify;
use super::memory::{VkBuffer, VkImage, colour_range};
use super::pipeline::{BLOCK_RANGE, RetiredPipeline, USER_BLOCK_RANGE, UserPipeline};
use super::plan::{
    ChainStep, DrawBatch, FramePlan, PassKind, PlanTarget, PlannedDraw, ShaderUse, Shaders, TextureSizes, TextureSlot,
    UNIFORM_STRIDE, chain, damage_of, overlay_of, plan, plan_into, quad_of,
};
use super::spatial::{DepthClass, DepthTarget, QuadBase, Replace};
use super::subtree::{EnsureOutcome, SceneTextures, ViewRequest, plan_scene, scene_surfaces, scene_views};
use super::sync::{InFlightFrame, STAMP_QUERIES, TimestampQuery};
use super::textures::{CacheFrame, Prepared, TextureSource, Wanted, colour_layers, write_texture_set};
use super::view::{self, WholePass};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{BackdropInput, Layer, OutputFrame, Surface, SurfaceSource};
use gnoblin_render_frame::image::Image;

use crate::log;

/// How many output-sized uploads the staging buffer keeps room for between frames. A frame of a few full-screen
/// windows fits; a larger frame grows the buffer for itself and gives it back after the frame.
pub(super) const STAGING_SCREENS: usize = 4;

/// What the renderer draws into: an image and the framebuffer over it.
pub(super) struct RenderTarget {
    context: Rc<VkContext>,
    pub(super) image: VkImage,
    pub(super) framebuffer: vk::Framebuffer,
}

impl RenderTarget {
    /// A target of `size` pixels. Its pixels are undefined until [`Vulkan::clear_target`].
    pub(super) fn new(context: &Rc<VkContext>, pass: vk::RenderPass, size: Size) -> Result<Self, vk::Result> {
        Self::with_usage(
            context,
            pass,
            size,
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::SAMPLED
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST,
        )
    }

    /// An image a frame draws into and then samples: a backdrop, or a scratch image of a shader chain.
    fn scratch(context: &Rc<VkContext>, pass: vk::RenderPass, size: Size) -> Result<Self, vk::Result> {
        Self::with_usage(
            context,
            pass,
            size,
            vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
        )
    }

    fn with_usage(
        context: &Rc<VkContext>,
        pass: vk::RenderPass,
        size: Size,
        usage: vk::ImageUsageFlags,
    ) -> Result<Self, vk::Result> {
        let image = VkImage::new(context, size.width, size.height, TARGET_FORMAT, usage, false)?;
        let views = [image.view()];
        let info = vk::FramebufferCreateInfo::default()
            .render_pass(pass)
            .attachments(&views)
            .width(size.width)
            .height(size.height)
            .layers(1);
        // SAFETY: the view is of an image that has the attachment usage and the pass's format.
        let framebuffer = unsafe { context.device.create_framebuffer(&info, None) }?;
        Ok(Self {
            context: Rc::clone(context),
            image,
            framebuffer,
        })
    }
}

impl Drop for RenderTarget {
    fn drop(&mut self) {
        // SAFETY: the owner waited for the last frame that drew into it, or retired it until that frame had finished.
        unsafe { self.context.device.destroy_framebuffer(self.framebuffer, None) };
    }
}

/// The pixels under the cursor, saved so a capture can leave it out.
#[derive(Default)]
pub(super) struct Saved {
    pub(super) buffer: Option<VkBuffer>,
    /// The rectangle of the output the buffer holds, packed rows of it.
    pub(super) rect: Option<Rect>,
}

impl Saved {
    /// The bytes of the saved rectangle, packed.
    fn bytes(&self) -> Option<&[u8]> {
        let rect = self.rect?;
        self.buffer
            .as_ref()?
            .read(0, rect.width as usize * rect.height as usize * 4)
    }
}

/// How many layers of a frame may read a backdrop.
///
/// A backdrop is drawn from the layers below it, and one of those may read its own, so the backdrops nest. Three deep is what
/// the GLES renderer allows; a layer past that is drawn without the passes that read one.
const MAX_BACKDROP_LAYERS: usize = 3;

/// The code of a pass left out because its backdrop is not there: the layer is past [`MAX_BACKDROP_LAYERS`], the backdrop's
/// image could not be made, or the pass was not marked as reading the backdrop its source reads.
const BACKDROP_UNAVAILABLE: &str = "backdrop-unavailable";

/// The code of the passes left out because the layer's scratch images could not be made.
const IMAGE_FAILED: &str = "image-failed";

/// The timestamps a frame has written around its shader draws.
#[derive(Default)]
struct Stamps {
    /// The queries used so far.
    used: u32,
    /// The key each pair of queries belongs to, and the first query of the pair.
    spans: Vec<(ShaderKey, u32)>,
    /// Whether a draw did not fit the pool, so the numbers are partial.
    overflowed: bool,
}

impl Stamps {
    /// Records `draw`, between two timestamps when it is a shader pass's and the pool has room.
    ///
    /// Both are written when everything before them has finished, so the pair measures the draw alone, not what ran before it.
    fn bracket(
        &mut self,
        device: &ash::Device,
        cmd: vk::CommandBuffer,
        query: Option<&TimestampQuery>,
        key: Option<ShaderKey>,
        draw: impl FnOnce(),
    ) {
        let (Some(query), Some(key)) = (query, key) else {
            draw();
            return;
        };
        if self.used + 2 > STAMP_QUERIES {
            self.overflowed = true;
            draw();
            return;
        }
        let first = self.used;
        // SAFETY: the pool holds `STAMP_QUERIES` queries, which `begin` reset, and both indices are below it.
        unsafe { device.cmd_write_timestamp(cmd, vk::PipelineStageFlags::BOTTOM_OF_PIPE, query.pool, first) };
        draw();
        // SAFETY: as above.
        unsafe { device.cmd_write_timestamp(cmd, vk::PipelineStageFlags::BOTTOM_OF_PIPE, query.pool, first + 1) };
        self.spans.push((key, first));
        self.used += 2;
    }
}

/// The images a layer's shader passes read: its backdrop, and the two scratch images its passes alternate between.
struct LayerImages {
    /// What is behind the layer, drawn into an image of the layer's size, with the frame that describes it.
    backdrop: Option<(RenderTarget, BackdropInput)>,
    /// The images the passes before the last draw into, alternating.
    chain: [Option<RenderTarget>; 2],
}

/// A render pass a frame records before its own.
enum Job {
    /// A View's scene, drawn into the picture the store holds, which later passes sample.
    View {
        colour: vk::Image,
        framebuffer: vk::Framebuffer,
        size: Size,
        plan: FramePlan,
    },
    /// The layers below a layer, drawn over its rectangle into its backdrop image.
    Backdrop {
        index: usize,
        plan: FramePlan,
        background: Color,
    },
    /// One pass of a layer's chain, drawn into one of its scratch images.
    Chain { index: usize, step: ChainStep },
}

/// The view pass of a frame: the scratch image the finished output is copied into and the pass that draws it back.
struct ViewPass {
    image: vk::Image,
    view: vk::ImageView,
    pipeline: vk::Pipeline,
    pass: WholePass,
}

/// What `paint` hands to `record`.
struct FrameRecording<'a> {
    frame: &'a OutputFrame,
    planned: &'a FramePlan,
    batch: DrawBatch,
    prepared: &'a Prepared,
    images: &'a BTreeMap<usize, LayerImages>,
    jobs: Vec<Job>,
    pipelines: &'a [(ShaderKey, vk::Pipeline)],
    /// The pictures of the Views the frame samples, by View.
    views: Vec<(u64, vk::ImageView)>,
    /// The view pass, when the frame has a magnifier or colour matrix that changes a pixel.
    filter: Option<ViewPass>,
    /// The framebuffer over the output's target and its depth-stencil image, when the frame has layers a matrix places and
    /// the device can draw them.
    models: Option<vk::Framebuffer>,
    /// Whether the frame only draws View pictures: its target, cursor and overlay are not touched.
    views_only: bool,
}

/// What a render pass draws into.
#[derive(Clone, Copy)]
struct PassTarget {
    framebuffer: vk::Framebuffer,
    size: Size,
    /// Whether the framebuffer has a depth-stencil image, which makes the pass [`DepthExtras::render_pass`].
    depth: bool,
}

impl PassTarget {
    /// A target with colour only.
    fn plain(framebuffer: vk::Framebuffer, size: Size) -> Self {
        Self {
            framebuffer,
            size,
            depth: false,
        }
    }
}

/// How an image rests between passes, and what a copy out of it must wait for and hand back.
#[derive(Clone, Copy)]
pub(super) struct Resting {
    layout: vk::ImageLayout,
    stage: vk::PipelineStageFlags,
    /// The access that wrote it last.
    written: vk::AccessFlags,
    /// The access the next pass makes of it.
    restored: vk::AccessFlags,
}

impl Resting {
    /// An image the renderer draws into: a target.
    pub(super) fn attachment() -> Self {
        Self {
            layout: vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            stage: vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            written: vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            restored: vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
        }
    }

    /// An image a frame samples: a View's picture.
    pub(super) fn sampled() -> Self {
        Self {
            layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            stage: vk::PipelineStageFlags::FRAGMENT_SHADER,
            written: vk::AccessFlags::SHADER_READ,
            restored: vk::AccessFlags::SHADER_READ,
        }
    }
}

/// What a draw binds: the uniform set, the textures by slot and the shader pipelines by key.
struct Bindings<'a> {
    uniform: vk::DescriptorSet,
    textures: &'a [(TextureSlot, vk::DescriptorSet)],
    pipelines: &'a [(ShaderKey, vk::Pipeline)],
}

/// A clear of some rectangles of the target to a colour.
struct Clear<'a> {
    colour: [f32; 4],
    rects: &'a [Rect],
}

/// What a submitted frame keeps in use until its timeline value is reached: the images made for it alone.
struct FrameObjects {
    _transient: Vec<VkImage>,
    _images: BTreeMap<usize, LayerImages>,
}

impl Vulkan {
    /// The slot the frame being recorded uses.
    pub(super) fn slot(&self) -> &InFlightFrame {
        self.ring.slot(self.active)
    }

    /// The host-visible bytes the renderer holds: the slots' buffers, the readback and the pixels saved under the cursor.
    pub(super) fn host_bytes(&self) -> u64 {
        self.ring
            .bytes()
            .saturating_add(self.readback.size() as u64)
            .saturating_add(
                self.cursor_under
                    .buffer
                    .as_ref()
                    .map_or(0, |buffer| buffer.size() as u64),
            )
    }

    /// Gives back the host-visible memory a past peak left behind, keeping what one steady frame needs: a readback of the
    /// target, and staging for [`STAGING_SCREENS`] output-sized uploads.
    ///
    /// The staging buffer a frame in flight reads goes to the retire queue with that frame's value. The readback is idle:
    /// a capture waits for its own submission.
    pub(super) fn trim(&mut self) {
        let target = self.physical.width as usize * self.physical.height as usize * 4;
        let readback = target.next_power_of_two();
        if self.readback.size() > readback
            && let Ok(small) = VkBuffer::new(
                &self.device.context,
                super::sync::MIN_BUFFER,
                vk::BufferUsageFlags::TRANSFER_DST,
                true,
            )
        {
            self.readback = small;
        }
        let slot = self.ring.slot_mut(self.active);
        if let Some(old) = slot.trim(readback.saturating_mul(STAGING_SCREENS)) {
            self.device.retire.borrow_mut().push(slot.value, old);
        }
    }

    /// Clears the target image `image` to transparent black and leaves it ready to draw into. The clear is submitted and not
    /// waited for: the queue runs it before any frame that draws into the target.
    pub(super) fn clear_target(&mut self, image: vk::Image) -> Result<(), vk::Result> {
        self.active = self.ring.acquire();
        self.ring.slot_mut(self.active).begin()?;
        let slot = self.slot();
        slot.barrier(
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            (vk::PipelineStageFlags::TOP_OF_PIPE, vk::AccessFlags::empty()),
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
        );
        let transparent = vk::ClearColorValue { float32: [0.0; 4] };
        // SAFETY: the image is in `TRANSFER_DST_OPTIMAL` after the barrier above.
        unsafe {
            self.device.context.device.cmd_clear_color_image(
                slot.cmd,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &transparent,
                &[colour_range()],
            );
        }
        slot.barrier(
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
        self.ring.slot_mut(self.active).submit().map(|_| ())
    }

    /// Draws `frame` into the target. See [`Vulkan::draw`].
    pub(super) fn paint(&mut self, frame: &OutputFrame, scale: Scale) -> Result<(), vk::Result> {
        let requests: Vec<ViewRequest<'_>> = frame
            .view_scenes()
            .iter()
            .map(|scene| ViewRequest { scene, capture: false })
            .collect();
        self.paint_with(frame, scale, &requests, false)
    }

    /// Draws `frame` into the target and the pictures of `requests` that the device does not hold yet.
    ///
    /// With `views_only` the frame is only the carrier of the pictures: its layers, target, cursor and overlay are left
    /// alone, which is what a read of a View's picture is.
    pub(super) fn paint_with(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        requests: &[ViewRequest<'_>],
        views_only: bool,
    ) -> Result<(), vk::Result> {
        let filtering = !views_only && view::is_active(frame.view(), frame.colour_matrix());
        let widened;
        let frame = if views_only {
            frame
        } else {
            if damage_of(frame, scale, self.physical).is_empty() {
                return Ok(());
            }
            // The magnifier and the colour matrix read and rewrite the whole output: a pixel not repainted would be filtered
            // again, and one left unfiltered after the filter was turned off would stay wrong. So the output is repainted
            // whole whenever the filter is on now or was on the frame before.
            let widen = filtering || self.filtered;
            self.filtered = filtering;
            if widen {
                let mut copy = frame.clone();
                copy.push_damage(Rect::new(0, 0, self.physical.width, self.physical.height));
                widened = copy;
                &widened
            } else {
                frame
            }
        };
        let device = Rc::clone(&self.device);
        // The timeline value this frame signals: the last use of every texture and image it touches.
        let value = device.context.timeline.submitted() + 1;
        let mut drawn: Vec<(u64, bool)> = Vec::new();
        let mut fresh: Vec<ViewRequest<'_>> = Vec::new();
        {
            let mut store = device.views.borrow_mut();
            let mut retire = device.retire.borrow_mut();
            if !views_only {
                let alive: Vec<u64> = frame
                    .live_views()
                    .iter()
                    .copied()
                    .chain(requests.iter().map(|request| request.scene.id()))
                    .collect();
                store.retire_except(&alive, &mut retire);
            }
            for request in requests {
                let outcome = store.ensure(*request, &device.context, device.extras.as_ref(), value, &mut retire);
                if outcome == EnsureOutcome::Draw {
                    drawn.push((request.scene.id(), request.capture));
                    fresh.push(*request);
                }
            }
            for id in sampled_views(frame, requests) {
                store.touch(id, value);
            }
        }
        if views_only && fresh.is_empty() {
            return Ok(());
        }
        self.active = self.ring.acquire();
        self.ring.slot_mut(self.active).begin()?;
        let painted = self.paint_body(frame, scale, &fresh, value, views_only);
        if painted.is_err() && !drawn.is_empty() {
            // The pictures this frame meant to draw were not: the store must not believe they were.
            let mut retire = device.retire.borrow_mut();
            device.views.borrow_mut().invalidate(&drawn, &mut retire);
        }
        painted
    }

    /// The rest of [`Vulkan::paint_with`], once the frame's slot is begun: textures, plans, recording and the submission.
    fn paint_body(
        &mut self,
        frame: &OutputFrame,
        scale: Scale,
        fresh: &[ViewRequest<'_>],
        value: u64,
        views_only: bool,
    ) -> Result<(), vk::Result> {
        let device = Rc::clone(&self.device);
        let (mut shaders, pipelines) = self.resolve_shaders(frame);
        let mut wanted = wanted(frame, &shaders);
        for (number, request) in fresh.iter().enumerate() {
            for (layer, surface) in scene_surfaces(request.scene.eyes(request.capture)) {
                if let Some(source) = surface_source(surface) {
                    wanted.push(Wanted {
                        slot: TextureSlot::Scene(number, layer),
                        source,
                        key: surface.content.as_deref(),
                    });
                }
            }
        }
        let prepared = device.textures.borrow_mut().prepare(
            wanted,
            &mut CacheFrame {
                context: &device.context,
                pipelines: &device.pipelines,
                slot: self.ring.slot_mut(self.active),
                retire: &mut device.retire.borrow_mut(),
                value,
            },
        )?;
        let mut sizes = TextureSizes {
            layers: vec![None; frame.layers().len()],
            ..TextureSizes::default()
        };
        for (slot, size) in &prepared.sizes {
            match *slot {
                TextureSlot::Layer(index) => sizes.layers[index] = Some(*size),
                TextureSlot::Overlay => sizes.overlay = Some(*size),
                TextureSlot::Cursor => sizes.cursor = Some(*size),
                TextureSlot::Chain(..) | TextureSlot::Backdrop(_) | TextureSlot::View(_) | TextureSlot::Scene(..) => {}
            }
        }
        // The pictures of the Views this frame samples, and how big each is.
        let mut views: Vec<(u64, vk::ImageView)> = Vec::new();
        let mut view_sizes: Vec<(u64, (u32, u32))> = Vec::new();
        {
            let store = device.views.borrow();
            for id in sampled_views(frame, fresh) {
                if let Some((view, size)) = store.sampled(id) {
                    views.push((id, view));
                    view_sizes.push((id, size));
                }
            }
        }
        for (index, layer) in frame.layers().iter().enumerate() {
            if let Layer::Surface(Surface {
                source: SurfaceSource::View(view),
                ..
            }) = layer
            {
                sizes.layers[index] = view_sizes.iter().find(|(id, _)| *id == view.id).map(|(_, size)| *size);
            }
        }
        let images = self.make_images(frame, scale, &mut shaders, &sizes);
        let mut batch = DrawBatch::default();
        let mut jobs = Vec::new();
        for (number, request) in fresh.iter().enumerate() {
            let Some(handles) = device.views.borrow().handles((request.scene.id(), request.capture)) else {
                continue;
            };
            let textures = SceneTextures {
                scene: number,
                sizes: &prepared.sizes,
                views: &view_sizes,
            };
            let plan = plan_scene(request.scene.eyes(request.capture), handles.size, &textures).append_to(&mut batch);
            jobs.push(Job::View {
                colour: handles.colour,
                framebuffer: handles.framebuffer,
                size: handles.size,
                plan,
            });
        }
        jobs.extend(self.plan_jobs(frame, scale, &shaders, &sizes, &images, &mut batch));
        let planned = plan(frame, scale, self.physical, &sizes, &shaders).append_to(&mut batch);
        let models = if planned.models_end > planned.layers_end {
            self.depth_framebuffer()
        } else {
            None
        };
        let filter = if !views_only && view::is_active(frame.view(), frame.colour_matrix()) {
            self.make_filter(frame, &mut batch)
        } else {
            None
        };
        if planned.filter_blur && !self.warned_filter_blur {
            self.warned_filter_blur = true;
            log::warn!(
                "unsupported-filter-blur: the Vulkan renderer does not blur a filter yet, so a filter that blurs is drawn graded only"
            );
        }
        let recorded = self.record(FrameRecording {
            frame,
            planned: &planned,
            batch,
            prepared: &prepared,
            images: &images,
            jobs,
            pipelines: &pipelines,
            views,
            filter,
            models,
            views_only,
        });
        match recorded {
            Ok(submitted) => {
                debug_assert_eq!(submitted, value, "the frame signalled the value it was promised");
                // What only this frame used goes when it has finished.
                device.retire.borrow_mut().push(
                    submitted,
                    FrameObjects {
                        _transient: prepared.transient,
                        _images: images,
                    },
                );
                Ok(())
            }
            Err(result) => {
                // The textures this frame meant to update were not uploaded: the cache must not believe they were. The
                // frame's own images go with the last frame submitted, which is no later than the one that failed.
                let mut retire = device.retire.borrow_mut();
                device.textures.borrow_mut().invalidate(&prepared.keys, &mut retire);
                retire.push(
                    device.context.timeline.submitted(),
                    FrameObjects {
                        _transient: prepared.transient,
                        _images: images,
                    },
                );
                Err(result)
            }
        }
    }

    /// The framebuffer over the target and its depth-stencil image, made when a frame first has a layer a matrix places. A
    /// device with no depth-stencil format draws no such layer, which is logged once.
    fn depth_framebuffer(&mut self) -> Option<vk::Framebuffer> {
        let Some(extras) = self.device.extras.as_ref() else {
            if !self.warned_models {
                self.warned_models = true;
                log::warn!(
                    "unsupported-3d: this device has no depth-stencil format, so a layer with a model is not drawn"
                );
            }
            return None;
        };
        if self.depth.is_none() {
            match DepthTarget::over(&self.device.context, extras, &self.target.image) {
                Ok(depth) => self.depth = Some(depth),
                Err(result) => {
                    log::warn!(
                        area = "render",
                        ?result,
                        "the depth image of the output could not be made; 3D layers are not drawn"
                    );
                    return None;
                }
            }
        }
        self.depth.as_ref().map(|depth| depth.framebuffer)
    }

    /// The view pass of a frame with a magnifier or a colour matrix: its block and quad joined to `batch`, and the scratch
    /// image and pipeline it draws with. `None`, logged, when the device cannot make them: the frame is drawn unfiltered.
    fn make_filter(&mut self, frame: &OutputFrame, batch: &mut DrawBatch) -> Option<ViewPass> {
        let device = Rc::clone(&self.device);
        let extras = device.extras.as_ref()?;
        let pipeline = match extras.replacing(&device.pipelines, Replace::View) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                log::warn!(area = "render", %error, "the view pass could not be made; the frame is drawn unfiltered");
                return None;
            }
        };
        if self.scratch.is_none() {
            let made = VkImage::new(
                &device.context,
                self.physical.width,
                self.physical.height,
                TARGET_FORMAT,
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
                false,
            );
            match made {
                Ok(image) => self.scratch = Some(image),
                Err(result) => {
                    log::warn!(
                        area = "render",
                        ?result,
                        "the view pass's copy could not be made; the frame is drawn unfiltered"
                    );
                    return None;
                }
            }
        }
        let scratch = self.scratch.as_ref()?;
        let block = view::ViewBlock::new(frame.view(), frame.colour_matrix(), self.physical).bytes();
        Some(ViewPass {
            image: scratch.raw(),
            view: scratch.view(),
            pipeline,
            pass: view::push_block(batch, &block, self.physical),
        })
    }

    /// Makes the pipeline of every shader pass the frame draws, and says which passes each layer has.
    ///
    /// A source that cannot be drawn (the validator refuses it, it does not compile, or the driver refuses the pipeline) is
    /// reported on the frame's timings with a short code and left out: its node is drawn without the pass. A pass that was not
    /// marked as reading the backdrop but whose source does is refused too, because the frame would not have built one.
    fn resolve_shaders(&mut self, frame: &OutputFrame) -> (Shaders, Vec<(ShaderKey, vk::Pipeline)>) {
        let mut cache = self.device.shaders.borrow_mut();
        cache.begin_frame();
        let mut shaders = Shaders {
            layers: vec![Vec::new(); frame.layers().len()],
            time: self.device.epoch.elapsed().as_secs_f32(),
        };
        let mut found: Vec<(ShaderKey, Result<UserPipeline, &'static str>)> = Vec::new();
        for (index, layer) in frame.layers().iter().enumerate() {
            let effects = match layer {
                Layer::Surface(surface) if surface.model.is_none() => &surface.effects,
                Layer::Solid(solid) if solid.model.is_none() => &solid.effects,
                _ => continue,
            };
            for pass in effects.passes() {
                let Pass::Shader(pass) = pass else { continue };
                let key = pass.key();
                let outcome = match found.iter().find(|(held, _)| *held == key) {
                    Some((_, outcome)) => *outcome,
                    None => {
                        let outcome = cache.resolve(pass);
                        found.push((key, outcome));
                        outcome
                    }
                };
                let outcome = outcome.and_then(|made| {
                    if made.backdrop && !pass.backdrop() {
                        Err(BACKDROP_UNAVAILABLE)
                    } else {
                        Ok(made)
                    }
                });
                match outcome {
                    Ok(made) => shaders.layers[index].push(ShaderUse {
                        key,
                        params: pass.params_padded(),
                        backdrop: made.backdrop,
                    }),
                    Err(code) => frame.shader_timings().record_refused(key, code),
                }
            }
        }
        let pipelines = found
            .into_iter()
            .filter_map(|(key, outcome)| outcome.ok().map(|made| (key, made.pipeline)))
            .collect();
        (shaders, pipelines)
    }

    /// Makes the images the layers with shader passes need: a backdrop for a layer whose pass reads one, and one or two scratch
    /// images for a layer with several passes. A layer whose images cannot be made, or whose backdrop is past
    /// [`MAX_BACKDROP_LAYERS`], loses the passes that need them and is drawn without.
    fn make_images(
        &self,
        frame: &OutputFrame,
        scale: Scale,
        shaders: &mut Shaders,
        sizes: &TextureSizes,
    ) -> BTreeMap<usize, LayerImages> {
        let mut made = BTreeMap::new();
        let mut backdrops = 0;
        for (index, layer) in frame.layers().iter().enumerate() {
            if shaders.layers[index].is_empty() {
                continue;
            }
            let dst = match layer {
                Layer::Surface(surface) => surface.dst,
                Layer::Solid(solid) => solid.dst,
                Layer::Overlay(_) => continue,
            };
            let quad = quad_of(dst, scale);
            if quad.is_empty() || sizes.layers.get(index).is_none_or(Option::is_none) {
                shaders.layers[index].clear();
                continue;
            }
            let mut input = None;
            if shaders.layers[index].iter().any(|pass| pass.backdrop) {
                if backdrops < MAX_BACKDROP_LAYERS {
                    input = frame.backdrop_input(index);
                }
                if input.is_none() {
                    for pass in shaders.layers[index].iter().filter(|pass| pass.backdrop) {
                        frame.shader_timings().record_refused(pass.key, BACKDROP_UNAVAILABLE);
                    }
                    shaders.layers[index].retain(|pass| !pass.backdrop);
                }
            }
            let passes = shaders.layers[index].len();
            if passes == 0 {
                continue;
            }
            match self.layer_images(Size::new(quad.width, quad.height), input, passes) {
                Ok(images) => {
                    backdrops += usize::from(images.backdrop.is_some());
                    made.insert(index, images);
                }
                Err(result) => {
                    log::warn!(
                        area = "render",
                        ?result,
                        "the images of a shader pass could not be made; its layer is drawn without it"
                    );
                    for pass in &shaders.layers[index] {
                        frame.shader_timings().record_refused(pass.key, IMAGE_FAILED);
                    }
                    shaders.layers[index].clear();
                }
            }
        }
        made
    }

    /// The backdrop image of a layer and the scratch images its passes alternate between.
    fn layer_images(&self, size: Size, input: Option<BackdropInput>, passes: usize) -> Result<LayerImages, vk::Result> {
        let render_pass = self.device.pipelines.render_pass;
        let context = &self.device.context;
        let backdrop = match input {
            Some(input) => Some((RenderTarget::scratch(context, render_pass, size)?, input)),
            None => None,
        };
        let mut chain = [None, None];
        for image in chain.iter_mut().take(passes.saturating_sub(1)) {
            *image = Some(RenderTarget::scratch(context, render_pass, size)?);
        }
        Ok(LayerImages { backdrop, chain })
    }

    /// Plans what has to be drawn before the frame, layer by layer from the bottom: the backdrop of a layer (the layers below
    /// it, secure ones black, over its rectangle) and then the steps of its chain. A layer's backdrop reads those of the layers
    /// below it, which is why the order is the layers'.
    fn plan_jobs(
        &self,
        frame: &OutputFrame,
        scale: Scale,
        shaders: &Shaders,
        sizes: &TextureSizes,
        images: &BTreeMap<usize, LayerImages>,
        batch: &mut DrawBatch,
    ) -> Vec<Job> {
        let mut jobs = Vec::new();
        for (index, layer_images) in images {
            if let Some((_, input)) = &layer_images.backdrop {
                let quad = quad_of(input.region, scale);
                let target = PlanTarget {
                    origin: (quad.x, quad.y),
                    size: Size::new(quad.width, quad.height),
                };
                let plan = plan_into(&input.frame, scale, self.physical, target, sizes, shaders).append_to(batch);
                jobs.push(Job::Backdrop {
                    index: *index,
                    plan,
                    background: input.frame.background(),
                });
            }
            for mut step in chain(&frame.layers()[*index], *index, scale, sizes, shaders) {
                step.plan = step.plan.append_to(batch);
                jobs.push(Job::Chain { index: *index, step });
            }
        }
        jobs
    }

    /// A descriptor set from the frame's arena that binds `view` with `sampler` as a texture and its sampler.
    pub(super) fn texture_set(
        &mut self,
        view: vk::ImageView,
        sampler: vk::Sampler,
    ) -> Result<vk::DescriptorSet, vk::Result> {
        let layout = self.device.pipelines.texture_layout;
        let set = self.ring.slot_mut(self.active).arena.allocate(layout)?;
        write_texture_set(&self.device.context, set, view, sampler);
        Ok(set)
    }

    /// Writes a frame's blocks and quads where its passes read them: the slot's uniform, shader-block and instance buffers.
    pub(super) fn upload_batch(&mut self, batch: &DrawBatch) -> Result<(), vk::Result> {
        let context = Rc::clone(&self.device.context);
        let instance_bytes: Vec<u8> = batch.instances.iter().flat_map(|value| value.to_ne_bytes()).collect();
        let slot = self.ring.slot_mut(self.active);
        InFlightFrame::ensure(
            &context,
            &mut slot.uniforms,
            batch.uniforms.len().max(UNIFORM_STRIDE),
            vk::BufferUsageFlags::UNIFORM_BUFFER,
        )?;
        InFlightFrame::ensure(
            &context,
            &mut slot.user_uniforms,
            batch.user.len().max(UNIFORM_STRIDE),
            vk::BufferUsageFlags::UNIFORM_BUFFER,
        )?;
        InFlightFrame::ensure(
            &context,
            &mut slot.instances,
            instance_bytes.len().max(16),
            vk::BufferUsageFlags::VERTEX_BUFFER,
        )?;
        slot.uniforms.write(0, &batch.uniforms);
        slot.user_uniforms.write(0, &batch.user);
        slot.instances.write(0, &instance_bytes);
        Ok(())
    }

    /// The set that binds the slot's two uniform buffers, valid until the slot's next frame.
    pub(super) fn uniform_set(&mut self) -> Result<vk::DescriptorSet, vk::Result> {
        let layout = self.device.pipelines.uniform_layout;
        let uniform_set = self.ring.slot_mut(self.active).arena.allocate(layout)?;
        let slot_uniforms = self.slot().uniforms.raw();
        let slot_user_uniforms = self.slot().user_uniforms.raw();
        let block = [vk::DescriptorBufferInfo::default()
            .buffer(slot_uniforms)
            .offset(0)
            .range(BLOCK_RANGE)];
        let user_block = [vk::DescriptorBufferInfo::default()
            .buffer(slot_user_uniforms)
            .offset(0)
            .range(USER_BLOCK_RANGE)];
        let write_uniforms = [
            vk::WriteDescriptorSet::default()
                .dst_set(uniform_set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .buffer_info(&block),
            vk::WriteDescriptorSet::default()
                .dst_set(uniform_set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .buffer_info(&user_block),
        ];
        // SAFETY: the set and buffers are live and the ranges lie inside the buffers.
        unsafe { self.device.context.device.update_descriptor_sets(&write_uniforms, &[]) };
        Ok(uniform_set)
    }

    /// Records the frame and submits it. Returns the timeline value it signals.
    fn record(&mut self, recording: FrameRecording<'_>) -> Result<u64, vk::Result> {
        let FrameRecording {
            frame,
            planned,
            batch,
            prepared,
            images,
            jobs,
            pipelines,
            views,
            filter,
            models,
            views_only,
        } = recording;
        let device = Rc::clone(&self.device);
        let context = Rc::clone(&device.context);
        self.upload_batch(&batch)?;
        if !views_only {
            let saved = &mut self.cursor_under;
            saved.rect = planned.cursor_visible;
            if let Some(rect) = planned.cursor_visible {
                let bytes = rect.width as usize * rect.height as usize * 4;
                if saved.buffer.as_ref().is_none_or(|buffer| buffer.size() < bytes) {
                    let bigger = VkBuffer::new(&context, bytes, vk::BufferUsageFlags::TRANSFER_DST, true)?;
                    if let Some(old) = saved.buffer.replace(bigger) {
                        // A frame in flight may still copy into the old one.
                        device.retire.borrow_mut().push(context.timeline.submitted(), old);
                    }
                }
            }
        }

        let uniform_set = self.uniform_set()?;
        // (the texture sets of the frame are next)
        let mut sets: Vec<(TextureSlot, vk::DescriptorSet)> =
            Vec::with_capacity(prepared.sets.len() + 2 * images.len() + views.len());
        sets.extend_from_slice(&prepared.sets);
        for (index, layer_images) in images {
            if let Some((target, _)) = &layer_images.backdrop {
                let set = self.texture_set(target.image.view(), device.pipelines.linear)?;
                sets.push((TextureSlot::Backdrop(*index), set));
            }
            for (which, target) in layer_images.chain.iter().enumerate() {
                if let Some(target) = target {
                    let set = self.texture_set(target.image.view(), device.pipelines.nearest)?;
                    sets.push((TextureSlot::Chain(*index, which), set));
                }
            }
        }
        for (id, view) in &views {
            let set = self.texture_set(*view, device.pipelines.linear)?;
            sets.push((TextureSlot::View(*id), set));
        }
        let filter_set = match &filter {
            Some(filter) => Some(self.texture_set(filter.view, device.pipelines.nearest)?),
            None => None,
        };

        self.upload(prepared);
        let bindings = Bindings {
            uniform: uniform_set,
            textures: &sets,
            pipelines,
        };
        let mut stamps = Stamps::default();
        let transparent = [0.0; 4];
        for job in &jobs {
            match job {
                Job::View {
                    colour,
                    framebuffer,
                    size,
                    plan,
                } => {
                    // A View's picture is cleared to opaque black, and drawn through the depth-stencil pass.
                    let whole = [Rect::new(0, 0, size.width, size.height)];
                    let clears = [Clear {
                        colour: [0.0, 0.0, 0.0, 1.0],
                        rects: &whole,
                    }];
                    self.to_drawn(*colour);
                    self.pass(
                        PassTarget {
                            framebuffer: *framebuffer,
                            size: *size,
                            depth: true,
                        },
                        &clears,
                        &plan.draws,
                        &bindings,
                        &mut stamps,
                    );
                    self.to_sampled(*colour);
                }
                Job::Backdrop {
                    index,
                    plan,
                    background,
                } => {
                    let Some((target, _)) = images.get(index).and_then(|images| images.backdrop.as_ref()) else {
                        continue;
                    };
                    let size = Size::new(target.image.width(), target.image.height());
                    // What lies outside the output stays transparent.
                    let clears = [
                        Clear {
                            colour: transparent,
                            rects: &[Rect::new(0, 0, size.width, size.height)],
                        },
                        Clear {
                            colour: premultiplied(*background),
                            rects: &plan.damage,
                        },
                    ];
                    self.to_drawn(target.image.raw());
                    self.pass(
                        PassTarget::plain(target.framebuffer, size),
                        &clears,
                        &plan.draws[..plan.layers_end],
                        &bindings,
                        &mut stamps,
                    );
                    self.to_sampled(target.image.raw());
                }
                Job::Chain { index, step } => {
                    let Some(target) = images.get(index).and_then(|images| images.chain[step.into].as_ref()) else {
                        continue;
                    };
                    let size = Size::new(target.image.width(), target.image.height());
                    let clears = [Clear {
                        colour: transparent,
                        rects: &step.plan.damage,
                    }];
                    self.to_drawn(target.image.raw());
                    self.pass(
                        PassTarget::plain(target.framebuffer, size),
                        &clears,
                        &step.plan.draws,
                        &bindings,
                        &mut stamps,
                    );
                    self.to_sampled(target.image.raw());
                }
            }
        }
        if !views_only {
            let background = premultiplied(frame.background());
            let main = [Clear {
                colour: background,
                rects: &planned.damage,
            }];
            let target = PassTarget::plain(self.target.framebuffer, self.physical);
            self.pass(
                target,
                &main,
                &planned.draws[..planned.layers_end],
                &bindings,
                &mut stamps,
            );
            if let Some(framebuffer) = models
                && planned.models_end > planned.layers_end
            {
                // The layers a matrix places are tested against a depth buffer cleared for this pass, over what the flat
                // layers drew.
                let placed = PassTarget {
                    framebuffer,
                    size: self.physical,
                    depth: true,
                };
                self.pass(
                    placed,
                    &[],
                    &planned.draws[planned.layers_end..planned.models_end],
                    &bindings,
                    &mut stamps,
                );
            }
            if planned.overlay_end > planned.models_end {
                self.pass(
                    target,
                    &[],
                    &planned.draws[planned.models_end..planned.overlay_end],
                    &bindings,
                    &mut stamps,
                );
            }
            if !planned.cursor_under.is_empty() {
                self.save_under(&planned.cursor_under, self.cursor_under.rect);
            }
            if planned.draws.len() > planned.overlay_end {
                self.pass(
                    target,
                    &[],
                    &planned.draws[planned.overlay_end..],
                    &bindings,
                    &mut stamps,
                );
            }
            if let (Some(filter), Some(texture)) = (&filter, filter_set) {
                self.record_view(filter.image, filter.pipeline, (uniform_set, texture), filter.pass);
            }
        }
        let mut keys: Vec<ShaderKey> = Vec::new();
        let drawn = jobs
            .iter()
            .flat_map(|job| match job {
                Job::View { plan, .. } | Job::Backdrop { plan, .. } => plan.draws.iter(),
                Job::Chain { step, .. } => step.plan.draws.iter(),
            })
            .chain(planned.draws.iter());
        for draw in drawn {
            if let PassKind::User(key) = draw.kind
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        let started = Instant::now();
        let value = self.ring.slot_mut(self.active).submit()?;
        if !keys.is_empty() {
            // A shell's shader is timed, which needs the frame finished: the watchdog is what stops one that is too slow.
            context.wait(value)?;
            self.report_timings(frame, &stamps, &keys, started.elapsed());
        }
        Ok(value)
    }

    /// Tells the frame's shader timings what each shader source cost this frame: the GPU time of its draws when the queue
    /// writes timestamps and the frame's draws fit the query pool, otherwise the wall time from submit to fence, which is
    /// the frame's and so counts every source in it for the whole frame.
    fn report_timings(&self, frame: &OutputFrame, stamps: &Stamps, keys: &[ShaderKey], wall: Duration) {
        if keys.is_empty() {
            return;
        }
        let timings = frame.shader_timings();
        let measured = self
            .slot()
            .query
            .as_ref()
            .filter(|_| !stamps.overflowed && stamps.used > 0)
            .and_then(|query| {
                let mut ticks = vec![0_u64; stamps.used as usize];
                // SAFETY: the frame's timeline value was waited, so every timestamp of the pool was written.
                unsafe {
                    self.device.context.device.get_query_pool_results(
                        query.pool,
                        0,
                        &mut ticks,
                        vk::QueryResultFlags::TYPE_64,
                    )
                }
                .ok()?;
                let mut sums: Vec<(ShaderKey, Duration)> = Vec::new();
                for (key, first) in &stamps.spans {
                    let first = *first as usize;
                    let elapsed = ticks[first + 1].wrapping_sub(ticks[first]) & query.mask;
                    let elapsed = Duration::from_nanos((elapsed as f64 * query.nanoseconds) as u64);
                    match sums.iter_mut().find(|(held, _)| held == key) {
                        Some((_, total)) => *total = total.saturating_add(elapsed),
                        None => sums.push((*key, elapsed)),
                    }
                }
                Some(sums)
            });
        match measured {
            Some(sums) => {
                for (key, elapsed) in sums {
                    timings.record(key, elapsed);
                }
            }
            None => {
                for key in keys {
                    timings.record(*key, wall);
                }
            }
        }
    }

    /// Records the copies that bring the frame's textures up to date.
    fn upload(&self, prepared: &Prepared) {
        let slot = self.slot();
        for copy in &prepared.copies {
            let (old, before) = if copy.fresh {
                (
                    vk::ImageLayout::UNDEFINED,
                    (vk::PipelineStageFlags::TOP_OF_PIPE, vk::AccessFlags::empty()),
                )
            } else {
                (
                    vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                    (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::SHADER_READ),
                )
            };
            slot.barrier(
                copy.image,
                old,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                before,
                (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
            );
            // SAFETY: the staging buffer holds the regions' rows at their offsets, they are disjoint and inside the
            // image, and the image is in `TRANSFER_DST_OPTIMAL` after the barrier above.
            unsafe {
                self.device.context.device.cmd_copy_buffer_to_image(
                    slot.cmd,
                    slot.staging.raw(),
                    copy.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &copy.regions,
                );
            }
            slot.barrier(
                copy.image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
                (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
                (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::SHADER_READ),
            );
        }
    }

    /// Copies the parts of the target the cursor is about to cover.
    fn save_under(&self, parts: &[Rect], visible: Option<Rect>) {
        let Some(visible) = visible else {
            return;
        };
        let saved = &self.cursor_under;
        let Some(buffer) = saved.buffer.as_ref() else {
            return;
        };
        let slot = self.slot();
        let image = self.target.image.raw();
        slot.barrier(
            image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_READ),
        );
        let regions: Vec<vk::BufferImageCopy> = parts
            .iter()
            .map(|part| {
                let row = i64::from(part.y) - i64::from(visible.y);
                let column = i64::from(part.x) - i64::from(visible.x);
                vk::BufferImageCopy::default()
                    .buffer_offset(((row * i64::from(visible.width) + column) * 4) as u64)
                    .buffer_row_length(visible.width)
                    .image_subresource(colour_layers())
                    .image_offset(vk::Offset3D {
                        x: part.x,
                        y: part.y,
                        z: 0,
                    })
                    .image_extent(vk::Extent3D {
                        width: part.width,
                        height: part.height,
                        depth: 1,
                    })
            })
            .collect();
        // SAFETY: every region lies inside the target and inside the saved buffer, which was
        // sized for the visible rectangle; the target is in `TRANSFER_SRC_OPTIMAL`.
        unsafe {
            self.device.context.device.cmd_copy_image_to_buffer(
                slot.cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer.raw(),
                &regions,
            );
        }
        slot.barrier(
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::empty()),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
    }

    /// Records one render pass into `target`: the clears, then the draws.
    ///
    /// A target with a depth-stencil image runs [`DepthExtras::render_pass`], which clears depth to one and stencil to zero, and its
    /// draws use the pipeline of their pass kind and class; a draw of a shell's shader has none there and is skipped. A draw
    /// that names a viewport (a View's eye) sets it, and a draw with a stencil class sets its reference.
    fn pass(
        &self,
        target: PassTarget,
        clears: &[Clear<'_>],
        draws: &[PlannedDraw],
        bindings: &Bindings<'_>,
        stamps: &mut Stamps,
    ) {
        let device = &self.device.context.device;
        let pipelines = &self.device.pipelines;
        let extras = self.device.extras.as_ref();
        let render_pass = match (target.depth, extras) {
            (false, _) => pipelines.render_pass,
            (true, Some(extras)) => extras.render_pass,
            // The callers never ask for a depth pass on a device that has none.
            (true, None) => return,
        };
        let slot = self.slot();
        let cmd = slot.cmd;
        let size = target.size;
        let cleared = [
            vk::ClearValue::default(),
            vk::ClearValue {
                depth_stencil: vk::ClearDepthStencilValue { depth: 1.0, stencil: 0 },
            },
        ];
        let mut begin = vk::RenderPassBeginInfo::default()
            .render_pass(render_pass)
            .framebuffer(target.framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: size.width,
                    height: size.height,
                },
            });
        if target.depth {
            begin = begin.clear_values(&cleared);
        }
        let whole = vk::Viewport {
            x: 0.0,
            y: 0.0,
            width: size.width as f32,
            height: size.height as f32,
            min_depth: 0.0,
            max_depth: 1.0,
        };
        // SAFETY: the pass, framebuffer and pipelines are live and compatible; every draw below
        // stays inside the buffers bound here, whose sizes `record` ensured for the plans, and
        // every scissor lies inside the target.
        unsafe {
            device.cmd_begin_render_pass(cmd, &begin, vk::SubpassContents::INLINE);
            device.cmd_set_viewport(cmd, 0, &[whole]);
            device.cmd_bind_vertex_buffers(cmd, 0, &[self.device.quad.raw(), slot.instances.raw()], &[0, 0]);
            for clear in clears.iter().filter(|clear| !clear.rects.is_empty()) {
                let attachment = vk::ClearAttachment {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    color_attachment: 0,
                    clear_value: vk::ClearValue {
                        color: vk::ClearColorValue { float32: clear.colour },
                    },
                };
                let rects: Vec<vk::ClearRect> = clear
                    .rects
                    .iter()
                    .map(|rect| vk::ClearRect {
                        rect: scissor(*rect),
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .collect();
                device.cmd_clear_attachments(cmd, &[attachment], &rects);
            }
            let mut bound = None;
            let mut eye = false;
            for draw in draws {
                let class = if target.depth {
                    draw.variant.class
                } else {
                    DepthClass::Flat
                };
                if bound != Some((draw.kind, class)) {
                    let pipeline = match (draw.kind, extras.filter(|_| target.depth)) {
                        (PassKind::Solid, None) => pipelines.solid,
                        (PassKind::Surface, None) => pipelines.surface,
                        (PassKind::Shadow, None) => pipelines.shadow,
                        (PassKind::InsetShadow, None) => pipelines.inset_shadow,
                        (PassKind::Border, None) => pipelines.border,
                        (PassKind::Solid, Some(extras)) => extras.placed(pipelines, QuadBase::Solid, class),
                        (PassKind::Surface, Some(extras)) => extras.placed(pipelines, QuadBase::Surface, class),
                        // A shadow, an inset shadow and a border are not drawn on a placed layer, as on GLES.
                        (PassKind::Shadow | PassKind::InsetShadow | PassKind::Border, Some(_)) => continue,
                        (PassKind::User(key), None) => match bindings.pipelines.iter().find(|(held, _)| *held == key) {
                            Some((_, pipeline)) => *pipeline,
                            None => continue,
                        },
                        // A shell's shader has no pipeline in a pass with a depth buffer.
                        (PassKind::User(_), Some(_)) => continue,
                    };
                    if pipeline == vk::Pipeline::null() {
                        continue;
                    }
                    device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
                    bound = Some((draw.kind, class));
                }
                match draw.viewport {
                    Some(rect) => {
                        eye = true;
                        device.cmd_set_viewport(
                            cmd,
                            0,
                            &[vk::Viewport {
                                x: rect.x as f32,
                                y: rect.y as f32,
                                width: rect.width as f32,
                                height: rect.height as f32,
                                min_depth: 0.0,
                                max_depth: 1.0,
                            }],
                        );
                    }
                    None if eye => {
                        eye = false;
                        device.cmd_set_viewport(cmd, 0, &[whole]);
                    }
                    None => {}
                }
                if target.depth && draw.variant.stenciled() {
                    device.cmd_set_stencil_reference(
                        cmd,
                        vk::StencilFaceFlags::FRONT_AND_BACK,
                        u32::from(draw.variant.reference),
                    );
                }
                device.cmd_set_scissor(cmd, 0, &[scissor(draw.scissor)]);
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    pipelines.layout,
                    0,
                    &[bindings.uniform],
                    &[draw.uniform_offset, draw.user_offset],
                );
                for (set, slot) in [(1, draw.slot), (2, draw.backdrop.map(TextureSlot::Backdrop))] {
                    if let Some(slot) = slot
                        && let Some((_, texture)) = bindings.textures.iter().find(|(candidate, _)| *candidate == slot)
                    {
                        device.cmd_bind_descriptor_sets(
                            cmd,
                            vk::PipelineBindPoint::GRAPHICS,
                            pipelines.layout,
                            set,
                            &[*texture],
                            &[],
                        );
                    }
                }
                let key = match draw.kind {
                    PassKind::User(key) => Some(key),
                    PassKind::Solid
                    | PassKind::Surface
                    | PassKind::Shadow
                    | PassKind::InsetShadow
                    | PassKind::Border => None,
                };
                // The draw is covered by this block's contract: the buffers, sets and scissor are the ones bound above.
                stamps.bracket(device, cmd, slot.query.as_ref(), key, || {
                    device.cmd_draw(cmd, 4, 1, 0, draw.instance);
                });
            }
            device.cmd_end_render_pass(cmd);
        }
    }

    /// Moves an image a pass drew into to the layout a later pass samples it in.
    pub(super) fn to_sampled(&self, image: vk::Image) {
        self.slot().barrier(
            image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
            (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::SHADER_READ),
        );
    }

    /// Moves an image to be drawn into, dropping what it held. The image may have been sampled by an earlier pass of this frame.
    pub(super) fn to_drawn(&self, image: vk::Image) {
        self.slot().barrier(
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::SHADER_READ),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
    }

    /// Moves an image that was sampled back to being drawn into, keeping what it holds.
    pub(super) fn to_attached(&self, image: vk::Image) {
        self.slot().barrier(
            image,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::empty()),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
    }

    /// Reads the target back, with the cursor's pixels put back when `!with_cursor`.
    ///
    /// The read is a submission of its own, and the call waits for it, which is also every frame submitted before it.
    pub(super) fn read_target(&mut self, with_cursor: bool) -> Result<Image, vk::Result> {
        let size = self.physical;
        let image = self.target.image.raw();
        let mut pixels = self.read_back(image, size, Resting::attachment())?;
        let stride = size.width as usize * 4;
        // The cursor is drawn last, so its pixels go back first.
        if !with_cursor && let (Some(rect), Some(saved)) = (self.cursor_under.rect, self.cursor_under.bytes()) {
            paste(&mut pixels, stride, rect, saved);
        }
        Image::new(size, stride, pixels).ok_or(vk::Result::ERROR_UNKNOWN)
    }

    /// Copies `image`, `size` pixels that rest as `resting` says, into the readback buffer and returns its bytes, packed.
    ///
    /// The read is a submission of its own, and the call waits for it, which is also every frame submitted before it. The
    /// image is left as it was found.
    pub(super) fn read_back(&mut self, image: vk::Image, size: Size, resting: Resting) -> Result<Vec<u8>, vk::Result> {
        let bytes = size.width as usize * size.height as usize * 4;
        let context = Rc::clone(&self.device.context);
        if self.readback.size() < bytes {
            // The readback is idle: a capture waits for its own submission.
            self.readback = VkBuffer::new(
                &context,
                bytes.next_power_of_two(),
                vk::BufferUsageFlags::TRANSFER_DST,
                true,
            )?;
        }
        self.active = self.ring.acquire();
        self.ring.slot_mut(self.active).begin()?;
        let slot = self.slot();
        slot.barrier(
            image,
            resting.layout,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            (resting.stage, resting.written),
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_READ),
        );
        let region = vk::BufferImageCopy::default()
            .image_subresource(colour_layers())
            .image_extent(vk::Extent3D {
                width: size.width,
                height: size.height,
                depth: 1,
            });
        let to_host = vk::BufferMemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::HOST_READ)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .buffer(self.readback.raw())
            .offset(0)
            .size(vk::WHOLE_SIZE);
        // SAFETY: the readback buffer holds `bytes` bytes and the image is in `TRANSFER_SRC_OPTIMAL` after the barrier above.
        unsafe {
            context.device.cmd_copy_image_to_buffer(
                slot.cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                self.readback.raw(),
                &[region],
            );
            context.device.cmd_pipeline_barrier(
                slot.cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::HOST,
                vk::DependencyFlags::empty(),
                &[],
                &[to_host],
                &[],
            );
        }
        slot.barrier(
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            resting.layout,
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::empty()),
            (resting.stage, resting.restored),
        );
        let value = self.ring.slot_mut(self.active).submit()?;
        context.wait(value)?;
        let Some(pixels) = self.readback.read(0, bytes) else {
            return Err(vk::Result::ERROR_MEMORY_MAP_FAILED);
        };
        Ok(pixels.to_vec())
    }

    /// The renderer error a failed call becomes; a lost device is remembered by the device and every output on it.
    pub(super) fn fail(&mut self, result: vk::Result) -> RenderError {
        let error = classify(result);
        if error == RenderError::Lost {
            self.lost = true;
            self.device.lose();
        }
        log::warn!(area = "render", ?result, ?error, "the Vulkan renderer failed a frame");
        error
    }

    /// Hands what the shader cache evicted to the retire queue, and drops what the GPU has finished with.
    pub(super) fn collect(&self) {
        let value = self.device.context.timeline.submitted();
        let evicted = self.device.shaders.borrow_mut().take_retired();
        let mut retire = self.device.retire.borrow_mut();
        for pipeline in evicted {
            retire.push(value, RetiredPipeline::new(&self.device.context, pipeline));
        }
        retire.release(self.device.context.completed());
    }
}

/// The textures of a frame that have pixels to make, with the slot each fills.
///
/// A solid layer that draws a shader pass has one pixel of its colour, which the pass reads as the layer's own.
fn wanted<'a>(frame: &'a OutputFrame, shaders: &Shaders) -> Vec<Wanted<'a>> {
    let mut found = Vec::new();
    for (index, layer) in frame.layers().iter().enumerate() {
        let surface = match layer {
            Layer::Surface(surface) => surface,
            Layer::Solid(solid) => {
                if solid.model.is_none() && !shaders.of(index).is_empty() {
                    let pixel = premultiplied(solid.colour).map(|value| (value * 255.0).round() as u8);
                    found.push(Wanted {
                        slot: TextureSlot::Layer(index),
                        source: TextureSource::Pixel(pixel),
                        key: None,
                    });
                }
                continue;
            }
            Layer::Overlay(_) => continue,
        };
        let Some(source) = surface_source(surface) else {
            continue;
        };
        found.push(Wanted {
            slot: TextureSlot::Layer(index),
            source,
            key: surface.content.as_deref(),
        });
    }
    if let Some(overlay) = overlay_of(frame) {
        found.push(Wanted {
            slot: TextureSlot::Overlay,
            source: TextureSource::Bitmap(overlay.image()),
            key: None,
        });
    }
    if let Some(cursor) = frame.cursor() {
        found.push(Wanted {
            slot: TextureSlot::Cursor,
            source: TextureSource::Bitmap(cursor.image()),
            key: None,
        });
    }
    found
}

/// What a surface layer's texture is made from, or `None` for a View's, which the renderer drew itself.
pub(super) fn surface_source(surface: &Surface) -> Option<TextureSource<'_>> {
    match &surface.source {
        SurfaceSource::Buffer(buffer) => Some(TextureSource::Shm(buffer)),
        // A snapshot is premultiplied pixels already, the same as the overlay bitmap.
        SurfaceSource::Snapshot(image) => Some(TextureSource::Bitmap(image)),
        // A linear dmabuf is mapped and read (`dmabuf.rs`); one the renderer cannot read is skipped when its shape is asked.
        SurfaceSource::Dmabuf(dmabuf) => Some(TextureSource::Dmabuf(dmabuf)),
        SurfaceSource::View(_) => None,
    }
}

/// Every View whose picture the frame's layers or the scenes it draws sample, each once.
fn sampled_views(frame: &OutputFrame, requests: &[ViewRequest<'_>]) -> Vec<u64> {
    let mut ids: Vec<u64> = frame
        .layers()
        .iter()
        .filter_map(|layer| match layer {
            Layer::Surface(Surface {
                source: SurfaceSource::View(view),
                ..
            }) => Some(view.id),
            _ => None,
        })
        .collect();
    for request in requests {
        ids.extend(scene_views(request.scene.eyes(request.capture)));
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

/// A straight colour as the premultiplied floats a clear takes.
fn premultiplied(colour: Color) -> [f32; 4] {
    let alpha = colour.a.clamp(0.0, 1.0);
    [
        colour.r.clamp(0.0, 1.0) * alpha,
        colour.g.clamp(0.0, 1.0) * alpha,
        colour.b.clamp(0.0, 1.0) * alpha,
        alpha,
    ]
}

/// A rectangle as the scissor the driver takes.
fn scissor(rect: Rect) -> vk::Rect2D {
    vk::Rect2D {
        offset: vk::Offset2D { x: rect.x, y: rect.y },
        extent: vk::Extent2D {
            width: rect.width,
            height: rect.height,
        },
    }
}

/// Writes the saved pixels of `rect` back over a readback with rows `stride` bytes apart.
fn paste(pixels: &mut [u8], stride: usize, rect: Rect, saved: &[u8]) {
    let row = rect.width as usize * 4;
    if row == 0 {
        return;
    }
    let x = rect.x as usize * 4;
    for (line, source) in saved.chunks_exact(row).enumerate() {
        let start = (rect.y as usize + line) * stride + x;
        if let Some(destination) = pixels.get_mut(start..start + row) {
            destination.copy_from_slice(source);
        }
    }
}
