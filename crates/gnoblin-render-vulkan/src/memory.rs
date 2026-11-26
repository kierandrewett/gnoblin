#![allow(unsafe_code)]
//! Device memory: the allocator every buffer and image is sub-allocated from, the counters the renderer reports, and the two
//! objects that own memory ([`VkBuffer`] and [`VkImage`]).
//!
//! # What is sub-allocated
//!
//! Every device-local image and every host-visible buffer is a range of a few large `VkDeviceMemory` blocks, made by
//! `gpu-allocator` (the Vulkan backend, on the `ash` the renderer already uses). A frame creates and destroys many small
//! objects, and one `vkAllocateMemory` each would hit the driver's allocation limit (4096 on most) and cost a kernel call
//! every time. The blocks are kept while any range of them is in use, so a steady session asks the driver for no memory.
//!
//! An image or buffer larger than a block gets a block of its own, which the allocator frees with it. Imported memory
//! (a client's dmabuf) is never from the allocator: it is the client's, and is made by `dmabuf.rs`.
//!
//! # What is counted
//!
//! [`RenderCounters`] hold what [`Stats`](crate::Stats) reports. They are `Cell`s: the renderer is single-threaded, and the
//! shader worker thread touches only the pipelines.
//!
//! # Failure modes
//!
//! 1. **The allocator is used after the device.** It is taken out of the context and dropped before the device is destroyed:
//!    `context::VkContext::drop`.
//! 2. **A block is freed under a range the GPU still reads.** Nothing here frees on its own schedule: an object is dropped
//!    by its owner, and an owner whose object a frame in flight reads hands it to the retire queue (`sync.rs`) first:
//!    `tests::a_texture_is_destroyed_only_after_its_last_frame_retires`.
//! 3. **A steady session asks the driver for memory.** The blocks are kept and ranges are reused, and the counter of device
//!    allocations does not move across steady frames: `tests::a_steady_session_allocates_no_device_memory`.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::rc::Rc;

use ash::vk;
use gpu_allocator::vulkan::{Allocation, AllocationCreateDesc, AllocationScheme, Allocator, AllocatorCreateDesc};
use gpu_allocator::{AllocationError, AllocationSizes, AllocatorDebugSettings, MemoryLocation};

use super::context::VkContext;
use super::error::VulkanError;

/// The size of a block of device-local memory: what one large allocation takes from the heap.
const DEVICE_BLOCK: u64 = 64 << 20;

/// The size of a block of host-visible memory.
const HOST_BLOCK: u64 = 16 << 20;

/// What the renderer counts, for [`Stats`](crate::Stats) and for the tests that assert a frame did nothing.
#[derive(Default)]
pub(super) struct RenderCounters {
    /// Queue submissions.
    pub submits: Cell<u64>,
    /// Bytes copied into the staging buffers for textures.
    pub uploaded: Cell<u64>,
    /// Textures uploaded, whole or in part.
    pub uploads: Cell<u64>,
    /// Times the allocator took memory from the driver: a block made, or an object that got one of its own.
    pub device_allocations: Cell<u64>,
    /// Bytes of ranges handed out and not yet given back.
    pub allocated: Cell<u64>,
    /// Images made.
    pub images_created: Cell<u64>,
    /// Images destroyed.
    pub images_destroyed: Cell<u64>,
}

impl RenderCounters {
    /// Adds `by` to a counter.
    pub(super) fn add(counter: &Cell<u64>, by: u64) {
        counter.set(counter.get().saturating_add(by));
    }
}

/// The allocator, and what it has done.
pub(super) struct Memory {
    /// `None` once the device is going away.
    allocator: RefCell<Option<Allocator>>,
    pub(super) counters: RenderCounters,
}

impl Memory {
    /// An allocator for `device`, which the caller made from `physical` on `instance`.
    pub(super) fn new(
        instance: &ash::Instance,
        device: &ash::Device,
        physical: vk::PhysicalDevice,
    ) -> Result<Self, VulkanError> {
        let allocator = Allocator::new(&AllocatorCreateDesc {
            instance: instance.clone(),
            device: device.clone(),
            physical_device: physical,
            debug_settings: AllocatorDebugSettings::default(),
            buffer_device_address: false,
            allocation_sizes: AllocationSizes::new(DEVICE_BLOCK, HOST_BLOCK),
        })
        .map_err(|_| VulkanError::driver("gpu-allocator", vk::Result::ERROR_INITIALIZATION_FAILED))?;
        Ok(Self {
            allocator: RefCell::new(Some(allocator)),
            counters: RenderCounters::default(),
        })
    }

    /// A range of memory for `requirements`, `linear` for a buffer and a linear image and not for an optimal one.
    ///
    /// # Errors
    ///
    /// The Vulkan result the failure amounts to: out of device memory when the heap has none, and an unknown error when
    /// the allocator is gone.
    pub(super) fn allocate(
        &self,
        name: &'static str,
        requirements: vk::MemoryRequirements,
        location: MemoryLocation,
        linear: bool,
    ) -> Result<Allocation, vk::Result> {
        let mut guard = self.allocator.borrow_mut();
        let allocator = guard.as_mut().ok_or(vk::Result::ERROR_UNKNOWN)?;
        let before = allocator.capacity();
        let allocation = allocator
            .allocate(&AllocationCreateDesc {
                name,
                requirements,
                location,
                linear,
                allocation_scheme: AllocationScheme::GpuAllocatorManaged,
            })
            .map_err(|error| match error {
                AllocationError::OutOfMemory => vk::Result::ERROR_OUT_OF_DEVICE_MEMORY,
                _ => vk::Result::ERROR_UNKNOWN,
            })?;
        if allocator.capacity() > before {
            RenderCounters::add(&self.counters.device_allocations, 1);
        }
        RenderCounters::add(&self.counters.allocated, allocation.size());
        Ok(allocation)
    }

    /// Gives a range back. A block with nothing left in it is freed by the allocator unless it is the last.
    pub(super) fn free(&self, allocation: Allocation) {
        let size = allocation.size();
        if let Some(allocator) = self.allocator.borrow_mut().as_mut() {
            let _ = allocator.free(allocation);
        }
        self.counters
            .allocated
            .set(self.counters.allocated.get().saturating_sub(size));
    }

    /// The bytes of device memory the allocator holds: every block, used or not.
    pub(super) fn capacity(&self) -> u64 {
        self.allocator.borrow().as_ref().map_or(0, Allocator::capacity)
    }

    /// Frees every block. The caller destroys the device right after, and nothing allocates again.
    pub(super) fn shutdown(&self) {
        drop(self.allocator.borrow_mut().take());
    }
}

/// A buffer in host-visible, coherent memory, mapped for as long as it lives.
pub(super) struct VkBuffer {
    context: Rc<VkContext>,
    raw: vk::Buffer,
    allocation: Option<Allocation>,
    size: usize,
    mapped: NonNull<u8>,
}

impl VkBuffer {
    /// A buffer of at least `size` bytes.
    ///
    /// `readback` prefers host-cached memory, which makes the CPU reading it faster; the others prefer memory the GPU
    /// reads fast.
    ///
    /// # Errors
    ///
    /// The driver's result, which is an out-of-memory one when the memory is not there.
    pub(super) fn new(
        context: &Rc<VkContext>,
        size: usize,
        usage: vk::BufferUsageFlags,
        readback: bool,
    ) -> Result<Self, vk::Result> {
        let size = size.max(16);
        let info = vk::BufferCreateInfo::default()
            .size(size as u64)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let device = &context.device;
        // SAFETY: `info` is a valid, exclusive buffer description.
        let raw = unsafe { device.create_buffer(&info, None) }?;
        // SAFETY: `raw` was just created on this device.
        let requirements = unsafe { device.get_buffer_memory_requirements(raw) };
        let location = if readback {
            MemoryLocation::GpuToCpu
        } else {
            MemoryLocation::CpuToGpu
        };
        let allocation = match context.memory.allocate("gnoblin buffer", requirements, location, true) {
            Ok(allocation) => allocation,
            Err(result) => {
                // SAFETY: the buffer is unused.
                unsafe { device.destroy_buffer(raw, None) };
                return Err(result);
            }
        };
        // SAFETY: the range is at least as large as the buffer needs, host-visible, and is bound once.
        let bound = unsafe { device.bind_buffer_memory(raw, allocation.memory(), allocation.offset()) };
        let Some(mapped) = bound.is_ok().then(|| allocation.mapped_ptr()).flatten() else {
            // SAFETY: neither handle is in use.
            unsafe { device.destroy_buffer(raw, None) };
            context.memory.free(allocation);
            return Err(bound.err().unwrap_or(vk::Result::ERROR_MEMORY_MAP_FAILED));
        };
        Ok(Self {
            context: Rc::clone(context),
            raw,
            allocation: Some(allocation),
            size,
            mapped: mapped.cast(),
        })
    }

    /// The buffer handle.
    pub(super) fn raw(&self) -> vk::Buffer {
        self.raw
    }

    /// The buffer's size in bytes.
    pub(super) fn size(&self) -> usize {
        self.size
    }

    /// Copies `bytes` to `offset`. Returns false, writing nothing, when they do not fit.
    pub(super) fn write(&mut self, offset: usize, bytes: &[u8]) -> bool {
        let Some(end) = offset.checked_add(bytes.len()).filter(|end| *end <= self.size) else {
            return false;
        };
        debug_assert!(end <= self.size);
        // SAFETY: `mapped` points at `size` bytes of mapped memory that live as long as `self`,
        // the range was bounds-checked above and `bytes` is a distinct host allocation.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.mapped.as_ptr().add(offset), bytes.len()) };
        true
    }

    /// The bytes from `offset`, for the CPU to fill before the frame that reads them is recorded.
    ///
    /// [`None`] when they do not fit.
    pub(super) fn as_mut_slice(&mut self, offset: usize, length: usize) -> Option<&mut [u8]> {
        let end = offset.checked_add(length).filter(|end| *end <= self.size)?;
        debug_assert!(end <= self.size);
        // SAFETY: in range by the check above; the mapping lives as long as `self` and no
        // submitted work reads this buffer while the renderer holds `&mut self`: a buffer a frame in flight reads is not
        // written until that frame's timeline value has been waited for.
        Some(unsafe { std::slice::from_raw_parts_mut(self.mapped.as_ptr().add(offset), length) })
    }

    /// The bytes from `offset`, which the device has finished writing.
    ///
    /// The caller has waited for the timeline value of the work that wrote them.
    pub(super) fn read(&self, offset: usize, length: usize) -> Option<&[u8]> {
        let end = offset.checked_add(length).filter(|end| *end <= self.size)?;
        debug_assert!(end <= self.size);
        // SAFETY: in range by the check above, and the memory is coherent and idle: no
        // submitted work writes it while the renderer holds `&self` after the wait.
        Some(unsafe { std::slice::from_raw_parts(self.mapped.as_ptr().add(offset), length) })
    }
}

impl Drop for VkBuffer {
    fn drop(&mut self) {
        // SAFETY: the owner waited for the frame that used the buffer, or retired it until that frame had finished; the
        // context outlives it.
        unsafe { self.context.device.destroy_buffer(self.raw, None) };
        if let Some(allocation) = self.allocation.take() {
            self.context.memory.free(allocation);
        }
    }
}

/// A 2D image with a view, in device-local memory.
pub(super) struct VkImage {
    context: Rc<VkContext>,
    raw: vk::Image,
    view: vk::ImageView,
    allocation: Option<Allocation>,
    width: u32,
    height: u32,
    format: vk::Format,
}

impl VkImage {
    /// An optimal-tiling image of `width` by `height` pixels.
    ///
    /// `opaque` reads the fourth channel as one, for a format whose fourth byte is padding.
    ///
    /// # Errors
    ///
    /// The driver's result. An image larger than the device allows is refused with an
    /// out-of-memory result, which is what it amounts to.
    pub(super) fn new(
        context: &Rc<VkContext>,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        opaque: bool,
    ) -> Result<Self, vk::Result> {
        Self::create(
            context,
            width,
            height,
            format,
            usage,
            opaque,
            vk::ImageAspectFlags::COLOR,
        )
    }

    /// A depth-stencil image of `format`, which a pass clears and tests against and nothing samples.
    ///
    /// # Errors
    ///
    /// As [`VkImage::new`].
    pub(super) fn depth_stencil(
        context: &Rc<VkContext>,
        width: u32,
        height: u32,
        format: vk::Format,
    ) -> Result<Self, vk::Result> {
        Self::create(
            context,
            width,
            height,
            format,
            vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT,
            false,
            vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL,
        )
    }

    fn create(
        context: &Rc<VkContext>,
        width: u32,
        height: u32,
        format: vk::Format,
        usage: vk::ImageUsageFlags,
        opaque: bool,
        aspect: vk::ImageAspectFlags,
    ) -> Result<Self, vk::Result> {
        #[cfg(test)]
        if let Some(result) = context.texture_fault() {
            return Err(result);
        }
        let largest = context.limits.max_image_dimension2_d;
        if width == 0 || height == 0 || width > largest || height > largest {
            return Err(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY);
        }
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        let device = &context.device;
        // SAFETY: `info` is a valid 2D image description within the device's limits.
        let raw = unsafe { device.create_image(&info, None) }?;
        // SAFETY: `raw` was just created on this device.
        let requirements = unsafe { device.get_image_memory_requirements(raw) };
        let allocation = match context
            .memory
            .allocate("gnoblin image", requirements, MemoryLocation::GpuOnly, false)
        {
            Ok(allocation) => allocation,
            Err(result) => {
                // SAFETY: the image is unused.
                unsafe { device.destroy_image(raw, None) };
                return Err(result);
            }
        };
        let alpha = if opaque {
            vk::ComponentSwizzle::ONE
        } else {
            vk::ComponentSwizzle::IDENTITY
        };
        let view_info = vk::ImageViewCreateInfo::default()
            .image(raw)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(format)
            .components(vk::ComponentMapping {
                r: vk::ComponentSwizzle::IDENTITY,
                g: vk::ComponentSwizzle::IDENTITY,
                b: vk::ComponentSwizzle::IDENTITY,
                a: alpha,
            })
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: aspect,
                ..colour_range()
            });
        // SAFETY: the memory is bound once, before the view is made over the image.
        let view = unsafe {
            device
                .bind_image_memory(raw, allocation.memory(), allocation.offset())
                .and_then(|()| device.create_image_view(&view_info, None))
        };
        match view {
            Ok(view) => {
                RenderCounters::add(&context.memory.counters.images_created, 1);
                Ok(Self {
                    context: Rc::clone(context),
                    raw,
                    view,
                    allocation: Some(allocation),
                    width,
                    height,
                    format,
                })
            }
            Err(result) => {
                // SAFETY: neither handle is in use.
                unsafe { device.destroy_image(raw, None) };
                context.memory.free(allocation);
                Err(result)
            }
        }
    }

    /// The image handle.
    pub(super) fn raw(&self) -> vk::Image {
        self.raw
    }

    /// The image view over the whole image.
    pub(super) fn view(&self) -> vk::ImageView {
        self.view
    }

    /// Width in pixels.
    pub(super) fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    pub(super) fn height(&self) -> u32 {
        self.height
    }

    /// The format the image was made in.
    pub(super) fn format(&self) -> vk::Format {
        self.format
    }

    /// The bytes the image nominally holds: width times height times the four bytes of a pixel.
    pub(super) fn bytes(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height) * 4
    }
}

impl Drop for VkImage {
    fn drop(&mut self) {
        // SAFETY: the owner waited for the frame that used the image, or retired it until that frame had finished; the
        // context outlives it.
        unsafe {
            self.context.device.destroy_image_view(self.view, None);
            self.context.device.destroy_image(self.raw, None);
        }
        if let Some(allocation) = self.allocation.take() {
            self.context.memory.free(allocation);
        }
        RenderCounters::add(&self.context.memory.counters.images_destroyed, 1);
    }
}

/// The subresource range of a single-mip, single-layer colour image.
pub(super) fn colour_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    }
}
