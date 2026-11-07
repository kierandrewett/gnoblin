#![allow(unsafe_code)]
//! The Vulkan instance, the device the renderer draws on, the validation layer and the
//! memory the renderer's buffers and images live in.
//!
//! This file and its siblings `pipeline.rs` and `frame.rs` are the `unsafe` shim of the
//! Vulkan renderer: `ash` is a raw binding, so every call into the loader or a driver is
//! `unsafe`. The rules the blocks rely on are the same everywhere:
//!
//! - a handle is created from a live [`VkContext`] and destroyed by the `Drop` of the
//!   wrapper that owns it, while that wrapper still holds an `Rc` to the context, so the
//!   device outlives everything made on it;
//! - the renderer drives one queue from one thread and waits for a frame's fence before
//!   it touches anything the frame used, so no handle is in use when it is destroyed;
//! - a structure handed to the driver borrows only data that outlives the call.
//!
//! # Device choice
//!
//! Every physical device the loader lists is looked at. A device is acceptable when it has a
//! graphics queue and supports the two formats the renderer uses (an `R8G8B8A8_UNORM` target
//! it can blend into and copy from, and a `B8G8R8A8_UNORM` texture it can sample and copy
//! into). Among those, a discrete GPU is preferred over an integrated one, then a virtual
//! one, then a CPU implementation such as lavapipe, which is accepted rather than refused:
//! it is what a machine with no GPU, and the tests, have.
//!
//! # Validation
//!
//! `VK_LAYER_KHRONOS_validation` is enabled in debug builds when it is installed
//! ([`Validation::Auto`]), and a debug messenger collects what it reports so a test can
//! fail on it. Errors of any type and validation warnings are collected; the rest is noise.
//! A release build, or a machine without the layer, runs without it.

#[cfg(test)]
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_void};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use ash::ext::debug_utils;
use ash::{Device, Entry, Instance, vk};

use super::error::VulkanError;

use crate::log;

/// The validation layer the renderer asks for in debug builds.
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// The format of the target a frame is drawn into and read back from.
///
/// Its bytes are red, green, blue, alpha in memory, which is the order [`Image`](crate::render::Image) promises.
pub(super) const TARGET_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// The format of a client's `argb8888` or `xrgb8888` buffer on a little-endian machine.
pub(super) const SHM_FORMAT: vk::Format = vk::Format::B8G8R8A8_UNORM;

/// How the renderer finds the Vulkan loader.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Loader {
    /// The system's loader, `libvulkan.so.1`.
    #[default]
    System,
    /// A loader at this path. It is how a test provokes [`VulkanError::NoLoader`].
    Library(PathBuf),
}

/// Which physical device the renderer uses.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DeviceChoice {
    /// The best acceptable device, ranked as the module documentation says.
    #[default]
    Best,
    /// The first acceptable device whose name contains this text, case-insensitively.
    Named(String),
}

/// Whether the validation layer is enabled.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Validation {
    /// On in debug builds when the layer is installed, off otherwise.
    #[default]
    Auto,
    /// Never.
    Off,
}

/// How a renderer is created.
///
/// The default is what the compositor uses. The other values exist for the two things a
/// test has to provoke on purpose: a machine with no loader and a machine with no
/// acceptable device.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VulkanOptions {
    /// Where the loader comes from.
    pub loader: Loader,
    /// Which device to use.
    pub device: DeviceChoice,
    /// Whether to enable the validation layer.
    pub validation: Validation,
}

/// What kind of device the renderer is drawing on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    /// A discrete GPU.
    Discrete,
    /// An integrated GPU.
    Integrated,
    /// A virtual GPU.
    Virtual,
    /// A CPU implementation, such as lavapipe.
    Cpu,
    /// Anything else the driver reports.
    Other,
}

impl DeviceKind {
    fn from_vulkan(kind: vk::PhysicalDeviceType) -> Self {
        match kind {
            vk::PhysicalDeviceType::DISCRETE_GPU => Self::Discrete,
            vk::PhysicalDeviceType::INTEGRATED_GPU => Self::Integrated,
            vk::PhysicalDeviceType::VIRTUAL_GPU => Self::Virtual,
            vk::PhysicalDeviceType::CPU => Self::Cpu,
            _ => Self::Other,
        }
    }

    /// Higher is preferred.
    fn rank(self) -> u8 {
        match self {
            Self::Discrete => 4,
            Self::Integrated => 3,
            Self::Virtual => 2,
            Self::Cpu => 1,
            Self::Other => 0,
        }
    }
}

/// The device a renderer chose, for a log line and a startup report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    /// The driver's name for the device.
    pub name: String,
    /// What kind of device it is.
    pub kind: DeviceKind,
}

/// Failures a test injects into the driver calls the renderer makes.
///
/// A real out-of-memory or a lost device cannot be provoked on a software driver, so the
/// two calls where it matters consult these cells. They exist only in test builds.
#[cfg(test)]
#[derive(Default)]
pub(super) struct Faults {
    /// Fail the next texture allocation after this many succeed.
    pub texture_allocation: Cell<Option<(u32, vk::Result)>>,
    /// Fail the next queue submission without submitting.
    pub submit: Cell<Option<vk::Result>>,
}

/// The loader, instance and logical device, with the memory properties and limits the
/// renderer reads.
pub(super) struct VkContext {
    // Kept alive because the instance's function pointers live in the loaded library.
    _entry: Entry,
    instance: Instance,
    messenger: Option<Messenger>,
    pub(super) device: Device,
    pub(super) queue: vk::Queue,
    pub(super) family: u32,
    memory: vk::PhysicalDeviceMemoryProperties,
    pub(super) limits: vk::PhysicalDeviceLimits,
    pub(super) info: DeviceInfo,
    /// What the validation layer reported, see [`Validation`].
    pub(super) messages: Arc<Mutex<Vec<String>>>,
    /// Whether the validation layer is on.
    pub(super) validating: bool,
    #[cfg(test)]
    pub(super) faults: Faults,
}

/// The debug messenger and the sink it writes into.
struct Messenger {
    loader: debug_utils::Instance,
    handle: vk::DebugUtilsMessengerEXT,
    /// An `Arc::into_raw` of the sink, released when the messenger is destroyed.
    sink: *const Mutex<Vec<String>>,
}

impl Drop for VkContext {
    fn drop(&mut self) {
        // SAFETY: every resource made on the device holds an `Rc` to this context, so none
        // is alive here; waiting for the queue first covers work that was still in flight
        // when the renderer was dropped. The messenger is destroyed before the instance
        // that owns it, and its sink is released after, once the layer cannot call it.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
            if let Some(messenger) = self.messenger.take() {
                messenger.loader.destroy_debug_utils_messenger(messenger.handle, None);
                drop(Arc::from_raw(messenger.sink));
            }
            self.instance.destroy_instance(None);
        }
    }
}

/// The debug messenger callback: keeps what a test must fail on.
unsafe extern "system" fn collect(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    kinds: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    user: *mut c_void,
) -> vk::Bool32 {
    let wanted = severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR)
        || (severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING)
            && kinds.contains(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION));
    if !wanted || data.is_null() || user.is_null() {
        return vk::FALSE;
    }
    // SAFETY: the layer passes a valid callback structure for the duration of the call and
    // `user` is the `Arc` the context leaked for this messenger, alive until it is destroyed.
    let (text, sink) = unsafe {
        let message = (*data).p_message;
        let text = if message.is_null() {
            String::new()
        } else {
            CStr::from_ptr(message).to_string_lossy().into_owned()
        };
        (text, &*(user as *const Mutex<Vec<String>>))
    };
    sink.lock().unwrap_or_else(PoisonError::into_inner).push(text);
    vk::FALSE
}

impl VkContext {
    /// Loads the loader, creates an instance and a device and returns the context.
    ///
    /// # Errors
    ///
    /// [`VulkanError::NoLoader`], [`VulkanError::NoDevice`] or [`VulkanError::Driver`].
    pub(super) fn create(options: &VulkanOptions) -> Result<Self, VulkanError> {
        // SAFETY: loading a shared library runs its initialisers; the system loader and a
        // path the caller named are the only libraries opened.
        let loaded = unsafe {
            match &options.loader {
                Loader::System => Entry::load(),
                Loader::Library(path) => Entry::load_from(path),
            }
        };
        let entry = loaded.map_err(|error| VulkanError::NoLoader {
            reason: error.to_string(),
        })?;
        let messages = Arc::new(Mutex::new(Vec::new()));
        let wants_validation = match options.validation {
            Validation::Off => false,
            Validation::Auto => cfg!(debug_assertions) && validation_layer_present(&entry),
        };
        let (instance, messenger) = match create_instance(&entry, wants_validation, &messages) {
            Ok(created) => created,
            Err(error @ VulkanError::Driver { .. }) if wants_validation => {
                // A layer that is listed but cannot load must not take the renderer down.
                log::warn!(%error, "[render] the Vulkan validation layer could not be enabled");
                create_instance(&entry, false, &messages)?
            }
            Err(error) => return Err(error),
        };
        let validating = messenger.is_some();
        match build(entry, instance, messenger, messages, validating, &options.device) {
            Ok(context) => Ok(context),
            Err(failure) => {
                let (error, entry, instance, messenger) = *failure;
                // SAFETY: nothing was created on this instance besides the messenger, and the
                // loader library (`entry`) is dropped last, after the instance that uses it.
                unsafe {
                    if let Some(messenger) = messenger {
                        messenger.loader.destroy_debug_utils_messenger(messenger.handle, None);
                        drop(Arc::from_raw(messenger.sink));
                    }
                    instance.destroy_instance(None);
                }
                drop(entry);
                Err(error)
            }
        }
    }

    /// The index of a memory type allowed by `bits` with all of `required`, preferring one
    /// that also has `preferred`.
    fn memory_type(
        &self,
        bits: u32,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
    ) -> Option<u32> {
        let types = &self.memory.memory_types[..self.memory.memory_type_count as usize];
        let find = |wanted: vk::MemoryPropertyFlags| {
            types
                .iter()
                .enumerate()
                .find(|(index, kind)| bits & (1 << index) != 0 && kind.property_flags.contains(wanted))
                .map(|(index, _)| index as u32)
        };
        find(required | preferred).or_else(|| find(required))
    }

    /// Allocates device memory for `requirements`.
    fn allocate(
        &self,
        requirements: vk::MemoryRequirements,
        required: vk::MemoryPropertyFlags,
        preferred: vk::MemoryPropertyFlags,
    ) -> Result<vk::DeviceMemory, vk::Result> {
        let index = self
            .memory_type(requirements.memory_type_bits, required, preferred)
            .ok_or(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY)?;
        let info = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index);
        // SAFETY: `info` names a memory type of this device and a size the driver reported.
        unsafe { self.device.allocate_memory(&info, None) }
    }

    /// The failure a test asked for on the next texture allocation, if it is due.
    #[cfg(test)]
    fn texture_fault(&self) -> Option<vk::Result> {
        let (skip, result) = self.faults.texture_allocation.get()?;
        if skip == 0 {
            self.faults.texture_allocation.set(None);
            Some(result)
        } else {
            self.faults.texture_allocation.set(Some((skip - 1, result)));
            None
        }
    }

    /// Waits until the device has finished everything submitted to it.
    pub(super) fn wait_idle(&self) {
        // SAFETY: a plain wait; a lost device returns an error, which leaves nothing to wait for.
        let _ = unsafe { self.device.device_wait_idle() };
    }

    /// What the validation layer has reported since the last call.
    pub(super) fn take_messages(&self) -> Vec<String> {
        std::mem::take(&mut *self.messages.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// Whether the loader lists the validation layer.
fn validation_layer_present(entry: &Entry) -> bool {
    // SAFETY: a plain query of the loader's layer list.
    let layers = unsafe { entry.enumerate_instance_layer_properties() }.unwrap_or_default();
    layers.iter().any(|layer| {
        // SAFETY: the driver NUL-terminates `layer_name` within its 256 bytes.
        let name = unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) };
        name == VALIDATION_LAYER
    })
}

/// Creates the instance, with the validation layer and a messenger when `validation`.
fn create_instance(
    entry: &Entry,
    validation: bool,
    messages: &Arc<Mutex<Vec<String>>>,
) -> Result<(Instance, Option<Messenger>), VulkanError> {
    let application = vk::ApplicationInfo::default()
        .application_name(c"gnoblin")
        .api_version(vk::API_VERSION_1_1);
    let layers: [*const c_char; 1] = [VALIDATION_LAYER.as_ptr()];
    let extensions: [*const c_char; 1] = [debug_utils::NAME.as_ptr()];
    let sink = Arc::into_raw(Arc::clone(messages));
    let mut messenger_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
        .message_severity(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR | vk::DebugUtilsMessageSeverityFlagsEXT::WARNING)
        .message_type(
            vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
        )
        .pfn_user_callback(Some(collect))
        .user_data(sink as *mut c_void);
    let mut info = vk::InstanceCreateInfo::default().application_info(&application);
    if validation {
        info = info
            .enabled_layer_names(&layers)
            .enabled_extension_names(&extensions)
            .push_next(&mut messenger_info);
    }
    // SAFETY: `info` and everything it points to live until the call returns.
    let created = unsafe { entry.create_instance(&info, None) };
    let instance = match created {
        Ok(instance) => instance,
        Err(result) => {
            // SAFETY: `sink` is the pointer made above and nothing else holds it.
            drop(unsafe { Arc::from_raw(sink) });
            return Err(VulkanError::instance(result));
        }
    };
    if !validation {
        // SAFETY: as above, the sink was never handed to a messenger.
        drop(unsafe { Arc::from_raw(sink) });
        return Ok((instance, None));
    }
    let loader = debug_utils::Instance::new(entry, &instance);
    // SAFETY: the instance enabled the debug-utils extension just above.
    match unsafe { loader.create_debug_utils_messenger(&messenger_info, None) } {
        Ok(handle) => Ok((instance, Some(Messenger { loader, handle, sink }))),
        Err(result) => {
            // SAFETY: the instance has nothing else on it and the sink was not handed over.
            unsafe {
                drop(Arc::from_raw(sink));
                instance.destroy_instance(None);
            }
            Err(VulkanError::driver("vkCreateDebugUtilsMessengerEXT", result))
        }
    }
}

/// What a physical device offers, for the choice.
struct Candidate {
    device: vk::PhysicalDevice,
    family: u32,
    info: DeviceInfo,
}

/// The error, the loader, the instance and the messenger when the device cannot be created, so
/// the caller can tear them down in order: the loader library must outlive the instance.
type Failure = Box<(VulkanError, Entry, Instance, Option<Messenger>)>;

/// Chooses a physical device and creates the logical device and the context.
fn build(
    entry: Entry,
    instance: Instance,
    messenger: Option<Messenger>,
    messages: Arc<Mutex<Vec<String>>>,
    validating: bool,
    choice: &DeviceChoice,
) -> Result<VkContext, Failure> {
    let candidate = match choose(&instance, choice) {
        Ok(candidate) => candidate,
        Err(error) => return Err(Box::new((error, entry, instance, messenger))),
    };
    log::debug!(device = %candidate.info.name, kind = ?candidate.info.kind, "[render] Vulkan device chosen");
    let priorities = [1.0_f32];
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(candidate.family)
        .queue_priorities(&priorities)];
    let info = vk::DeviceCreateInfo::default().queue_create_infos(&queues);
    // SAFETY: the physical device came from this instance and `info` outlives the call.
    let device = match unsafe { instance.create_device(candidate.device, &info, None) } {
        Ok(device) => device,
        Err(result) => {
            let error = VulkanError::driver("vkCreateDevice", result);
            return Err(Box::new((error, entry, instance, messenger)));
        }
    };
    // SAFETY: the family was requested in `queues`, which asked for one queue.
    let queue = unsafe { device.get_device_queue(candidate.family, 0) };
    // SAFETY: plain property queries of a physical device of this instance.
    let (memory, properties) = unsafe {
        (
            instance.get_physical_device_memory_properties(candidate.device),
            instance.get_physical_device_properties(candidate.device),
        )
    };
    Ok(VkContext {
        _entry: entry,
        instance,
        messenger,
        device,
        queue,
        family: candidate.family,
        memory,
        limits: properties.limits,
        info: candidate.info,
        messages,
        validating,
        #[cfg(test)]
        faults: Faults::default(),
    })
}

/// Picks the device the renderer draws on.
fn choose(instance: &Instance, choice: &DeviceChoice) -> Result<Candidate, VulkanError> {
    // SAFETY: a plain query of the instance's physical devices.
    let devices = unsafe { instance.enumerate_physical_devices() }.map_err(|result| VulkanError::NoDevice {
        reason: format!("the loader could not list devices: {result:?}"),
    })?;
    if devices.is_empty() {
        return Err(VulkanError::NoDevice {
            reason: "the loader lists no device; no Vulkan driver is installed".to_owned(),
        });
    }
    let mut accepted: Vec<Candidate> = Vec::new();
    let mut refusals = Vec::new();
    for device in devices {
        // SAFETY: queries of a device this instance listed.
        let (properties, families) = unsafe {
            (
                instance.get_physical_device_properties(device),
                instance.get_physical_device_queue_family_properties(device),
            )
        };
        // SAFETY: the driver NUL-terminates the name.
        let name = unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        if let DeviceChoice::Named(wanted) = choice
            && !name.to_lowercase().contains(&wanted.to_lowercase())
        {
            refusals.push(format!("{name}: does not match `{wanted}`"));
            continue;
        }
        let Some(family) = families
            .iter()
            .position(|family| family.queue_flags.contains(vk::QueueFlags::GRAPHICS))
        else {
            refusals.push(format!("{name}: no graphics queue"));
            continue;
        };
        if let Err(missing) = formats_supported(instance, device) {
            refusals.push(format!("{name}: {missing}"));
            continue;
        }
        accepted.push(Candidate {
            device,
            family: family as u32,
            info: DeviceInfo {
                name,
                kind: DeviceKind::from_vulkan(properties.device_type),
            },
        });
    }
    accepted
        .into_iter()
        .max_by_key(|candidate| candidate.info.kind.rank())
        .ok_or_else(|| VulkanError::NoDevice {
            reason: refusals.join("; "),
        })
}

/// Whether the device can use the formats the renderer needs.
fn formats_supported(instance: &Instance, device: vk::PhysicalDevice) -> Result<(), String> {
    let needs = [
        (
            TARGET_FORMAT,
            vk::FormatFeatureFlags::COLOR_ATTACHMENT
                | vk::FormatFeatureFlags::COLOR_ATTACHMENT_BLEND
                | vk::FormatFeatureFlags::TRANSFER_SRC
                | vk::FormatFeatureFlags::TRANSFER_DST
                | vk::FormatFeatureFlags::SAMPLED_IMAGE
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR,
        ),
        (
            SHM_FORMAT,
            vk::FormatFeatureFlags::SAMPLED_IMAGE
                | vk::FormatFeatureFlags::SAMPLED_IMAGE_FILTER_LINEAR
                | vk::FormatFeatureFlags::TRANSFER_DST,
        ),
    ];
    for (format, features) in needs {
        // SAFETY: a plain query of a device this instance listed.
        let properties = unsafe { instance.get_physical_device_format_properties(device, format) };
        if !properties.optimal_tiling_features.contains(features) {
            return Err(format!("{format:?} lacks {features:?}"));
        }
    }
    Ok(())
}

/// A buffer in host-visible, coherent memory, mapped for as long as it lives.
pub(super) struct VkBuffer {
    context: Rc<VkContext>,
    raw: vk::Buffer,
    memory: vk::DeviceMemory,
    size: usize,
    mapped: *mut u8,
}

impl VkBuffer {
    /// A buffer of at least `size` bytes.
    ///
    /// `cached` prefers host-cached memory, which makes the CPU reading a readback faster.
    ///
    /// # Errors
    ///
    /// The driver's result, which is an out-of-memory one when the memory is not there.
    pub(super) fn new(
        context: &Rc<VkContext>,
        size: usize,
        usage: vk::BufferUsageFlags,
        cached: bool,
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
        let preferred = if cached {
            vk::MemoryPropertyFlags::HOST_CACHED
        } else {
            vk::MemoryPropertyFlags::empty()
        };
        let memory = match context.allocate(
            requirements,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            preferred,
        ) {
            Ok(memory) => memory,
            Err(result) => {
                // SAFETY: the buffer is unused.
                unsafe { device.destroy_buffer(raw, None) };
                return Err(result);
            }
        };
        // SAFETY: `memory` is host-visible, at least as large as the buffer, and is bound once.
        let mapped = unsafe {
            device
                .bind_buffer_memory(raw, memory, 0)
                .and_then(|()| device.map_memory(memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty()))
        };
        match mapped {
            Ok(pointer) => Ok(Self {
                context: Rc::clone(context),
                raw,
                memory,
                size,
                mapped: pointer.cast(),
            }),
            Err(result) => {
                // SAFETY: neither handle is in use.
                unsafe {
                    device.destroy_buffer(raw, None);
                    device.free_memory(memory, None);
                }
                Err(result)
            }
        }
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
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.mapped.add(offset), bytes.len()) };
        true
    }

    /// The bytes from `offset`, for the CPU to fill before the frame that reads them is recorded.
    ///
    /// [`None`] when they do not fit.
    pub(super) fn as_mut_slice(&mut self, offset: usize, length: usize) -> Option<&mut [u8]> {
        let end = offset.checked_add(length).filter(|end| *end <= self.size)?;
        debug_assert!(end <= self.size);
        // SAFETY: in range by the check above; the mapping lives as long as `self` and no
        // submitted work reads this buffer while the renderer holds `&mut self` between frames.
        Some(unsafe { std::slice::from_raw_parts_mut(self.mapped.add(offset), length) })
    }

    /// The bytes from `offset`, which the device has finished writing.
    ///
    /// The caller has waited for the fence of the work that wrote them.
    pub(super) fn read(&self, offset: usize, length: usize) -> Option<&[u8]> {
        let end = offset.checked_add(length).filter(|end| *end <= self.size)?;
        debug_assert!(end <= self.size);
        // SAFETY: in range by the check above, and the memory is coherent and idle: no
        // submitted work writes it while the renderer holds `&self` between frames.
        Some(unsafe { std::slice::from_raw_parts(self.mapped.add(offset), length) })
    }
}

impl Drop for VkBuffer {
    fn drop(&mut self) {
        // SAFETY: the owner waited for the frame that used the buffer; the context outlives it.
        unsafe {
            self.context.device.unmap_memory(self.memory);
            self.context.device.destroy_buffer(self.raw, None);
            self.context.device.free_memory(self.memory, None);
        }
    }
}

/// A 2D image with a view, in device-local memory.
pub(super) struct VkImage {
    context: Rc<VkContext>,
    raw: vk::Image,
    view: vk::ImageView,
    memory: vk::DeviceMemory,
    width: u32,
    height: u32,
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
        let memory = match context.allocate(
            requirements,
            vk::MemoryPropertyFlags::empty(),
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        ) {
            Ok(memory) => memory,
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
            .subresource_range(colour_range());
        // SAFETY: the memory is bound once, before the view is made over the image.
        let view = unsafe {
            device
                .bind_image_memory(raw, memory, 0)
                .and_then(|()| device.create_image_view(&view_info, None))
        };
        match view {
            Ok(view) => Ok(Self {
                context: Rc::clone(context),
                raw,
                view,
                memory,
                width,
                height,
            }),
            Err(result) => {
                // SAFETY: neither handle is in use.
                unsafe {
                    device.destroy_image(raw, None);
                    device.free_memory(memory, None);
                }
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
}

impl Drop for VkImage {
    fn drop(&mut self) {
        // SAFETY: the owner waited for the frame that used the image; the context outlives it.
        unsafe {
            self.context.device.destroy_image_view(self.view, None);
            self.context.device.destroy_image(self.raw, None);
            self.context.device.free_memory(self.memory, None);
        }
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
