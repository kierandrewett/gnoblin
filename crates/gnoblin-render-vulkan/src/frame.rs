#![allow(unsafe_code)]
//! Recording a frame, submitting it and reading the target back.
//!
//! A frame is one command buffer, submitted once and waited for before `draw` returns. That is
//! the honest cost of an offscreen renderer whose output is read by the CPU: there is nothing
//! to overlap with, and a fence per frame is what tells the renderer the device is lost.
//! Presentation to a display will not wait like this, which is the next slice's work.
//!
//! # A frame, in order
//!
//! 1. **Shader pipelines.** Every shader pass of the frame gets its pipeline (`ShaderPipelines`), made
//!    now if its source has not been drawn before. A source that cannot be drawn is reported on the
//!    frame's timings and its passes are left out.
//! 2. **Textures.** Every surface layer's `wl_shm` pixels are copied once, into a staging
//!    buffer, and from there into an image of their own. The overlay and the cursor are made
//!    the same way, and a solid layer with a shader pass has a one-pixel texture of its colour. A
//!    buffer that cannot be read, or an image the device cannot allocate, is logged and its layer
//!    skipped: the rest of the frame is drawn.
//! 3. **Images of the shader layers.** A layer whose pass reads the backdrop gets an image of its
//!    size to draw the backdrop into, and a layer with several passes gets two scratch images; a
//!    layer whose images cannot be made is drawn without its passes.
//! 4. **Plan.** With the textures known, the frame, each backdrop and each pass of a chain
//!    become lists of draws (`plan.rs`), joined into one set of buffers.
//! 5. **Shader jobs**, from the bottom layer up: a layer's backdrop is drawn (the layers below
//!    it, secure ones black, over its rectangle) and then the passes of its chain, each into its
//!    own render pass and then made readable. A layer reads the backdrops of those below it, so
//!    the order is the layers'.
//! 6. **Pass A.** The damage is cleared to the background and the layers are drawn, each
//!    clipped to the damage rectangle it belongs to.
//! 7. **Overlay.** The overlay is drawn over the layers. The cursor's pixels are copied into
//!    a host buffer first so a capture can leave the cursor out, then the cursor is drawn.
//! 8. **Submit and wait**, and tell the frame's timings what each shader source cost.
//!
//! # Layouts
//!
//! The target lives in `COLOR_ATTACHMENT_OPTIMAL` between frames. A copy out of it moves it to
//! `TRANSFER_SRC_OPTIMAL` and back with explicit barriers; the render pass has the matching
//! subpass dependencies. A texture goes `UNDEFINED`, `TRANSFER_DST_OPTIMAL`,
//! `SHADER_READ_ONLY_OPTIMAL` and is destroyed when the frame has finished. A backdrop or scratch
//! image goes `UNDEFINED`, `COLOR_ATTACHMENT_OPTIMAL` (the render pass draws into it) and
//! `SHADER_READ_ONLY_OPTIMAL` by a barrier after the pass; a scratch image a later pass of the same
//! chain draws into again is moved back by a barrier that waits for the reads of the pass before.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ash::vk;
use gnoblin_render::{Pass, ShaderKey};
use gnoblin_types::{Color, Rect, Scale, Size};

use super::Vulkan;
use super::context::{SHM_FORMAT, TARGET_FORMAT, VkBuffer, VkContext, VkImage, colour_range};
use super::error::classify;
use super::pipeline::{BLOCK_RANGE, DescriptorArena, USER_BLOCK_RANGE, UserPipeline};
use super::plan::{
    ChainStep, DrawBatch, FramePlan, PassKind, PlanTarget, PlannedDraw, ShaderUse, Shaders, TextureSizes, TextureSlot,
    UNIFORM_STRIDE, chain, damage_of, overlay_of, plan, plan_into, quad_of,
};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{BackdropInput, Layer, OutputFrame, SurfaceSource};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shm;

use crate::log;

/// How long a frame may take before the device is treated as lost, in nanoseconds.
const FRAME_TIMEOUT: u64 = 10_000_000_000;

/// The alignment of a texture's bytes inside the staging buffer.
const STAGING_ALIGN: usize = 16;

/// The size of a recorder's staging and readback buffers when it is made, and what a trim leaves.
const MIN_BUFFER: usize = 4096;

/// How many output-sized uploads the staging buffer keeps room for between frames. A frame of a few full-screen
/// windows fits; a larger frame grows the buffer for itself and gives it back after the frame.
const STAGING_SCREENS: usize = 4;

/// What the renderer draws into: an image and the framebuffer over it.
pub(super) struct RenderTarget {
    context: Rc<VkContext>,
    pub(super) image: VkImage,
    pub(super) framebuffer: vk::Framebuffer,
}

impl RenderTarget {
    /// A target of `size` pixels. Its pixels are undefined until [`FrameRecorder::clear`].
    pub(super) fn new(context: &Rc<VkContext>, pass: vk::RenderPass, size: Size) -> Result<Self, vk::Result> {
        Self::with_usage(
            context,
            pass,
            size,
            vk::ImageUsageFlags::COLOR_ATTACHMENT
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
        // SAFETY: the renderer waited for the last frame that drew into it.
        unsafe { self.context.device.destroy_framebuffer(self.framebuffer, None) };
    }
}

/// The pixels under the cursor, saved so a capture can leave it out.
#[derive(Default)]
pub(super) struct Saved {
    buffer: Option<VkBuffer>,
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

/// The command buffer, the fence and the host-visible buffers a frame uses.
pub(super) struct FrameRecorder {
    context: Rc<VkContext>,
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    arena: DescriptorArena,
    quad: VkBuffer,
    uniforms: VkBuffer,
    /// The shader passes' own blocks: the second uniform binding of group 0.
    user_uniforms: VkBuffer,
    instances: VkBuffer,
    staging: VkBuffer,
    readback: VkBuffer,
    /// The timestamp queries, when the queue writes them.
    query: Option<TimestampQuery>,
    /// How long the last submission took from submit to fence.
    wall: Duration,
}

impl FrameRecorder {
    pub(super) fn new(context: &Rc<VkContext>) -> Result<Self, vk::Result> {
        let device = &context.device;
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(context.family)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: the family index is the one the device's queue was taken from.
        let pool = unsafe { device.create_command_pool(&pool_info, None) }?;
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: the pool was just created on this device.
        let cmd = match unsafe { device.allocate_command_buffers(&allocate) } {
            Ok(buffers) => buffers[0],
            Err(result) => {
                // SAFETY: nothing was recorded.
                unsafe { device.destroy_command_pool(pool, None) };
                return Err(result);
            }
        };
        // SAFETY: a plain fence, created unsignalled.
        let fence = match unsafe { device.create_fence(&vk::FenceCreateInfo::default(), None) } {
            Ok(fence) => fence,
            Err(result) => {
                // SAFETY: nothing was recorded.
                unsafe { device.destroy_command_pool(pool, None) };
                return Err(result);
            }
        };
        // From here the guard destroys the three handles if a buffer cannot be made.
        let mut guard = Handles {
            context: Rc::clone(context),
            pool,
            fence,
        };
        let mut quad = VkBuffer::new(context, 32, vk::BufferUsageFlags::VERTEX_BUFFER, false)?;
        // The unit quad as a triangle strip: the vertex stage scales it to the rectangle.
        let corners: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
        let bytes: Vec<u8> = corners.iter().flat_map(|value| value.to_ne_bytes()).collect();
        quad.write(0, &bytes);
        let made = Self {
            context: Rc::clone(context),
            pool,
            cmd,
            fence,
            arena: DescriptorArena::new(context),
            quad,
            uniforms: VkBuffer::new(context, 4096, vk::BufferUsageFlags::UNIFORM_BUFFER, false)?,
            user_uniforms: VkBuffer::new(context, 4096, vk::BufferUsageFlags::UNIFORM_BUFFER, false)?,
            instances: VkBuffer::new(context, 4096, vk::BufferUsageFlags::VERTEX_BUFFER, false)?,
            staging: VkBuffer::new(context, 4096, vk::BufferUsageFlags::TRANSFER_SRC, false)?,
            readback: VkBuffer::new(context, 4096, vk::BufferUsageFlags::TRANSFER_DST, true)?,
            query: make_query(context),
            wall: Duration::ZERO,
        };
        guard.pool = vk::CommandPool::null();
        guard.fence = vk::Fence::null();
        Ok(made)
    }

    /// Clears the target to transparent black and leaves it ready to draw into.
    pub(super) fn clear(&mut self, target: &RenderTarget) -> Result<(), vk::Result> {
        self.begin()?;
        let image = target.image.raw();
        self.barrier(
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            (vk::PipelineStageFlags::TOP_OF_PIPE, vk::AccessFlags::empty()),
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
        );
        let transparent = vk::ClearColorValue { float32: [0.0; 4] };
        // SAFETY: the image is in `TRANSFER_DST_OPTIMAL` after the barrier above.
        unsafe {
            self.context.device.cmd_clear_color_image(
                self.cmd,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &transparent,
                &[colour_range()],
            );
        }
        self.barrier(
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
        self.submit()
    }

    /// Starts recording a new frame: resets the fence, the command buffer and the descriptors.
    fn begin(&mut self) -> Result<(), vk::Result> {
        let device = &self.context.device;
        // SAFETY: the previous frame's fence was waited, so neither the fence nor the buffer is
        // in use, and the pool allows resetting its buffers.
        unsafe {
            device.reset_fences(&[self.fence])?;
            device.reset_command_buffer(self.cmd, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(
                self.cmd,
                &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            if let Some(query) = &self.query {
                device.cmd_reset_query_pool(self.cmd, query.pool, 0, STAMP_QUERIES);
            }
        }
        self.arena.reset()
    }

    /// Ends the command buffer, submits it and waits for the fence.
    ///
    /// A timeout is reported as a lost device: the work may still be running, and the renderer
    /// can say nothing more about what the device holds.
    fn submit(&mut self) -> Result<(), vk::Result> {
        let device = &self.context.device;
        // SAFETY: the buffer is in the recording state and holds complete commands.
        unsafe { device.end_command_buffer(self.cmd) }?;
        #[cfg(test)]
        if let Some(result) = self.context.faults.submit.take() {
            return Err(result);
        }
        let buffers = [self.cmd];
        let submit = vk::SubmitInfo::default().command_buffers(&buffers);
        let started = Instant::now();
        // SAFETY: the queue belongs to this device and is used from this thread only; the
        // fence was reset in `begin`.
        unsafe {
            device.queue_submit(self.context.queue, &[submit], self.fence)?;
            device.wait_for_fences(&[self.fence], true, FRAME_TIMEOUT)?;
        }
        self.wall = started.elapsed();
        Ok(())
    }

    /// An image layout transition with the stages and accesses on each side.
    fn barrier(
        &self,
        image: vk::Image,
        old: vk::ImageLayout,
        new: vk::ImageLayout,
        before: (vk::PipelineStageFlags, vk::AccessFlags),
        after: (vk::PipelineStageFlags, vk::AccessFlags),
    ) {
        let barrier = vk::ImageMemoryBarrier::default()
            .old_layout(old)
            .new_layout(new)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(image)
            .subresource_range(colour_range())
            .src_access_mask(before.1)
            .dst_access_mask(after.1);
        // SAFETY: the command buffer is recording and the image belongs to this device.
        unsafe {
            self.context.device.cmd_pipeline_barrier(
                self.cmd,
                before.0,
                after.0,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[barrier],
            );
        }
    }

    /// Grows `buffer` to hold at least `bytes`.
    fn ensure(
        context: &Rc<VkContext>,
        buffer: &mut VkBuffer,
        bytes: usize,
        usage: vk::BufferUsageFlags,
        cached: bool,
    ) -> Result<(), vk::Result> {
        if buffer.size() < bytes {
            // The device is idle between frames, so the old buffer can be dropped at once.
            *buffer = VkBuffer::new(context, bytes.next_power_of_two(), usage, cached)?;
        }
        Ok(())
    }

    /// The host-visible bytes the recorder holds now: the quad, uniform, shader-block, instance, staging and readback buffers.
    pub(super) fn bytes(&self) -> u64 {
        [
            &self.quad,
            &self.uniforms,
            &self.user_uniforms,
            &self.instances,
            &self.staging,
            &self.readback,
        ]
        .iter()
        .map(|buffer| buffer.size() as u64)
        .fold(0, u64::saturating_add)
    }

    /// Gives back what a past peak left behind: a staging buffer above `staging_retain` bytes and a readback buffer
    /// above `readback_retain` are replaced by the smallest a recorder starts with, and the next frame that needs
    /// more grows it again.
    ///
    /// [`FrameRecorder::ensure`] only grows, so without this one large frame (a window opened at a huge size, an
    /// output supersampled and then not) keeps its host-visible memory for the life of the renderer. A buffer that
    /// cannot be replaced is kept: a trim that fails costs memory, never a frame.
    pub(super) fn trim(&mut self, staging_retain: usize, readback_retain: usize) {
        let context = Rc::clone(&self.context);
        if self.staging.size() > staging_retain
            && let Ok(buffer) = VkBuffer::new(&context, MIN_BUFFER, vk::BufferUsageFlags::TRANSFER_SRC, false)
        {
            // The device is idle between frames, so the old buffer can be dropped at once.
            self.staging = buffer;
        }
        if self.readback.size() > readback_retain
            && let Ok(buffer) = VkBuffer::new(&context, MIN_BUFFER, vk::BufferUsageFlags::TRANSFER_DST, true)
        {
            self.readback = buffer;
        }
    }
}

/// Destroys a command pool and a fence if the recorder could not be completed.
struct Handles {
    context: Rc<VkContext>,
    pool: vk::CommandPool,
    fence: vk::Fence,
}

impl Drop for Handles {
    fn drop(&mut self) {
        // SAFETY: nothing was submitted; a null handle is a no-op.
        unsafe {
            self.context.device.destroy_fence(self.fence, None);
            self.context.device.destroy_command_pool(self.pool, None);
        }
    }
}

impl Drop for FrameRecorder {
    fn drop(&mut self) {
        // SAFETY: the renderer is idle; destroying the pool frees its command buffer.
        unsafe {
            if let Some(query) = &self.query {
                self.context.device.destroy_query_pool(query.pool, None);
            }
            self.context.device.destroy_fence(self.fence, None);
            self.context.device.destroy_command_pool(self.pool, None);
        }
    }
}

/// A texture made for one frame: its image and where its pixels wait in the staging buffer.
struct Upload {
    slot: TextureSlot,
    image: VkImage,
    offset: usize,
}

/// A texture to make: its source and the space it takes.
enum PixelSource<'a> {
    Shm {
        buffer: &'a smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer,
        shape: shm::ShmShape,
    },
    Bitmap(&'a Image),
    /// One premultiplied pixel: a solid layer's colour, which its shader pass reads as the layer's own.
    Pixel([u8; 4]),
}

impl PixelSource<'_> {
    fn size(&self) -> (u32, u32) {
        match self {
            Self::Shm { shape, .. } => (shape.width, shape.height),
            Self::Bitmap(image) => {
                let size = image.size();
                (size.width, size.height)
            }
            Self::Pixel(_) => (1, 1),
        }
    }

    fn bytes(&self) -> usize {
        let (width, height) = self.size();
        width as usize * height as usize * 4
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

/// How many timestamps a frame can write: two for each draw of a shader pass.
const STAMP_QUERIES: u32 = 512;

/// The timestamp queries of the device: the pool, and what a tick of it is worth.
struct TimestampQuery {
    pool: vk::QueryPool,
    /// The length of a tick in nanoseconds.
    nanoseconds: f64,
    /// The bits of a timestamp the queue writes.
    mask: u64,
}

/// The pool of timestamp queries, when the queue writes any.
fn make_query(context: &VkContext) -> Option<TimestampQuery> {
    if context.timestamp_bits == 0 {
        return None;
    }
    let info = vk::QueryPoolCreateInfo::default()
        .query_type(vk::QueryType::TIMESTAMP)
        .query_count(STAMP_QUERIES);
    // SAFETY: `info` is a valid description; the pool is destroyed with the recorder.
    let pool = unsafe { context.device.create_query_pool(&info, None) }.ok()?;
    let bits = context.timestamp_bits;
    Some(TimestampQuery {
        pool,
        nanoseconds: f64::from(context.limits.timestamp_period),
        mask: if bits >= 64 { u64::MAX } else { (1_u64 << bits) - 1 },
    })
}

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
    /// The layers below a layer, drawn over its rectangle into its backdrop image.
    Backdrop {
        index: usize,
        plan: FramePlan,
        background: Color,
    },
    /// One pass of a layer's chain, drawn into one of its scratch images.
    Chain { index: usize, step: ChainStep },
}

/// What `paint` hands to `record`.
struct FrameRecording<'a> {
    frame: &'a OutputFrame,
    planned: &'a FramePlan,
    batch: DrawBatch,
    uploads: &'a [Upload],
    images: &'a BTreeMap<usize, LayerImages>,
    jobs: Vec<Job>,
    pipelines: &'a [(ShaderKey, vk::Pipeline)],
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

impl Vulkan {
    /// The host-visible bytes the renderer holds: the recorder's buffers and the pixels saved under the cursor.
    pub(super) fn host_bytes(&self) -> u64 {
        self.recorder.bytes().saturating_add(
            self.cursor_under
                .buffer
                .as_ref()
                .map_or(0, |buffer| buffer.size() as u64),
        )
    }

    /// Gives back the host-visible memory a past peak left in the recorder, keeping what one steady frame needs: a
    /// readback of the target, and staging for [`STAGING_SCREENS`] output-sized uploads.
    pub(super) fn trim(&mut self) {
        let target = self.physical.width as usize * self.physical.height as usize * 4;
        let readback = target.next_power_of_two();
        self.recorder.trim(readback.saturating_mul(STAGING_SCREENS), readback);
    }

    /// Draws `frame` into the target. See [`Vulkan::draw`].
    pub(super) fn paint(&mut self, frame: &OutputFrame, scale: Scale) -> Result<(), vk::Result> {
        if damage_of(frame, scale, self.physical).is_empty() {
            return Ok(());
        }
        let (mut shaders, pipelines) = self.resolve_shaders(frame);
        let sources = sources(frame, &shaders);
        let staging_bytes = sources
            .iter()
            .map(|(_, source)| source.bytes().next_multiple_of(STAGING_ALIGN))
            .sum::<usize>();
        let context = Rc::clone(&self.context);
        FrameRecorder::ensure(
            &context,
            &mut self.recorder.staging,
            staging_bytes,
            vk::BufferUsageFlags::TRANSFER_SRC,
            false,
        )?;
        let mut uploads: Vec<Upload> = Vec::with_capacity(sources.len());
        let mut sizes = TextureSizes {
            layers: vec![None; frame.layers().len()],
            ..TextureSizes::default()
        };
        let mut offset = 0;
        for (slot, source) in &sources {
            let (width, height) = source.size();
            let (format, opaque) = match source {
                PixelSource::Shm { shape, .. } => (SHM_FORMAT, shape.opaque),
                PixelSource::Bitmap(_) | PixelSource::Pixel(_) => (TARGET_FORMAT, false),
            };
            let image = match VkImage::new(
                &context,
                width,
                height,
                format,
                vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST,
                opaque,
            ) {
                Ok(image) => image,
                Err(result) => {
                    log::warn!(
                        area = "render",
                        ?result,
                        ?slot,
                        "a texture could not be made; the layer is skipped"
                    );
                    continue;
                }
            };
            let Some(destination) = self.recorder.staging.as_mut_slice(offset, source.bytes()) else {
                continue;
            };
            if !fill(source, destination) {
                continue;
            }
            match *slot {
                TextureSlot::Layer(index) => sizes.layers[index] = Some((width, height)),
                TextureSlot::Overlay => sizes.overlay = Some((width, height)),
                TextureSlot::Cursor => sizes.cursor = Some((width, height)),
                TextureSlot::Chain(..) | TextureSlot::Backdrop(_) => continue,
            }
            uploads.push(Upload {
                slot: *slot,
                image,
                offset,
            });
            offset += source.bytes().next_multiple_of(STAGING_ALIGN);
        }
        let images = self.make_images(frame, scale, &mut shaders, &sizes);
        let mut batch = DrawBatch::default();
        let jobs = self.plan_jobs(frame, scale, &shaders, &sizes, &images, &mut batch);
        let planned = plan(frame, scale, self.physical, &sizes, &shaders).append_to(&mut batch);
        if planned.skipped_models > 0 && !self.warned_models {
            self.warned_models = true;
            log::warn!(
                layers = planned.skipped_models,
                "unsupported-3d: the Vulkan renderer has no depth pass, so a layer with a model is not drawn"
            );
        }
        self.record(FrameRecording {
            frame,
            planned: &planned,
            batch,
            uploads: &uploads,
            images: &images,
            jobs,
            pipelines: &pipelines,
        })
    }

    /// Makes the pipeline of every shader pass the frame draws, and says which passes each layer has.
    ///
    /// A source that cannot be drawn (the validator refuses it, it does not compile, or the driver refuses the pipeline) is
    /// reported on the frame's timings with a short code and left out: its node is drawn without the pass. A pass that was not
    /// marked as reading the backdrop but whose source does is refused too, because the frame would not have built one.
    fn resolve_shaders(&mut self, frame: &OutputFrame) -> (Shaders, Vec<(ShaderKey, vk::Pipeline)>) {
        self.shaders.begin_frame();
        let mut shaders = Shaders {
            layers: vec![Vec::new(); frame.layers().len()],
            time: self.epoch.elapsed().as_secs_f32(),
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
                        let outcome = self.shaders.resolve(pass);
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
        let render_pass = self.pipelines.render_pass;
        let backdrop = match input {
            Some(input) => Some((RenderTarget::scratch(&self.context, render_pass, size)?, input)),
            None => None,
        };
        let mut chain = [None, None];
        for image in chain.iter_mut().take(passes.saturating_sub(1)) {
            *image = Some(RenderTarget::scratch(&self.context, render_pass, size)?);
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

    /// A descriptor set that binds `view` with `sampler` as a texture and its sampler.
    fn texture_set(&mut self, view: vk::ImageView, sampler: vk::Sampler) -> Result<vk::DescriptorSet, vk::Result> {
        let set = self.recorder.arena.allocate(self.pipelines.texture_layout)?;
        let image = [vk::DescriptorImageInfo::default()
            .image_view(view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
        let sampler = [vk::DescriptorImageInfo::default().sampler(sampler)];
        let writes = [
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(0)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .image_info(&image),
            vk::WriteDescriptorSet::default()
                .dst_set(set)
                .dst_binding(1)
                .descriptor_type(vk::DescriptorType::SAMPLER)
                .image_info(&sampler),
        ];
        // SAFETY: the set, view and sampler are live and the layout matches the set's.
        unsafe { self.context.device.update_descriptor_sets(&writes, &[]) };
        Ok(set)
    }

    /// Records the frame and submits it.
    fn record(&mut self, recording: FrameRecording<'_>) -> Result<(), vk::Result> {
        let FrameRecording {
            frame,
            planned,
            batch,
            uploads,
            images,
            jobs,
            pipelines,
        } = recording;
        let context = Rc::clone(&self.context);
        FrameRecorder::ensure(
            &context,
            &mut self.recorder.uniforms,
            batch.uniforms.len().max(UNIFORM_STRIDE),
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            false,
        )?;
        FrameRecorder::ensure(
            &context,
            &mut self.recorder.user_uniforms,
            batch.user.len().max(UNIFORM_STRIDE),
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            false,
        )?;
        let instance_bytes: Vec<u8> = batch.instances.iter().flat_map(|value| value.to_ne_bytes()).collect();
        FrameRecorder::ensure(
            &context,
            &mut self.recorder.instances,
            instance_bytes.len().max(16),
            vk::BufferUsageFlags::VERTEX_BUFFER,
            false,
        )?;
        self.recorder.uniforms.write(0, &batch.uniforms);
        self.recorder.user_uniforms.write(0, &batch.user);
        self.recorder.instances.write(0, &instance_bytes);
        let saved = &mut self.cursor_under;
        saved.rect = planned.cursor_visible;
        if let Some(rect) = planned.cursor_visible {
            let bytes = rect.width as usize * rect.height as usize * 4;
            if saved.buffer.as_ref().is_none_or(|buffer| buffer.size() < bytes) {
                saved.buffer = Some(VkBuffer::new(
                    &context,
                    bytes,
                    vk::BufferUsageFlags::TRANSFER_DST,
                    true,
                )?);
            }
        }

        self.recorder.begin()?;
        let uniform_set = self.recorder.arena.allocate(self.pipelines.uniform_layout)?;
        let block = [vk::DescriptorBufferInfo::default()
            .buffer(self.recorder.uniforms.raw())
            .offset(0)
            .range(BLOCK_RANGE)];
        let user_block = [vk::DescriptorBufferInfo::default()
            .buffer(self.recorder.user_uniforms.raw())
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
        unsafe { context.device.update_descriptor_sets(&write_uniforms, &[]) };
        let mut sets: Vec<(TextureSlot, vk::DescriptorSet)> = Vec::with_capacity(uploads.len() + 2 * images.len());
        for upload in uploads {
            let sampler = if upload.slot.nearest() {
                self.pipelines.nearest
            } else {
                self.pipelines.linear
            };
            let set = self.texture_set(upload.image.view(), sampler)?;
            sets.push((upload.slot, set));
        }
        for (index, layer_images) in images {
            if let Some((target, _)) = &layer_images.backdrop {
                let set = self.texture_set(target.image.view(), self.pipelines.linear)?;
                sets.push((TextureSlot::Backdrop(*index), set));
            }
            for (which, target) in layer_images.chain.iter().enumerate() {
                if let Some(target) = target {
                    let set = self.texture_set(target.image.view(), self.pipelines.nearest)?;
                    sets.push((TextureSlot::Chain(*index, which), set));
                }
            }
        }

        for upload in uploads {
            self.upload(upload);
        }
        let bindings = Bindings {
            uniform: uniform_set,
            textures: &sets,
            pipelines,
        };
        let mut stamps = Stamps::default();
        let transparent = [0.0; 4];
        for job in &jobs {
            match job {
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
                    self.pass((target.framebuffer, size), &clears, &plan.draws, &bindings, &mut stamps);
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
                        (target.framebuffer, size),
                        &clears,
                        &step.plan.draws,
                        &bindings,
                        &mut stamps,
                    );
                    self.to_sampled(target.image.raw());
                }
            }
        }
        let background = premultiplied(frame.background());
        let main = [Clear {
            colour: background,
            rects: &planned.damage,
        }];
        let target = (self.target.framebuffer, self.physical);
        self.pass(
            target,
            &main,
            &planned.draws[..planned.layers_end],
            &bindings,
            &mut stamps,
        );
        if planned.overlay_end > planned.layers_end {
            self.pass(
                target,
                &[],
                &planned.draws[planned.layers_end..planned.overlay_end],
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
        self.recorder.submit()?;
        let mut keys: Vec<ShaderKey> = Vec::new();
        let drawn = jobs
            .iter()
            .flat_map(|job| match job {
                Job::Backdrop { plan, .. } => plan.draws.iter(),
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
        self.report_timings(frame, &stamps, &keys);
        Ok(())
    }

    /// Tells the frame's shader timings what each shader source cost this frame: the GPU time of its draws when the queue
    /// writes timestamps and the frame's draws fit the query pool, otherwise the wall time from submit to fence, which is
    /// the frame's and so counts every source in it for the whole frame.
    fn report_timings(&self, frame: &OutputFrame, stamps: &Stamps, keys: &[ShaderKey]) {
        if keys.is_empty() {
            return;
        }
        let timings = frame.shader_timings();
        let measured = self
            .recorder
            .query
            .as_ref()
            .filter(|_| !stamps.overflowed && stamps.used > 0)
            .and_then(|query| {
                let mut ticks = vec![0_u64; stamps.used as usize];
                // SAFETY: the frame's fence was waited, so every timestamp of the pool was written.
                unsafe {
                    self.context
                        .device
                        .get_query_pool_results(query.pool, 0, &mut ticks, vk::QueryResultFlags::TYPE_64)
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
                    timings.record(*key, self.recorder.wall);
                }
            }
        }
    }

    /// Copies a texture's pixels from the staging buffer into its image.
    fn upload(&self, upload: &Upload) {
        let recorder = &self.recorder;
        let image = upload.image.raw();
        recorder.barrier(
            image,
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            (vk::PipelineStageFlags::TOP_OF_PIPE, vk::AccessFlags::empty()),
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
        );
        let region = vk::BufferImageCopy::default()
            .buffer_offset(upload.offset as u64)
            .image_subresource(colour_layers())
            .image_extent(vk::Extent3D {
                width: upload.image.width(),
                height: upload.image.height(),
                depth: 1,
            });
        // SAFETY: the staging buffer holds the texture's rows at `offset`, and the image is in
        // `TRANSFER_DST_OPTIMAL` after the barrier above.
        unsafe {
            self.context.device.cmd_copy_buffer_to_image(
                recorder.cmd,
                recorder.staging.raw(),
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[region],
            );
        }
        recorder.barrier(
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::TRANSFER_WRITE),
            (vk::PipelineStageFlags::FRAGMENT_SHADER, vk::AccessFlags::SHADER_READ),
        );
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
        let recorder = &self.recorder;
        let image = self.target.image.raw();
        recorder.barrier(
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
            self.context.device.cmd_copy_image_to_buffer(
                recorder.cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                buffer.raw(),
                &regions,
            );
        }
        recorder.barrier(
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
    fn pass(
        &self,
        target: (vk::Framebuffer, Size),
        clears: &[Clear<'_>],
        draws: &[PlannedDraw],
        bindings: &Bindings<'_>,
        stamps: &mut Stamps,
    ) {
        let device = &self.context.device;
        let cmd = self.recorder.cmd;
        let (framebuffer, size) = target;
        let begin = vk::RenderPassBeginInfo::default()
            .render_pass(self.pipelines.render_pass)
            .framebuffer(framebuffer)
            .render_area(vk::Rect2D {
                offset: vk::Offset2D { x: 0, y: 0 },
                extent: vk::Extent2D {
                    width: size.width,
                    height: size.height,
                },
            });
        let viewport = vk::Viewport {
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
            device.cmd_set_viewport(cmd, 0, &[viewport]);
            device.cmd_bind_vertex_buffers(
                cmd,
                0,
                &[self.recorder.quad.raw(), self.recorder.instances.raw()],
                &[0, 0],
            );
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
            for draw in draws {
                if bound != Some(draw.kind) {
                    let pipeline = match draw.kind {
                        PassKind::Solid => self.pipelines.solid,
                        PassKind::Surface => self.pipelines.surface,
                        PassKind::Shadow => self.pipelines.shadow,
                        PassKind::InsetShadow => self.pipelines.inset_shadow,
                        PassKind::Border => self.pipelines.border,
                        PassKind::User(key) => match bindings.pipelines.iter().find(|(held, _)| *held == key) {
                            Some((_, pipeline)) => *pipeline,
                            None => continue,
                        },
                    };
                    device.cmd_bind_pipeline(cmd, vk::PipelineBindPoint::GRAPHICS, pipeline);
                    bound = Some(draw.kind);
                }
                device.cmd_set_scissor(cmd, 0, &[scissor(draw.scissor)]);
                device.cmd_bind_descriptor_sets(
                    cmd,
                    vk::PipelineBindPoint::GRAPHICS,
                    self.pipelines.layout,
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
                            self.pipelines.layout,
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
                stamps.bracket(device, cmd, self.recorder.query.as_ref(), key, || {
                    device.cmd_draw(cmd, 4, 1, 0, draw.instance);
                });
            }
            device.cmd_end_render_pass(cmd);
        }
    }

    /// Moves an image a pass drew into to the layout a later pass samples it in.
    fn to_sampled(&self, image: vk::Image) {
        self.recorder.barrier(
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
    fn to_drawn(&self, image: vk::Image) {
        self.recorder.barrier(
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

    /// Reads the target back, with the cursor's pixels put back when `!with_cursor`.
    pub(super) fn read_target(&mut self, with_cursor: bool) -> Result<Image, vk::Result> {
        let size = self.physical;
        let bytes = size.width as usize * size.height as usize * 4;
        let context = Rc::clone(&self.context);
        FrameRecorder::ensure(
            &context,
            &mut self.recorder.readback,
            bytes,
            vk::BufferUsageFlags::TRANSFER_DST,
            true,
        )?;
        self.recorder.begin()?;
        let image = self.target.image.raw();
        let recorder = &self.recorder;
        recorder.barrier(
            image,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
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
            .buffer(recorder.readback.raw())
            .offset(0)
            .size(vk::WHOLE_SIZE);
        // SAFETY: the readback buffer holds `bytes` bytes and the target is in
        // `TRANSFER_SRC_OPTIMAL` after the barrier above.
        unsafe {
            self.context.device.cmd_copy_image_to_buffer(
                recorder.cmd,
                image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                recorder.readback.raw(),
                &[region],
            );
            self.context.device.cmd_pipeline_barrier(
                recorder.cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::HOST,
                vk::DependencyFlags::empty(),
                &[],
                &[to_host],
                &[],
            );
        }
        recorder.barrier(
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
            (vk::PipelineStageFlags::TRANSFER, vk::AccessFlags::empty()),
            (
                vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE,
            ),
        );
        self.recorder.submit()?;
        let Some(pixels) = self.recorder.readback.read(0, bytes) else {
            return Err(vk::Result::ERROR_MEMORY_MAP_FAILED);
        };
        let mut pixels = pixels.to_vec();
        let stride = size.width as usize * 4;
        // The cursor is drawn last, so its pixels go back first.
        if !with_cursor && let (Some(rect), Some(saved)) = (self.cursor_under.rect, self.cursor_under.bytes()) {
            paste(&mut pixels, stride, rect, saved);
        }
        Image::new(size, stride, pixels).ok_or(vk::Result::ERROR_UNKNOWN)
    }

    /// The renderer error a failed call becomes; a lost device is remembered.
    pub(super) fn fail(&mut self, result: vk::Result) -> RenderError {
        let error = classify(result);
        if error == RenderError::Lost {
            self.lost = true;
            // Nothing is drawn on this device again, so every shader pipeline goes now.
            self.shaders.drain();
        }
        log::warn!(area = "render", ?result, ?error, "the Vulkan renderer failed a frame");
        error
    }
}

/// The layers of a frame that have pixels to upload, with the slot each fills.
///
/// A solid layer that draws a shader pass has one pixel of its colour, which the pass reads as the layer's own.
fn sources<'a>(frame: &'a OutputFrame, shaders: &Shaders) -> Vec<(TextureSlot, PixelSource<'a>)> {
    let mut found = Vec::new();
    for (index, layer) in frame.layers().iter().enumerate() {
        let surface = match layer {
            Layer::Surface(surface) => surface,
            Layer::Solid(solid) => {
                if solid.model.is_none() && !shaders.of(index).is_empty() {
                    let pixel = premultiplied(solid.colour).map(|value| (value * 255.0).round() as u8);
                    found.push((TextureSlot::Layer(index), PixelSource::Pixel(pixel)));
                }
                continue;
            }
            Layer::Overlay(_) => continue,
        };
        if surface.model.is_some() {
            continue;
        }
        match &surface.source {
            SurfaceSource::Buffer(buffer) => match shm::shape(buffer) {
                Ok(shape) => found.push((TextureSlot::Layer(index), PixelSource::Shm { buffer, shape })),
                Err(error) => {
                    log::warn!(area = "render", %error, "a client buffer could not be read; the layer is skipped")
                }
            },
            // A snapshot is premultiplied pixels already, the same as the overlay bitmap.
            SurfaceSource::Snapshot(image) => found.push((TextureSlot::Layer(index), PixelSource::Bitmap(image))),
            SurfaceSource::Dmabuf(_) => {
                log::warn!(
                    area = "render",
                    "a GPU buffer reached the Vulkan renderer, which cannot import it; the layer is skipped"
                );
            }
            SurfaceSource::View(_) => {
                log::warn!(
                    area = "render",
                    "a View's GPU texture reached the Vulkan renderer, which draws a View on the CPU; the layer is skipped"
                );
            }
        }
    }
    if let Some(overlay) = overlay_of(frame) {
        found.push((TextureSlot::Overlay, PixelSource::Bitmap(overlay.image())));
    }
    if let Some(cursor) = frame.cursor() {
        found.push((TextureSlot::Cursor, PixelSource::Bitmap(cursor.image())));
    }
    found
}

/// Copies a source's pixels, rows packed, into `destination`. Returns false when it cannot.
fn fill(source: &PixelSource<'_>, destination: &mut [u8]) -> bool {
    match source {
        PixelSource::Shm { buffer, .. } => match shm::read_packed(buffer, destination) {
            Ok(_) => true,
            Err(error) => {
                log::warn!(area = "render", %error, "a client buffer could not be read; the layer is skipped");
                false
            }
        },
        PixelSource::Bitmap(image) => {
            let size = image.size();
            let row = size.width as usize * 4;
            for (line, out) in destination.chunks_exact_mut(row).enumerate() {
                let start = line * image.stride();
                let Some(pixels) = image.data().get(start..start + row) else {
                    return false;
                };
                out.copy_from_slice(pixels);
            }
            true
        }
        PixelSource::Pixel(pixel) => {
            destination.copy_from_slice(pixel);
            true
        }
    }
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

/// The colour layers of the first mip level.
fn colour_layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
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
