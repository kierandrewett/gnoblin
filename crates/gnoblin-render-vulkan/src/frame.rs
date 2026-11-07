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
//! 1. **Textures.** Every surface layer's `wl_shm` pixels are copied once, into a staging
//!    buffer, and from there into an image of their own. The overlay and the cursor are made
//!    the same way. A buffer that cannot be read, or an image the device cannot allocate, is
//!    logged and its layer skipped: the rest of the frame is drawn.
//! 2. **Plan.** With the textures known, the frame becomes a list of draws (`plan.rs`).
//! 3. **Pass A.** The damage is cleared to the background and the layers are drawn, each
//!    clipped to the damage rectangle it belongs to.
//! 4. **Overlay.** The pixels it will cover are copied into a host buffer so a capture can
//!    leave the overlay out, then it is drawn. The cursor does the same, above the overlay.
//! 5. **Submit and wait.**
//!
//! # Layouts
//!
//! The target lives in `COLOR_ATTACHMENT_OPTIMAL` between frames. A copy out of it moves it to
//! `TRANSFER_SRC_OPTIMAL` and back with explicit barriers; the render pass has the matching
//! subpass dependencies. A texture goes `UNDEFINED`, `TRANSFER_DST_OPTIMAL`,
//! `SHADER_READ_ONLY_OPTIMAL` and is destroyed when the frame has finished.

use std::rc::Rc;

use ash::vk;
use gnoblin_types::{Color, Rect, Scale, Size};

use super::Vulkan;
use super::context::{SHM_FORMAT, TARGET_FORMAT, VkBuffer, VkContext, VkImage, colour_range};
use super::error::classify;
use super::pipeline::{BLOCK_RANGE, DescriptorArena};
use super::plan::{FramePlan, PassKind, TextureSizes, TextureSlot, damage_of, overlay_of, plan};
use gnoblin_render_frame::error::RenderError;
use gnoblin_render_frame::frame::{Layer, OutputFrame};
use gnoblin_render_frame::image::Image;
use gnoblin_render_frame::shm;

use crate::log;

/// How long a frame may take before the device is treated as lost, in nanoseconds.
const FRAME_TIMEOUT: u64 = 10_000_000_000;

/// The alignment of a texture's bytes inside the staging buffer.
const STAGING_ALIGN: usize = 16;

/// What the renderer draws into: an image and the framebuffer over it.
pub(super) struct RenderTarget {
    context: Rc<VkContext>,
    pub(super) image: VkImage,
    pub(super) framebuffer: vk::Framebuffer,
}

impl RenderTarget {
    /// A target of `size` pixels. Its pixels are undefined until [`FrameRecorder::clear`].
    pub(super) fn new(context: &Rc<VkContext>, pass: vk::RenderPass, size: Size) -> Result<Self, vk::Result> {
        let image = VkImage::new(
            context,
            size.width,
            size.height,
            TARGET_FORMAT,
            vk::ImageUsageFlags::COLOR_ATTACHMENT
                | vk::ImageUsageFlags::TRANSFER_SRC
                | vk::ImageUsageFlags::TRANSFER_DST,
            false,
        )?;
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

/// The pixels under the overlay or the cursor, saved so a capture can leave them out.
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
    instances: VkBuffer,
    staging: VkBuffer,
    readback: VkBuffer,
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
            instances: VkBuffer::new(context, 4096, vk::BufferUsageFlags::VERTEX_BUFFER, false)?,
            staging: VkBuffer::new(context, 4096, vk::BufferUsageFlags::TRANSFER_SRC, false)?,
            readback: VkBuffer::new(context, 4096, vk::BufferUsageFlags::TRANSFER_DST, true)?,
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
        // SAFETY: the queue belongs to this device and is used from this thread only; the
        // fence was reset in `begin`.
        unsafe {
            device.queue_submit(self.context.queue, &[submit], self.fence)?;
            device.wait_for_fences(&[self.fence], true, FRAME_TIMEOUT)?;
        }
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
}

impl PixelSource<'_> {
    fn size(&self) -> (u32, u32) {
        match self {
            Self::Shm { shape, .. } => (shape.width, shape.height),
            Self::Bitmap(image) => {
                let size = image.size();
                (size.width, size.height)
            }
        }
    }

    fn bytes(&self) -> usize {
        let (width, height) = self.size();
        width as usize * height as usize * 4
    }
}

impl Vulkan {
    /// Draws `frame` into the target. See [`Vulkan::draw`].
    pub(super) fn paint(&mut self, frame: &OutputFrame, scale: Scale) -> Result<(), vk::Result> {
        if damage_of(frame, scale, self.physical).is_empty() {
            return Ok(());
        }
        let sources = sources(frame);
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
                PixelSource::Bitmap(_) => (TARGET_FORMAT, false),
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
            }
            uploads.push(Upload {
                slot: *slot,
                image,
                offset,
            });
            offset += source.bytes().next_multiple_of(STAGING_ALIGN);
        }
        let planned = plan(frame, scale, self.physical, &sizes);
        if planned.skipped_models > 0 && !self.warned_models {
            self.warned_models = true;
            log::warn!(
                layers = planned.skipped_models,
                "unsupported-3d: the Vulkan renderer has no depth pass, so a layer with a model is not drawn"
            );
        }
        self.record(frame, &planned, &uploads)
    }

    /// Records the frame and submits it.
    fn record(&mut self, frame: &OutputFrame, planned: &FramePlan, uploads: &[Upload]) -> Result<(), vk::Result> {
        let context = Rc::clone(&self.context);
        let uniform_bytes = planned.uniforms.len().max(super::plan::UNIFORM_STRIDE);
        FrameRecorder::ensure(
            &context,
            &mut self.recorder.uniforms,
            uniform_bytes,
            vk::BufferUsageFlags::UNIFORM_BUFFER,
            false,
        )?;
        let instance_bytes: Vec<u8> = planned.instances.iter().flat_map(|value| value.to_ne_bytes()).collect();
        FrameRecorder::ensure(
            &context,
            &mut self.recorder.instances,
            instance_bytes.len().max(16),
            vk::BufferUsageFlags::VERTEX_BUFFER,
            false,
        )?;
        self.recorder.uniforms.write(0, &planned.uniforms);
        self.recorder.instances.write(0, &instance_bytes);
        for (saved, visible) in [
            (&mut self.overlay_under, planned.overlay_visible),
            (&mut self.cursor_under, planned.cursor_visible),
        ] {
            saved.rect = visible;
            if let Some(rect) = visible {
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
        }

        self.recorder.begin()?;
        let uniform_set = self.recorder.arena.allocate(self.pipelines.uniform_layout)?;
        let block = [vk::DescriptorBufferInfo::default()
            .buffer(self.recorder.uniforms.raw())
            .offset(0)
            .range(BLOCK_RANGE)];
        let mut sets: Vec<(TextureSlot, vk::DescriptorSet)> = Vec::with_capacity(uploads.len());
        for upload in uploads {
            let set = self.recorder.arena.allocate(self.pipelines.texture_layout)?;
            let image = [vk::DescriptorImageInfo::default()
                .image_view(upload.image.view())
                .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)];
            let sampler = [vk::DescriptorImageInfo::default().sampler(if upload.slot.nearest() {
                self.pipelines.nearest
            } else {
                self.pipelines.linear
            })];
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
            unsafe { context.device.update_descriptor_sets(&writes, &[]) };
            sets.push((upload.slot, set));
        }
        let write_uniform = [vk::WriteDescriptorSet::default()
            .dst_set(uniform_set)
            .dst_binding(0)
            .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
            .buffer_info(&block)];
        // SAFETY: the set and buffer are live and the range lies inside the buffer.
        unsafe { context.device.update_descriptor_sets(&write_uniform, &[]) };

        for upload in uploads {
            self.upload(upload);
        }
        let background = premultiplied(frame.background());
        self.pass(
            planned,
            &planned.draws[..planned.layers_end],
            uniform_set,
            &sets,
            Some(background),
        );
        if !planned.overlay_under.is_empty() {
            self.save_under(&planned.overlay_under, self.overlay_under.rect, true);
        }
        if planned.overlay_end > planned.layers_end {
            self.pass(
                planned,
                &planned.draws[planned.layers_end..planned.overlay_end],
                uniform_set,
                &sets,
                None,
            );
        }
        if !planned.cursor_under.is_empty() {
            self.save_under(&planned.cursor_under, self.cursor_under.rect, false);
        }
        if planned.draws.len() > planned.overlay_end {
            self.pass(planned, &planned.draws[planned.overlay_end..], uniform_set, &sets, None);
        }
        self.recorder.submit()
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

    /// Copies the parts of the target the overlay or the cursor is about to cover.
    fn save_under(&self, parts: &[Rect], visible: Option<Rect>, overlay: bool) {
        let (Some(visible), saved) = (
            visible,
            if overlay {
                &self.overlay_under
            } else {
                &self.cursor_under
            },
        ) else {
            return;
        };
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

    /// Records one render pass: an optional clear of the damage, then the draws.
    fn pass(
        &self,
        planned: &FramePlan,
        draws: &[super::plan::PlannedDraw],
        uniform_set: vk::DescriptorSet,
        sets: &[(TextureSlot, vk::DescriptorSet)],
        clear: Option<[f32; 4]>,
    ) {
        let device = &self.context.device;
        let cmd = self.recorder.cmd;
        let size = self.physical;
        let begin = vk::RenderPassBeginInfo::default()
            .render_pass(self.pipelines.render_pass)
            .framebuffer(self.target.framebuffer)
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
        // stays inside the buffers bound here, whose sizes `record` ensured for the plan, and
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
            if let Some(colour) = clear {
                let attachment = vk::ClearAttachment {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    color_attachment: 0,
                    clear_value: vk::ClearValue {
                        color: vk::ClearColorValue { float32: colour },
                    },
                };
                let rects: Vec<vk::ClearRect> = planned
                    .damage
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
                    &[uniform_set],
                    &[draw.uniform_offset],
                );
                if let Some(slot) = draw.slot
                    && let Some((_, set)) = sets.iter().find(|(candidate, _)| *candidate == slot)
                {
                    device.cmd_bind_descriptor_sets(
                        cmd,
                        vk::PipelineBindPoint::GRAPHICS,
                        self.pipelines.layout,
                        1,
                        &[*set],
                        &[],
                    );
                }
                device.cmd_draw(cmd, 4, 1, 0, draw.instance);
            }
            device.cmd_end_render_pass(cmd);
        }
    }

    /// Reads the target back, with the overlay's pixels put back when `hide_overlay` and the
    /// cursor's when `!with_cursor`.
    pub(super) fn read_target(&mut self, hide_overlay: bool, with_cursor: bool) -> Result<Image, vk::Result> {
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
        // The cursor was drawn last, so it goes back first, over the overlay.
        if !with_cursor && let (Some(rect), Some(saved)) = (self.cursor_under.rect, self.cursor_under.bytes()) {
            paste(&mut pixels, stride, rect, saved);
        }
        if hide_overlay && let (Some(rect), Some(saved)) = (self.overlay_under.rect, self.overlay_under.bytes()) {
            paste(&mut pixels, stride, rect, saved);
        }
        Image::new(size, stride, pixels).ok_or(vk::Result::ERROR_UNKNOWN)
    }

    /// The renderer error a failed call becomes; a lost device is remembered.
    pub(super) fn fail(&mut self, result: vk::Result) -> RenderError {
        let error = classify(result);
        if error == RenderError::Lost {
            self.lost = true;
        }
        log::warn!(area = "render", ?result, ?error, "the Vulkan renderer failed a frame");
        error
    }
}

/// The layers of a frame that have pixels to upload, with the slot each fills.
fn sources(frame: &OutputFrame) -> Vec<(TextureSlot, PixelSource<'_>)> {
    let mut found = Vec::new();
    for (index, layer) in frame.layers().iter().enumerate() {
        let Layer::Surface(surface) = layer else {
            continue;
        };
        if surface.model.is_some() {
            continue;
        }
        match shm::shape(&surface.buffer) {
            Ok(shape) => found.push((
                TextureSlot::Layer(index),
                PixelSource::Shm {
                    buffer: &surface.buffer,
                    shape,
                },
            )),
            Err(error) => {
                log::warn!(area = "render", %error, "a client buffer could not be read; the layer is skipped")
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
