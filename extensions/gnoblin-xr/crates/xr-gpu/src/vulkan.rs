//! The Vulkan side: one device shared with OpenXR, and a [`Presenter`] that copies into its swapchain images.
//!
//! This is the only module in the crate that uses `unsafe`. Every unsafe block says why it holds. The setup follows
//! `XR_KHR_vulkan_enable2`: the runtime creates the `VkInstance` and the `VkDevice` (so it can add the extensions it
//! needs), and the session is then created on that device. The loaded Vulkan library is `libvulkan` by name, through
//! `ash`'s loader, so a machine with no Vulkan fails with [`GpuError::Unavailable`] and not at link time.

#![allow(unsafe_code)]

use std::sync::Arc;

use ash::vk::{self, Handle};
use openxr as xr;

use crate::error::GpuError;
use crate::format::{Pixels, convert, packed_len};
use crate::presenter::{Presenter, Target};

/// The Vulkan API version the extension asks for. Vulkan 1.1 is what OpenXR runtimes on Linux require and offer.
const API_VERSION: u32 = vk::make_api_version(0, 1, 1, 0);

fn vulkan_error(call: &'static str, result: vk::Result) -> GpuError {
    GpuError::Vulkan {
        call,
        result: result.as_raw(),
    }
}

fn xr_error(call: &str, error: impl std::fmt::Debug) -> GpuError {
    GpuError::OpenXr(format!("{call}: {error:?}"))
}

/// A Vulkan instance, physical device, device and queue created through OpenXR.
///
/// It is shared by `Arc` between the OpenXR session, which must not outlive it, and the [`VulkanPresenter`]. The
/// device and the instance are destroyed when the last owner drops.
pub struct VulkanDevice {
    /// Keeps the loaded Vulkan library mapped for as long as the instance and device exist.
    _entry: ash::Entry,
    instance: ash::Instance,
    physical_device: vk::PhysicalDevice,
    device: ash::Device,
    queue_family_index: u32,
    queue: vk::Queue,
}

impl VulkanDevice {
    /// Creates the Vulkan instance and device for `system` through the OpenXR instance, which must have
    /// `XR_KHR_vulkan_enable2` enabled.
    ///
    /// # Errors
    ///
    /// No Vulkan loader, a runtime that needs a different Vulkan version, or a Vulkan or OpenXR call fails.
    pub fn create(xr_instance: &xr::Instance, system: xr::SystemId) -> Result<Arc<Self>, GpuError> {
        let requirements = xr_instance
            .graphics_requirements::<xr::Vulkan>(system)
            .map_err(|e| xr_error("xrGetVulkanGraphicsRequirements2KHR", e))?;
        let wanted = xr::Version::new(1, 1, 0);
        if wanted < requirements.min_api_version_supported
            || wanted.major() > requirements.max_api_version_supported.major()
        {
            return Err(GpuError::Unavailable(format!(
                "the runtime supports Vulkan {} to {}, and the extension needs 1.1",
                requirements.min_api_version_supported, requirements.max_api_version_supported
            )));
        }

        // SAFETY: `Entry::load` opens the platform Vulkan loader by name and resolves `vkGetInstanceProcAddr`. The
        // loader is trusted system code, and the returned `Entry` keeps the library mapped while it lives. It is
        // stored in the `VulkanDevice`, which outlives every handle made from it.
        let entry = unsafe { ash::Entry::load() }.map_err(|e| GpuError::Unavailable(e.to_string()))?;
        // SAFETY: both types are `extern "system"` function pointers with the same signature. They differ only in
        // how the handle types are spelled: ash's `vk::Instance` is a `u64` newtype and OpenXR's `VkInstance` is
        // `*const c_void`, and both are passed in the register of a pointer-sized integer on every 64-bit target.
        let get_instance_proc_addr = unsafe {
            std::mem::transmute::<vk::PFN_vkGetInstanceProcAddr, xr::sys::platform::VkGetInstanceProcAddr>(
                entry.static_fn().get_instance_proc_addr,
            )
        };

        let application = vk::ApplicationInfo::default()
            .application_name(c"gnoblin-xr")
            .engine_name(c"gnoblin-xr")
            .api_version(API_VERSION);
        let instance_info = vk::InstanceCreateInfo::default().application_info(&application);
        // SAFETY: `get_instance_proc_addr` comes from the Vulkan loader loaded above, and `instance_info` is a
        // complete `VkInstanceCreateInfo` that lives past the call. OpenXR adds the extensions the runtime needs.
        let raw_instance = unsafe {
            xr_instance.create_vulkan_instance(
                system,
                get_instance_proc_addr,
                std::ptr::from_ref(&instance_info).cast(),
            )
        }
        .map_err(|e| xr_error("xrCreateVulkanInstanceKHR", e))?
        .map_err(|code| vulkan_error("xrCreateVulkanInstanceKHR", vk::Result::from_raw(code)))?;
        // SAFETY: `raw_instance` is the valid instance OpenXR just created with the loader's proc address, and
        // `entry` supplies the matching function table.
        let instance =
            unsafe { ash::Instance::load(entry.static_fn(), vk::Instance::from_raw(raw_instance as usize as u64)) };

        match Self::create_device(
            xr_instance,
            system,
            &entry,
            &instance,
            get_instance_proc_addr,
            raw_instance,
        ) {
            Ok((physical_device, device, queue_family_index, queue)) => Ok(Arc::new(Self {
                _entry: entry,
                instance,
                physical_device,
                device,
                queue_family_index,
                queue,
            })),
            Err(error) => {
                // SAFETY: nothing else holds a handle derived from the instance, and no device was created.
                unsafe { instance.destroy_instance(None) };
                Err(error)
            }
        }
    }

    fn create_device(
        xr_instance: &xr::Instance,
        system: xr::SystemId,
        _entry: &ash::Entry,
        instance: &ash::Instance,
        get_instance_proc_addr: xr::sys::platform::VkGetInstanceProcAddr,
        raw_instance: xr::sys::platform::VkInstance,
    ) -> Result<(vk::PhysicalDevice, ash::Device, u32, vk::Queue), GpuError> {
        // SAFETY: `raw_instance` was returned by `create_vulkan_instance` on this OpenXR instance.
        let raw_physical = unsafe { xr_instance.vulkan_graphics_device(system, raw_instance) }
            .map_err(|e| xr_error("xrGetVulkanGraphicsDevice2KHR", e))?;
        let physical_device = vk::PhysicalDevice::from_raw(raw_physical as usize as u64);

        // SAFETY: `physical_device` belongs to `instance`.
        let families = unsafe { instance.get_physical_device_queue_family_properties(physical_device) };
        let queue_family_index = families
            .iter()
            .position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
            .and_then(|index| u32::try_from(index).ok())
            .ok_or_else(|| GpuError::Unavailable("the Vulkan device has no graphics queue".to_owned()))?;

        let priorities = [1.0_f32];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family_index)
            .queue_priorities(&priorities)];
        let device_info = vk::DeviceCreateInfo::default().queue_create_infos(&queues);
        // SAFETY: `physical_device` is the one OpenXR chose for `system`, `device_info` is a complete
        // `VkDeviceCreateInfo` that lives past the call, and `get_instance_proc_addr` is the loader's. OpenXR adds the
        // device extensions the runtime needs.
        let raw_device = unsafe {
            xr_instance.create_vulkan_device(
                system,
                get_instance_proc_addr,
                raw_physical,
                std::ptr::from_ref(&device_info).cast(),
            )
        }
        .map_err(|e| xr_error("xrCreateVulkanDeviceKHR", e))?
        .map_err(|code| vulkan_error("xrCreateVulkanDeviceKHR", vk::Result::from_raw(code)))?;
        // SAFETY: `raw_device` is the valid device OpenXR just created from `physical_device` of `instance`.
        let device = unsafe { ash::Device::load(instance.fp_v1_0(), vk::Device::from_raw(raw_device as usize as u64)) };
        // SAFETY: the device was created with one queue in `queue_family_index`, so queue 0 exists.
        let queue = unsafe { device.get_device_queue(queue_family_index, 0) };
        Ok((physical_device, device, queue_family_index, queue))
    }

    /// The handles the OpenXR session is created with (`XrGraphicsBindingVulkanKHR`).
    pub fn session_create_info(&self) -> xr::vulkan::SessionCreateInfo {
        xr::vulkan::SessionCreateInfo {
            instance: self.instance.handle().as_raw() as usize as xr::sys::platform::VkInstance,
            physical_device: self.physical_device.as_raw() as usize as xr::sys::platform::VkPhysicalDevice,
            device: self.device.handle().as_raw() as usize as xr::sys::platform::VkDevice,
            queue_family_index: self.queue_family_index,
            queue_index: 0,
        }
    }
}

impl Drop for VulkanDevice {
    fn drop(&mut self) {
        // SAFETY: the device and the instance were created by `create` and are destroyed once, device first. Every
        // other owner holds this struct by `Arc`, so none of their handles is in use after the last one is gone. The
        // OpenXR session must be destroyed before the last `Arc` drops: its owner keeps one to enforce that.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
        }
    }
}

/// Host-visible memory the pictures are converted into before the copy.
struct Staging {
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    mapped: *mut u8,
    capacity: usize,
}

/// A [`Presenter`] that copies pictures into swapchain images on a [`VulkanDevice`].
///
/// Each upload converts the picture into a mapped staging buffer, copies the buffer into the image with one
/// submission and waits for it, so the caller may release the image at once.
pub struct VulkanPresenter {
    device: Arc<VulkanDevice>,
    pool: vk::CommandPool,
    commands: vk::CommandBuffer,
    fence: vk::Fence,
    staging: Option<Staging>,
}

impl VulkanPresenter {
    /// Creates the command pool, command buffer and fence on `device`.
    ///
    /// # Errors
    ///
    /// A Vulkan call fails.
    pub fn new(device: Arc<VulkanDevice>) -> Result<Self, GpuError> {
        let d = &device.device;
        let pool_info = vk::CommandPoolCreateInfo::default()
            .queue_family_index(device.queue_family_index)
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER);
        // SAFETY: `d` is a valid device, and the create info is complete.
        let pool =
            unsafe { d.create_command_pool(&pool_info, None) }.map_err(|e| vulkan_error("vkCreateCommandPool", e))?;
        let allocate = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: `pool` was just created on `d`.
        let commands = match unsafe { d.allocate_command_buffers(&allocate) } {
            Ok(buffers) => buffers[0],
            Err(e) => {
                // SAFETY: nothing was allocated from the pool.
                unsafe { d.destroy_command_pool(pool, None) };
                return Err(vulkan_error("vkAllocateCommandBuffers", e));
            }
        };
        // SAFETY: `d` is a valid device.
        let fence = match unsafe { d.create_fence(&vk::FenceCreateInfo::default(), None) } {
            Ok(fence) => fence,
            Err(e) => {
                // SAFETY: the pool owns the only command buffer made from it, and none is pending.
                unsafe { d.destroy_command_pool(pool, None) };
                return Err(vulkan_error("vkCreateFence", e));
            }
        };
        Ok(Self {
            device,
            pool,
            commands,
            fence,
            staging: None,
        })
    }

    /// Makes sure the staging buffer holds at least `len` bytes.
    fn ensure_staging(&mut self, len: usize) -> Result<(), GpuError> {
        if self.staging.as_ref().is_some_and(|staging| staging.capacity >= len) {
            return Ok(());
        }
        self.free_staging();
        let d = &self.device.device;
        let info = vk::BufferCreateInfo::default()
            .size(len as u64)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC | vk::BufferUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: `d` is a valid device, and the create info is complete.
        let buffer = unsafe { d.create_buffer(&info, None) }.map_err(|e| vulkan_error("vkCreateBuffer", e))?;
        // SAFETY: `buffer` was just created on `d`.
        let requirements = unsafe { d.get_buffer_memory_requirements(buffer) };
        // SAFETY: the physical device belongs to the instance.
        let properties = unsafe {
            self.device
                .instance
                .get_physical_device_memory_properties(self.device.physical_device)
        };
        let wanted = vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT;
        let memory_type = (0..properties.memory_type_count).find(|&index| {
            requirements.memory_type_bits & (1 << index) != 0
                && properties.memory_types[index as usize].property_flags.contains(wanted)
        });
        let Some(memory_type) = memory_type else {
            // SAFETY: the buffer has no memory bound and no work uses it.
            unsafe { d.destroy_buffer(buffer, None) };
            return Err(GpuError::Unavailable(
                "the Vulkan device has no host-visible coherent memory".to_owned(),
            ));
        };
        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(memory_type);
        // SAFETY: `d` is a valid device, and `memory_type` is a valid index for it.
        let memory = match unsafe { d.allocate_memory(&allocate, None) } {
            Ok(memory) => memory,
            Err(e) => {
                // SAFETY: the buffer has no memory bound and no work uses it.
                unsafe { d.destroy_buffer(buffer, None) };
                return Err(vulkan_error("vkAllocateMemory", e));
            }
        };
        // SAFETY: `memory` is host visible, unmapped, at least `requirements.size` bytes, and `buffer` needs no
        // more than that from offset 0.
        let mapped = unsafe {
            d.bind_buffer_memory(buffer, memory, 0)
                .and_then(|()| d.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()))
        };
        let mapped = match mapped {
            Ok(pointer) => pointer.cast::<u8>(),
            Err(e) => {
                // SAFETY: no work uses the buffer or the memory, and the memory is not mapped.
                unsafe {
                    d.destroy_buffer(buffer, None);
                    d.free_memory(memory, None);
                }
                return Err(vulkan_error("vkBindBufferMemory", e));
            }
        };
        self.staging = Some(Staging {
            buffer,
            memory,
            mapped,
            capacity: len,
        });
        Ok(())
    }

    fn free_staging(&mut self) {
        if let Some(staging) = self.staging.take() {
            let d = &self.device.device;
            // SAFETY: every submission waits on its fence before it returns, so no work uses the buffer. The
            // memory was mapped once and is unmapped before it is freed.
            unsafe {
                d.unmap_memory(staging.memory);
                d.destroy_buffer(staging.buffer, None);
                d.free_memory(staging.memory, None);
            }
        }
    }

    /// Records `record` into the command buffer, submits it and waits for it to finish.
    fn submit(&mut self, record: impl FnOnce(&ash::Device, vk::CommandBuffer)) -> Result<(), GpuError> {
        let d = &self.device.device;
        let begin = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        let buffers = [self.commands];
        let submit = vk::SubmitInfo::default().command_buffers(&buffers);
        // SAFETY: the previous submission on this buffer and fence finished before it returned (`vkWaitForFences`),
        // so the buffer is not pending and the fence is signalled or unused. Only this presenter uses the queue
        // during the call, which is single-threaded, and the OpenXR runtime does not use it between its own calls.
        unsafe {
            d.reset_command_buffer(self.commands, vk::CommandBufferResetFlags::empty())
                .map_err(|e| vulkan_error("vkResetCommandBuffer", e))?;
            d.begin_command_buffer(self.commands, &begin)
                .map_err(|e| vulkan_error("vkBeginCommandBuffer", e))?;
            record(d, self.commands);
            d.end_command_buffer(self.commands)
                .map_err(|e| vulkan_error("vkEndCommandBuffer", e))?;
            d.reset_fences(&[self.fence])
                .map_err(|e| vulkan_error("vkResetFences", e))?;
            d.queue_submit(self.device.queue, &[submit], self.fence)
                .map_err(|e| vulkan_error("vkQueueSubmit", e))?;
            d.wait_for_fences(&[self.fence], true, u64::MAX)
                .map_err(|e| vulkan_error("vkWaitForFences", e))?;
        }
        Ok(())
    }

    /// Reads a swapchain image back into host memory, rows tightly packed in the image's format. For tests: the
    /// swapchain must have been created with the transfer-source usage. The image must be acquired, and it is left
    /// in `COLOR_ATTACHMENT_OPTIMAL`.
    ///
    /// # Errors
    ///
    /// A Vulkan call fails.
    pub fn read_back(&mut self, target: Target) -> Result<Vec<u8>, GpuError> {
        let len = packed_len(target.width, target.height);
        self.ensure_staging(len)?;
        let Some(staging) = self.staging.as_ref() else {
            return Err(GpuError::Unavailable("no staging buffer".to_owned()));
        };
        let (buffer, mapped) = (staging.buffer, staging.mapped);
        let image = vk::Image::from_raw(target.image);
        let extent = vk::Extent3D {
            width: target.width,
            height: target.height,
            depth: 1,
        };
        self.submit(|d, commands| {
            let region = vk::BufferImageCopy::default()
                .image_subresource(color_layers())
                .image_extent(extent);
            // SAFETY: `commands` is recording, and `image` and `buffer` are valid handles on this device.
            unsafe {
                barrier(
                    d,
                    commands,
                    image,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                );
                d.cmd_copy_image_to_buffer(
                    commands,
                    image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    buffer,
                    &[region],
                );
                barrier(
                    d,
                    commands,
                    image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                );
            }
        })?;
        // SAFETY: `mapped` points at `capacity >= len` bytes of coherent memory, and the copy has finished.
        Ok(unsafe { std::slice::from_raw_parts(mapped, len) }.to_vec())
    }
}

fn color_layers() -> vk::ImageSubresourceLayers {
    vk::ImageSubresourceLayers {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        mip_level: 0,
        base_array_layer: 0,
        layer_count: 1,
    }
}

/// Records a layout transition of the whole colour image between two transfer-compatible layouts.
///
/// # Safety
///
/// `commands` must be recording on `d`, and `image` must be a valid colour image of that device.
unsafe fn barrier(
    d: &ash::Device,
    commands: vk::CommandBuffer,
    image: vk::Image,
    old: vk::ImageLayout,
    new: vk::ImageLayout,
) {
    let access = |layout: vk::ImageLayout| match layout {
        vk::ImageLayout::TRANSFER_DST_OPTIMAL => vk::AccessFlags::TRANSFER_WRITE,
        vk::ImageLayout::TRANSFER_SRC_OPTIMAL => vk::AccessFlags::TRANSFER_READ,
        vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL => {
            vk::AccessFlags::COLOR_ATTACHMENT_READ | vk::AccessFlags::COLOR_ATTACHMENT_WRITE
        }
        _ => vk::AccessFlags::empty(),
    };
    let barrier = vk::ImageMemoryBarrier::default()
        .old_layout(old)
        .new_layout(new)
        .src_access_mask(access(old))
        .dst_access_mask(access(new))
        .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
        .image(image)
        .subresource_range(vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::COLOR,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        });
    // SAFETY: the caller guarantees the command buffer and the image.
    unsafe {
        d.cmd_pipeline_barrier(
            commands,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[barrier],
        );
    }
}

impl Presenter for VulkanPresenter {
    fn upload(&mut self, target: Target, pixels: &Pixels<'_>) -> Result<(), GpuError> {
        pixels.check()?;
        if (pixels.width, pixels.height) != (target.width, target.height) {
            return Err(GpuError::BadPixels(format!(
                "the picture is {}x{} and the image {}x{}",
                pixels.width, pixels.height, target.width, target.height
            )));
        }
        let len = packed_len(pixels.width, pixels.height);
        self.ensure_staging(len)?;
        let Some(staging) = self.staging.as_ref() else {
            return Err(GpuError::Unavailable("no staging buffer".to_owned()));
        };
        let (buffer, mapped) = (staging.buffer, staging.mapped);
        // SAFETY: `mapped` points at `capacity >= len` bytes of coherent host memory that no GPU work is using,
        // since the last submission was waited for, and nothing else aliases it.
        let destination = unsafe { std::slice::from_raw_parts_mut(mapped, len) };
        convert(pixels, target.format, destination)?;

        let image = vk::Image::from_raw(target.image);
        let extent = vk::Extent3D {
            width: target.width,
            height: target.height,
            depth: 1,
        };
        self.submit(|d, commands| {
            let region = vk::BufferImageCopy::default()
                .image_subresource(color_layers())
                .image_extent(extent);
            // SAFETY: `commands` is recording, and `image` and `buffer` are valid handles on this device. The whole
            // image is overwritten, so its old contents are discarded (`UNDEFINED`). It ends in the layout OpenXR
            // expects a swapchain image to be released in.
            unsafe {
                barrier(
                    d,
                    commands,
                    image,
                    vk::ImageLayout::UNDEFINED,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                );
                d.cmd_copy_buffer_to_image(
                    commands,
                    buffer,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
                barrier(
                    d,
                    commands,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
                );
            }
        })
    }
}

impl Drop for VulkanPresenter {
    fn drop(&mut self) {
        self.free_staging();
        let d = &self.device.device;
        // SAFETY: every submission waited for its fence, so nothing is pending. The command buffer is freed with
        // its pool.
        unsafe {
            d.destroy_fence(self.fence, None);
            d.destroy_command_pool(self.pool, None);
        }
    }
}
