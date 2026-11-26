#![allow(unsafe_code)]
//! Frames in flight: the timeline every submission signals, the queue that holds objects until the GPU is done with them,
//! and the ring of per-frame resources an output records into.
//!
//! # The model
//!
//! A device has one timeline semaphore. Every submission signals the next value, so a value stands for "this frame and every
//! frame before it have finished". Three things follow:
//!
//! - **`draw` does not wait.** It records into the output's next [`InFlightFrame`], submits and returns. Only a frame that must
//!   read back what it drew (a capture) waits, and only for its own value.
//! - **A slot is reused when its value has passed.** An output has two slots, so at most two of its frames are in flight; the
//!   one that has to be waited for is the older, and in a steady session it finished long ago.
//! - **An object a frame used is destroyed when the timeline passes that frame's value.** The [`RetireQueue`] holds it until
//!   then. That replaces waiting for the device to go idle before a destroy.
//!
//! The slot owns what the CPU writes before a frame and the GPU reads during it: the command buffer, the descriptor sets of
//! the frame, the uniform and instance buffers and the staging buffer. Two slots mean one frame's data is never overwritten
//! while the other still reads it.
//!
//! # Failure modes
//!
//! 1. **A slot's buffers are overwritten while the GPU reads them.** A slot is waited for ([`InFlightFrame::begin`]) before
//!    anything of it is written: `tests::a_frame_returns_before_the_gpu_finishes`.
//! 2. **An object is destroyed under a frame that reads it.** It goes to the retire queue with the value of the last frame
//!    that used it: `tests::a_texture_is_destroyed_only_after_its_last_frame_retires`.
//! 3. **A hang is waited for forever.** Every wait has the context's timeout, and a timeout is a lost device:
//!    `context::WAIT_TIMEOUT`.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;

use ash::vk;

use super::context::VkContext;
use super::memory::{VkBuffer, colour_range};
use super::pipeline::DescriptorArena;

/// How many frames an output may have in flight.
pub(super) const FRAMES_IN_FLIGHT: usize = 2;

/// The size of a slot's staging, uniform and instance buffers when it is made, and what a trim leaves.
pub(super) const MIN_BUFFER: usize = 4096;

/// How many timestamps a frame can write: two for each draw of a shader pass.
pub(super) const STAMP_QUERIES: u32 = 512;

/// The device's timeline semaphore: its value is the number of the last frame the GPU finished, and the next frame signals
/// one more.
pub(super) struct Timeline {
    semaphore: vk::Semaphore,
    /// The highest value a submission has been asked to signal.
    submitted: Cell<u64>,
}

impl Timeline {
    /// A timeline at value 0.
    ///
    /// # Errors
    ///
    /// The driver's result.
    pub(super) fn new(device: &ash::Device) -> Result<Self, vk::Result> {
        let mut kind = vk::SemaphoreTypeCreateInfo::default()
            .semaphore_type(vk::SemaphoreType::TIMELINE)
            .initial_value(0);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut kind);
        // SAFETY: the device was created with timeline semaphores enabled.
        let semaphore = unsafe { device.create_semaphore(&info, None) }?;
        Ok(Self {
            semaphore,
            submitted: Cell::new(0),
        })
    }

    /// The semaphore a submission signals.
    pub(super) fn semaphore(&self) -> vk::Semaphore {
        self.semaphore
    }

    /// The value of the last submission: what a thing that needs every frame so far to finish waits for.
    pub(super) fn submitted(&self) -> u64 {
        self.submitted.get()
    }

    /// Notes that a submission signals `value`.
    pub(super) fn record(&self, value: u64) {
        self.submitted.set(value);
    }

    /// Destroys the semaphore. The caller idled the device.
    pub(super) fn destroy(&self, device: &ash::Device) {
        // SAFETY: nothing waits on the semaphore: the caller waited for the device to be idle.
        unsafe { device.destroy_semaphore(self.semaphore, None) };
    }
}

/// Objects waiting for the GPU to finish with them, each with the timeline value after which it may go.
///
/// An owner that drops an object a frame in flight may read hands it here instead, with the value of the last frame that
/// used it, and [`RetireQueue::release`] drops it when the timeline passes that value. The objects destroy themselves on
/// drop, so the queue needs to know nothing of what they are.
#[derive(Default)]
pub(super) struct RetireQueue {
    items: Vec<(u64, Box<dyn Any>)>,
}

impl RetireQueue {
    /// Holds `object` until the timeline reaches `value`.
    pub(super) fn push(&mut self, value: u64, object: impl Any) {
        self.items.push((value, Box::new(object)));
    }

    /// Drops what the timeline has passed. Returns how many objects went.
    pub(super) fn release(&mut self, completed: u64) -> usize {
        let before = self.items.len();
        self.items.retain(|(value, _)| *value > completed);
        before - self.items.len()
    }

    /// How many objects wait.
    pub(super) fn len(&self) -> usize {
        self.items.len()
    }

    /// Drops everything. The caller idled the device.
    pub(super) fn clear(&mut self) {
        self.items.clear();
    }
}

/// The timestamp queries of the device: the pool, and what a tick of it is worth.
pub(super) struct TimestampQuery {
    pub(super) pool: vk::QueryPool,
    /// The length of a tick in nanoseconds.
    pub(super) nanoseconds: f64,
    /// The bits of a timestamp the queue writes.
    pub(super) mask: u64,
}

/// The pool of timestamp queries, when the queue writes any.
fn make_query(context: &VkContext) -> Option<TimestampQuery> {
    if context.timestamp_bits == 0 {
        return None;
    }
    let info = vk::QueryPoolCreateInfo::default()
        .query_type(vk::QueryType::TIMESTAMP)
        .query_count(STAMP_QUERIES);
    // SAFETY: `info` is a valid description; the pool is destroyed with the slot.
    let pool = unsafe { context.device.create_query_pool(&info, None) }.ok()?;
    let bits = context.timestamp_bits;
    Some(TimestampQuery {
        pool,
        nanoseconds: f64::from(context.limits.timestamp_period),
        mask: if bits >= 64 { u64::MAX } else { (1_u64 << bits) - 1 },
    })
}

/// One frame's worth of resources: the command buffer, the descriptor sets and the host-visible buffers a frame writes.
pub(super) struct InFlightFrame {
    context: Rc<VkContext>,
    pool: vk::CommandPool,
    pub(super) cmd: vk::CommandBuffer,
    pub(super) arena: DescriptorArena,
    pub(super) uniforms: VkBuffer,
    /// The shader passes' own blocks: the second uniform binding of group 0.
    pub(super) user_uniforms: VkBuffer,
    pub(super) instances: VkBuffer,
    pub(super) staging: VkBuffer,
    /// The timestamp queries, when the queue writes them.
    pub(super) query: Option<TimestampQuery>,
    /// The timeline value this slot's last submission signals: 0 before it has submitted.
    pub(super) value: u64,
}

impl InFlightFrame {
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
        // From here the guard destroys the pool if a buffer cannot be made.
        let mut guard = PoolGuard {
            context: Rc::clone(context),
            pool,
        };
        let made = Self {
            context: Rc::clone(context),
            pool,
            cmd,
            arena: DescriptorArena::new(context),
            uniforms: VkBuffer::new(context, MIN_BUFFER, vk::BufferUsageFlags::UNIFORM_BUFFER, false)?,
            user_uniforms: VkBuffer::new(context, MIN_BUFFER, vk::BufferUsageFlags::UNIFORM_BUFFER, false)?,
            instances: VkBuffer::new(context, MIN_BUFFER, vk::BufferUsageFlags::VERTEX_BUFFER, false)?,
            staging: VkBuffer::new(context, MIN_BUFFER, vk::BufferUsageFlags::TRANSFER_SRC, false)?,
            query: make_query(context),
            value: 0,
        };
        guard.pool = vk::CommandPool::null();
        Ok(made)
    }

    /// Starts recording a new frame: waits until the slot's previous frame has finished, then resets the command buffer and
    /// the descriptors. Nothing the slot owns is written before this.
    ///
    /// # Errors
    ///
    /// `TIMEOUT` when the previous frame did not finish in time, which the caller reports as a lost device, and the driver's
    /// result otherwise.
    pub(super) fn begin(&mut self) -> Result<(), vk::Result> {
        self.context.wait(self.value)?;
        let device = &self.context.device;
        // SAFETY: the slot's previous submission has finished, so neither the buffer nor its sets are in use, and the pool
        // allows resetting its buffers.
        unsafe {
            device.reset_command_buffer(self.cmd, vk::CommandBufferResetFlags::empty())?;
            device.begin_command_buffer(
                self.cmd,
                &vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
            if let Some(query) = &self.query {
                device.cmd_reset_query_pool(self.cmd, query.pool, 0, STAMP_QUERIES);
            }
        }
        self.arena.reset()?;
        self.barrier_all();
        Ok(())
    }

    /// Ends the command buffer and submits it, and says which timeline value it signals.
    ///
    /// # Errors
    ///
    /// The driver's result.
    pub(super) fn submit(&mut self) -> Result<u64, vk::Result> {
        // SAFETY: the buffer is in the recording state and holds complete commands.
        unsafe { self.context.device.end_command_buffer(self.cmd) }?;
        let value = self.context.submit(self.cmd)?;
        self.value = value;
        Ok(value)
    }

    /// A memory dependency from every earlier command on the queue to every later one.
    ///
    /// A frame begins with it, so what the frames in flight beside this one wrote (the target, a texture, the pixels
    /// saved under the cursor) is finished and visible before this frame reads or writes it again. The frames share one
    /// queue and mostly overlap on the CPU side, so this costs the GPU nothing a frame did not wait for anyway.
    fn barrier_all(&self) {
        let memory = vk::MemoryBarrier::default()
            .src_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
            .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE);
        // SAFETY: the command buffer is recording.
        unsafe {
            self.context.device.cmd_pipeline_barrier(
                self.cmd,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[memory],
                &[],
                &[],
            );
        }
    }

    /// An image layout transition with the stages and accesses on each side.
    pub(super) fn barrier(
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

    /// Grows `buffer` to hold at least `bytes`. The slot is idle, so the old buffer is dropped at once.
    pub(super) fn ensure(
        context: &Rc<VkContext>,
        buffer: &mut VkBuffer,
        bytes: usize,
        usage: vk::BufferUsageFlags,
    ) -> Result<(), vk::Result> {
        if buffer.size() < bytes {
            *buffer = VkBuffer::new(context, bytes.next_power_of_two(), usage, false)?;
        }
        Ok(())
    }

    /// The host-visible bytes the slot holds now: its uniform, shader-block, instance and staging buffers.
    pub(super) fn bytes(&self) -> u64 {
        [&self.uniforms, &self.user_uniforms, &self.instances, &self.staging]
            .iter()
            .map(|buffer| buffer.size() as u64)
            .fold(0, u64::saturating_add)
    }

    /// Gives back what a past peak left behind: a staging buffer above `staging_retain` bytes is replaced by the smallest a
    /// slot starts with, and the next frame that needs more grows it again. The buffer the slot just submitted a frame with
    /// is returned for the caller to retire with that frame's value: the GPU may still be reading it.
    ///
    /// [`InFlightFrame::ensure`] only grows, so without this one large frame (a window opened at a huge size, an output
    /// supersampled and then not) keeps its host-visible memory for the life of the renderer. A buffer that cannot be
    /// replaced is kept: a trim that fails costs memory, never a frame.
    pub(super) fn trim(&mut self, staging_retain: usize) -> Option<VkBuffer> {
        if self.staging.size() <= staging_retain {
            return None;
        }
        let small = VkBuffer::new(&self.context, MIN_BUFFER, vk::BufferUsageFlags::TRANSFER_SRC, false).ok()?;
        Some(std::mem::replace(&mut self.staging, small))
    }
}

impl Drop for InFlightFrame {
    fn drop(&mut self) {
        // SAFETY: the owner waited for the slot's last frame; destroying the pool frees its command buffer.
        unsafe {
            if let Some(query) = &self.query {
                self.context.device.destroy_query_pool(query.pool, None);
            }
            self.context.device.destroy_command_pool(self.pool, None);
        }
    }
}

/// Destroys a command pool if a slot could not be completed.
struct PoolGuard {
    context: Rc<VkContext>,
    pool: vk::CommandPool,
}

impl Drop for PoolGuard {
    fn drop(&mut self) {
        // SAFETY: nothing was submitted; a null handle is a no-op.
        unsafe { self.context.device.destroy_command_pool(self.pool, None) };
    }
}

/// The slots of one output, used in turn.
pub(super) struct FrameRing {
    slots: [InFlightFrame; FRAMES_IN_FLIGHT],
    /// The slot the next frame records into.
    next: usize,
}

impl FrameRing {
    pub(super) fn new(context: &Rc<VkContext>) -> Result<Self, vk::Result> {
        Ok(Self {
            slots: [InFlightFrame::new(context)?, InFlightFrame::new(context)?],
            next: 0,
        })
    }

    /// The index of the slot the next frame records into, which is then the one after.
    pub(super) fn acquire(&mut self) -> usize {
        let index = self.next;
        self.next = (index + 1) % FRAMES_IN_FLIGHT;
        index
    }

    /// A slot.
    pub(super) fn slot(&self, index: usize) -> &InFlightFrame {
        &self.slots[index]
    }

    /// A slot, to record into.
    pub(super) fn slot_mut(&mut self, index: usize) -> &mut InFlightFrame {
        &mut self.slots[index]
    }

    /// The host-visible bytes every slot holds.
    pub(super) fn bytes(&self) -> u64 {
        self.slots.iter().map(InFlightFrame::bytes).fold(0, u64::saturating_add)
    }

    /// The value of the newest frame any slot has submitted: what the output waits for before it is destroyed.
    pub(super) fn newest(&self) -> u64 {
        self.slots.iter().map(|slot| slot.value).max().unwrap_or(0)
    }
}
