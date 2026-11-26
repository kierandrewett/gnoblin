#![allow(unsafe_code)]
//! The Vulkan instance, the device the renderer draws on, the validation layer, and the one queue and timeline every frame
//! of every output on the device is submitted to.
//!
//! This file and its siblings `memory.rs`, `pipeline.rs`, `sync.rs`, `textures.rs` and `frame.rs` are the `unsafe` shim of
//! the Vulkan renderer: `ash` is a raw binding, so every call into the loader or a driver is
//! `unsafe`. The rules the blocks rely on are the same everywhere:
//!
//! - a handle is created from a live [`VkContext`] and destroyed by the `Drop` of the
//!   wrapper that owns it, while that wrapper still holds an `Rc` to the context, so the
//!   device outlives everything made on it;
//! - the renderer drives one queue from one thread. A frame signals the next value of the context's timeline semaphore, and
//!   nothing a frame used is touched or destroyed until the timeline has reached its value ([`VkContext::wait`], or the
//!   retire queue of `sync.rs`);
//! - a structure handed to the driver borrows only data that outlives the call.
//!
//! # Device choice
//!
//! Every physical device the loader lists is looked at. A device is acceptable when it speaks Vulkan 1.2 with timeline
//! semaphores, has a graphics queue and supports the two formats the renderer uses (an `R8G8B8A8_UNORM` target
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
//! [`Validation::Synchronized`] adds the layer's synchronisation checks, which is where races between frames in flight
//! show. A release build, or a machine without the layer, runs without it.

#[cfg(test)]
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_void};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use ash::ext::{debug_utils, validation_features};
use ash::{Device, Entry, Instance, vk};

use super::error::VulkanError;
use super::memory::{Counters, Memory};
use super::pipeline::PipelineCacheLocation;
use super::sync::Timeline;

use crate::log;

/// The validation layer the renderer asks for in debug builds.
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// The format of the target a frame is drawn into and read back from.
///
/// Its bytes are red, green, blue, alpha in memory, which is the order [`Image`](gnoblin_render_frame::image::Image) promises.
pub(super) const TARGET_FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// The format of a client's `argb8888` or `xrgb8888` buffer on a little-endian machine.
pub(super) const SHM_FORMAT: vk::Format = vk::Format::B8G8R8A8_UNORM;

/// How long a wait for the GPU may take before the device is treated as lost.
pub(super) const WAIT_TIMEOUT: Duration = Duration::from_secs(10);

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
    /// As [`Validation::Auto`], and with the layer's synchronisation validation, which checks every barrier of a frame
    /// against the frames in flight beside it. The tests ask for it. It costs a good deal of CPU time, so a session does
    /// not.
    Synchronized,
    /// Never.
    Off,
}

/// How a renderer is created.
///
/// The default is what the compositor uses. The other values exist for the things a
/// test has to provoke on purpose: a machine with no loader, a machine with no
/// acceptable device, and a cache file that is not there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VulkanOptions {
    /// Where the loader comes from.
    pub loader: Loader,
    /// Which device to use.
    pub device: DeviceChoice,
    /// Whether to enable the validation layer.
    pub validation: Validation,
    /// Where the driver's pipeline cache is kept between runs.
    pub pipeline_cache: PipelineCacheLocation,
}

/// What kind of device the renderer is drawing on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuKind {
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

impl GpuKind {
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
    pub kind: GpuKind,
}

/// Failures a test injects into the driver calls the renderer makes.
///
/// A real out-of-memory or a lost device cannot be provoked on a software driver, so the
/// calls where it matters consult these cells. They exist only in test builds.
#[cfg(test)]
#[derive(Default)]
pub(super) struct Faults {
    /// Fail the next texture allocation after this many succeed.
    pub texture_allocation: Cell<Option<(u32, vk::Result)>>,
    /// Fail the next queue submission without submitting.
    pub submit: Cell<Option<vk::Result>>,
    /// Fail the next creation of a shell's shader pipeline, after its source compiled.
    pub pipeline: Cell<Option<vk::Result>>,
    /// Make the next shader build wait this long on its worker before it starts, as a slow driver would.
    pub build_delay: Cell<Option<std::time::Duration>>,
    /// Make the next submission wait for a timeline semaphore the test holds to reach a value, as a slow GPU would: the
    /// frame is submitted and does not run until the test signals it.
    pub gate: Cell<Option<(vk::Semaphore, u64)>>,
}

/// The loader, instance and logical device, with the memory properties and limits the
/// renderer reads, the allocator, and the timeline every submission signals.
pub(super) struct VkContext {
    // Kept alive because the instance's function pointers live in the loaded library.
    _entry: Entry,
    instance: Instance,
    messenger: Option<Messenger>,
    pub(super) device: Device,
    pub(super) queue: vk::Queue,
    pub(super) family: u32,
    pub(super) limits: vk::PhysicalDeviceLimits,
    /// How many bits of a timestamp the queue writes: 0 when it writes none.
    pub(super) timestamp_bits: u32,
    pub(super) info: DeviceInfo,
    /// What identifies the device's pipeline cache: vendor, device and the driver's UUID.
    pub(super) cache_identity: CacheIdentity,
    /// What the validation layer reported, see [`Validation`].
    pub(super) messages: Arc<Mutex<Vec<String>>>,
    /// Whether the validation layer is on.
    pub(super) validating: bool,
    /// The allocator every buffer and image is made from, and the counters it keeps.
    pub(super) memory: Memory,
    /// Counts the frames submitted and says which have finished.
    pub(super) timeline: Timeline,
    #[cfg(test)]
    pub(super) faults: Faults,
}

/// What a pipeline cache blob is for: it is refused by any other device or driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CacheIdentity {
    pub vendor: u32,
    pub device: u32,
    pub uuid: [u8; vk::UUID_SIZE],
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
        // when the renderer was dropped. The allocator frees its blocks while the device is
        // there to take them, and the timeline goes before the device that made it. The messenger is destroyed before the
        // instance that owns it, and its sink is released after, once the layer cannot call it.
        unsafe {
            let _ = self.device.device_wait_idle();
            self.memory.shutdown();
            self.timeline.destroy(&self.device);
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
            Validation::Auto | Validation::Synchronized => cfg!(debug_assertions) && validation_layer_present(&entry),
        };
        let synchronized = wants_validation && options.validation == Validation::Synchronized;
        let (instance, messenger) = match create_instance(&entry, wants_validation, synchronized, &messages) {
            Ok(created) => created,
            Err(error @ VulkanError::Driver { .. }) if wants_validation => {
                // A layer that is listed but cannot load must not take the renderer down.
                log::warn!(area = "render", %error, "the Vulkan validation layer could not be enabled");
                create_instance(&entry, false, false, &messages)?
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

    /// The failure a test asked for on the next texture allocation, if it is due.
    #[cfg(test)]
    pub(super) fn texture_fault(&self) -> Option<vk::Result> {
        let (skip, result) = self.faults.texture_allocation.get()?;
        if skip == 0 {
            self.faults.texture_allocation.set(None);
            Some(result)
        } else {
            self.faults.texture_allocation.set(Some((skip - 1, result)));
            None
        }
    }

    /// Submits one command buffer and returns the timeline value it signals when it has finished.
    ///
    /// Nothing waits: the caller keeps what the buffer uses alive until [`VkContext::completed`] reaches the value, and
    /// waits for it ([`VkContext::wait`]) only when it must read what the buffer wrote.
    ///
    /// # Errors
    ///
    /// The driver's result: a lost device, or out of memory.
    pub(super) fn submit(&self, cmd: vk::CommandBuffer) -> Result<u64, vk::Result> {
        #[cfg(test)]
        if let Some(result) = self.faults.submit.take() {
            return Err(result);
        }
        let value = self.timeline.submitted() + 1;
        let buffers = [cmd];
        let signals = [self.timeline.semaphore()];
        let signal_values = [value];
        #[cfg(test)]
        let gate = self.faults.gate.take();
        #[cfg(not(test))]
        let gate: Option<(vk::Semaphore, u64)> = None;
        let waits: Vec<vk::Semaphore> = gate.iter().map(|(semaphore, _)| *semaphore).collect();
        let wait_values: Vec<u64> = gate.iter().map(|(_, value)| *value).collect();
        let stages = vec![vk::PipelineStageFlags::ALL_COMMANDS; waits.len()];
        let mut timeline = vk::TimelineSemaphoreSubmitInfo::default()
            .wait_semaphore_values(&wait_values)
            .signal_semaphore_values(&signal_values);
        let submit = vk::SubmitInfo::default()
            .command_buffers(&buffers)
            .wait_semaphores(&waits)
            .wait_dst_stage_mask(&stages)
            .signal_semaphores(&signals)
            .push_next(&mut timeline);
        // SAFETY: the queue belongs to this device and is used from this thread only; the buffer is in the executable
        // state and everything it uses stays alive until the value is reached.
        unsafe { self.device.queue_submit(self.queue, &[submit], vk::Fence::null()) }?;
        self.timeline.record(value);
        Counters::add(&self.memory.counters.submits, 1);
        Ok(value)
    }

    /// Waits until the timeline has reached `value`, which is a frame's: a value of 0 is none.
    ///
    /// # Errors
    ///
    /// `TIMEOUT` after [`WAIT_TIMEOUT`], which the caller treats as a lost device, and the driver's result otherwise.
    pub(super) fn wait(&self, value: u64) -> Result<(), vk::Result> {
        if value == 0 || self.completed() >= value {
            return Ok(());
        }
        let semaphores = [self.timeline.semaphore()];
        let values = [value];
        let info = vk::SemaphoreWaitInfo::default().semaphores(&semaphores).values(&values);
        let nanoseconds = u64::try_from(WAIT_TIMEOUT.as_nanos()).unwrap_or(u64::MAX);
        // SAFETY: the semaphore is this context's timeline, and `value` is one it was asked to signal.
        unsafe { self.device.wait_semaphores(&info, nanoseconds) }
    }

    /// The highest timeline value the GPU has reached: every frame up to it has finished. A device that cannot say is
    /// taken to have finished nothing.
    pub(super) fn completed(&self) -> u64 {
        // SAFETY: a plain query of this context's timeline.
        unsafe { self.device.get_semaphore_counter_value(self.timeline.semaphore()) }.unwrap_or(0)
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

/// Creates the instance, with the validation layer and a messenger when `validation`, and its synchronisation checks when
/// `synchronized`.
fn create_instance(
    entry: &Entry,
    validation: bool,
    synchronized: bool,
    messages: &Arc<Mutex<Vec<String>>>,
) -> Result<(Instance, Option<Messenger>), VulkanError> {
    let application = vk::ApplicationInfo::default()
        .application_name(c"gnoblin")
        .api_version(vk::API_VERSION_1_2);
    let layers: [*const c_char; 1] = [VALIDATION_LAYER.as_ptr()];
    let mut extensions: Vec<*const c_char> = vec![debug_utils::NAME.as_ptr()];
    if synchronized {
        extensions.push(validation_features::NAME.as_ptr());
    }
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
    let enabled_features = [vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION];
    let mut features = vk::ValidationFeaturesEXT::default().enabled_validation_features(&enabled_features);
    let mut info = vk::InstanceCreateInfo::default().application_info(&application);
    if validation {
        info = info
            .enabled_layer_names(&layers)
            .enabled_extension_names(&extensions)
            .push_next(&mut messenger_info);
        if synchronized {
            info = info.push_next(&mut features);
        }
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
    timestamp_bits: u32,
    info: DeviceInfo,
    cache_identity: CacheIdentity,
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
    log::debug!(area = "render", device = %candidate.info.name, kind = ?candidate.info.kind, "Vulkan device chosen");
    let priorities = [1.0_f32];
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(candidate.family)
        .queue_priorities(&priorities)];
    let mut features = vk::PhysicalDeviceVulkan12Features::default().timeline_semaphore(true);
    let info = vk::DeviceCreateInfo::default()
        .queue_create_infos(&queues)
        .push_next(&mut features);
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
    // SAFETY: a plain property query of a physical device of this instance.
    let properties = unsafe { instance.get_physical_device_properties(candidate.device) };
    let made = Memory::new(&instance, &device, candidate.device).and_then(|memory| {
        let timeline = Timeline::new(&device).map_err(|result| VulkanError::driver("vkCreateSemaphore", result));
        timeline.map(|timeline| (memory, timeline))
    });
    let (memory, timeline) = match made {
        Ok(made) => made,
        Err(error) => {
            // SAFETY: nothing was submitted to the device.
            unsafe { device.destroy_device(None) };
            return Err(Box::new((error, entry, instance, messenger)));
        }
    };
    Ok(VkContext {
        _entry: entry,
        instance,
        messenger,
        device,
        queue,
        family: candidate.family,
        limits: properties.limits,
        timestamp_bits: candidate.timestamp_bits,
        info: candidate.info,
        cache_identity: candidate.cache_identity,
        messages,
        validating,
        memory,
        timeline,
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
        if properties.api_version < vk::API_VERSION_1_2 {
            refusals.push(format!(
                "{name}: speaks Vulkan {}.{}, and the renderer needs 1.2 for timeline semaphores",
                vk::api_version_major(properties.api_version),
                vk::api_version_minor(properties.api_version)
            ));
            continue;
        }
        if !timeline_semaphores(instance, device) {
            refusals.push(format!("{name}: has no timeline semaphores"));
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
            timestamp_bits: if properties.limits.timestamp_compute_and_graphics == vk::TRUE {
                families[family].timestamp_valid_bits
            } else {
                0
            },
            info: DeviceInfo {
                name,
                kind: GpuKind::from_vulkan(properties.device_type),
            },
            cache_identity: CacheIdentity {
                vendor: properties.vendor_id,
                device: properties.device_id,
                uuid: properties.pipeline_cache_uuid,
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

/// Whether the device can wait on and signal a timeline semaphore.
fn timeline_semaphores(instance: &Instance, device: vk::PhysicalDevice) -> bool {
    let mut timeline = vk::PhysicalDeviceVulkan12Features::default();
    let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut timeline);
    // SAFETY: a plain query of a device this instance listed, which speaks Vulkan 1.2.
    unsafe { instance.get_physical_device_features2(device, &mut features) };
    timeline.timeline_semaphore == vk::TRUE
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
