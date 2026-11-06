#![allow(unsafe_code)]
//! Recording commands, submitting them and waiting for them.
//!
//! A frame is one command buffer, submitted once and waited for before the call returns. That is
//! the honest cost of an offscreen renderer whose output is read by the CPU: there is nothing
//! to overlap with, and a fence per submission is what tells the renderer the device is lost.
//! Presentation to a display will not wait like this, which is a later slice's work.
//!
//! # Layouts
//!
//! The target lives in `COLOR_ATTACHMENT_OPTIMAL` between submissions. A copy out of it moves it
//! to `TRANSFER_SRC_OPTIMAL` and back with explicit barriers; the render pass has the matching
//! subpass dependencies.

use std::rc::Rc;

use ash::vk;
use gnoblin_log::warn;
use gnoblin_types::Size;

use super::Vulkan;
use super::context::{TARGET_FORMAT, VkBuffer, VkContext, VkImage, colour_range};
use super::error::classify;
use super::pipeline::DescriptorArena;
use crate::render::RenderError;
use crate::render::image::Image;

/// How long a frame may take before the device is treated as lost, in nanoseconds.
const FRAME_TIMEOUT: u64 = 10_000_000_000;

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

/// The command buffer, the fence and the readback buffer.
pub(super) struct FrameRecorder {
    context: Rc<VkContext>,
    pool: vk::CommandPool,
    cmd: vk::CommandBuffer,
    fence: vk::Fence,
    arena: DescriptorArena,
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
        let made = Self {
            context: Rc::clone(context),
            pool,
            cmd,
            fence,
            arena: DescriptorArena::new(context),
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

impl Vulkan {
    /// Reads the target back.
    pub(super) fn read_target(&mut self) -> Result<Image, vk::Result> {
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
        let pixels = pixels.to_vec();
        let stride = size.width as usize * 4;
        Image::new(size, stride, pixels).ok_or(vk::Result::ERROR_UNKNOWN)
    }

    /// The renderer error a failed call becomes; a lost device is remembered.
    pub(super) fn fail(&mut self, result: vk::Result) -> RenderError {
        let error = classify(result);
        if error == RenderError::Lost {
            self.lost = true;
        }
        warn!(?result, ?error, "[render] the Vulkan renderer failed a frame");
        error
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
